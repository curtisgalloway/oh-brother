// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! The render.py port: every public function here maps one-to-one to
//! a function there, and the layout arithmetic (integer division,
//! ceils, Python's round-half-even) is mirrored deliberately — change
//! render.py first, then this file.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use image::{imageops, GrayImage, Luma};

use crate::font::{self, LoadedFont};
use crate::{code128, fontcache, hostfonts, Canvas, RenderError, Result, DPI};

const GRIDFINITY_U_MM: f64 = 42.0;

// Widest label this renderer will produce (~70 m of tape — far beyond
// any cartridge). Python has no such bound and dies with a catchable
// MemoryError on absurd widths; Rust's allocator ABORTS the process
// instead, so oversized requests must be rejected before allocating.
const MAX_RENDER_PX: i64 = 500_000;

const WHITE: Luma<u8> = Luma([255]);
const BLACK: Luma<u8> = Luma([0]);

/// Python's `\s` / str.strip() whitespace class: Unicode White_Space
/// plus the C0 information separators U+001C..U+001F, which Rust's
/// char::is_whitespace excludes.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn py_trim(s: &str) -> &str {
    s.trim_matches(py_space)
}

fn py_trim_start(s: &str) -> &str {
    s.trim_start_matches(py_space)
}

fn is_blank(s: &str) -> bool {
    py_trim(s).is_empty()
}

/// The text-rendering knobs render.py takes as keyword arguments.
#[derive(Clone)]
pub struct TextOptions {
    /// Font id, alias, host family name, or file path (`#N` for a ttc
    /// face); None means the manifest default font.
    pub font: Option<String>,
    /// Fixed font size in px; None (or 0, like Python's falsy check)
    /// auto-fits the tape.
    pub size: Option<u32>,
    pub margin_px: u32,
    pub line_gap_px: u32,
    /// Horizontal stretch for text segments only — code modules keep
    /// exact widths to stay scannable.
    pub hscale: f64,
}

impl Default for TextOptions {
    fn default() -> TextOptions {
        TextOptions {
            font: None,
            size: None,
            margin_px: 8,
            line_gap_px: 2,
            hscale: 1.0,
        }
    }
}

impl TextOptions {
    fn fixed_size(&self) -> Option<u32> {
        self.size.filter(|&s| s > 0)
    }
}

/// Resolve a font spec to a loadable font file path (render.resolve_font).
///
/// Accepts a manifest font id, an alias (the Brother device-font names
/// and legacy macOS shortcuts), a file path with optional `#N` ttc
/// face index, or the family name of a host-installed font. None
/// means the manifest default font.
pub fn resolve_font(spec: Option<&str>) -> Result<String> {
    let cache = fontcache::global();
    let Some(spec) = spec else {
        return cached_or_fallback(cache.default_font_id(), "default font");
    };
    if spec.contains('/')
        || spec.ends_with(".ttf")
        || spec.ends_with(".ttc")
        || spec.ends_with(".otf")
    {
        let (path, _) = font::path_and_index(spec)?;
        if !std::path::Path::new(path).exists() {
            return Err(RenderError(format!("font file not found: {spec}")));
        }
        return Ok(spec.to_owned());
    }
    let key = spec.to_lowercase();
    if cache.font_by_id(&key).is_some() {
        return cached_or_fallback(&key, spec);
    }
    if let Some(target) = cache.alias_target(&key) {
        let target = target.to_owned();
        return cached_or_fallback(&target, spec);
    }
    if let Some(host) = hostfonts::path_for_family(spec) {
        return Ok(host);
    }
    Err(RenderError(format!(
        "unknown font {spec:?}; known ids: {}. Aliases, installed \
         font family names, and file paths also work",
        cache.visible_ids().join(", ")
    )))
}

/// The cached path for font_id, fetching if needed — but a print must
/// never fail just because the network is down: fall back loudly
/// (render._cached_or_fallback).
fn cached_or_fallback(font_id: &str, requested: &str) -> Result<String> {
    let cache = fontcache::global();
    match cache.ensure(font_id) {
        Ok(path) => Ok(path.to_string_lossy().into_owned()),
        Err(e) => match cache.best_effort_font() {
            Some(fallback) => {
                eprintln!(
                    "label: {requested}: not downloadable right now ({e}); \
                     falling back to {fallback}"
                );
                Ok(fallback)
            }
            None => Err(RenderError(format!(
                "font {requested:?} is not cached, cannot be downloaded \
                 ({e}), and no system fallback font was found"
            ))),
        },
    }
}

/// Cached icon/symbol fonts for per-character fallback. Never fetches,
/// so an empty cache just means fewer fallback glyphs. The id ORDER is
/// mirrored in render._fallback_font_paths and index.html's
/// FALLBACK_FONT_IDS — keep all three in sync.
fn fallback_font_paths() -> Vec<String> {
    let cache = fontcache::global();
    ["icons", "noto-symbols-2", "noto-symbols-1", "noto-emoji"]
        .iter()
        .filter_map(|id| cache.path_for(id))
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// :name -> replacement text (or icon-font codepoint) from the manifest.
fn symbol_expansions() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut out = HashMap::new();
        for sym in fontcache::global().symbols() {
            let Some(name) = sym["name"].as_str() else {
                continue;
            };
            let replacement = if sym["kind"] == "text" {
                sym["text"].as_str().unwrap_or_default().to_owned()
            } else {
                let cp = sym["cp"].as_u64().unwrap_or(0) as u32;
                char::from_u32(cp).map(String::from).unwrap_or_default()
            };
            out.insert(name.to_owned(), replacement);
        }
        out
    })
}

/// Replace :name tokens with their catalog glyphs; unknown names and
/// ordinary colons (12:30) pass through untouched. Mirrors the regex
/// `:([a-z0-9][a-z0-9-]*)`.
pub fn expand_symbols(text: &str) -> String {
    let is_start = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let is_cont = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' && i + 1 < chars.len() && is_start(chars[i + 1]) {
            let mut j = i + 1;
            while j < chars.len() && is_cont(chars[j]) {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            match symbol_expansions().get(&name) {
                Some(replacement) => out.push_str(replacement),
                None => {
                    out.push(':');
                    out.push_str(&name);
                }
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Split a line into (font_path, text) runs by glyph coverage
/// (render._split_runs).
fn split_runs(line: &str, main_font: &str) -> Result<Vec<(String, String)>> {
    let fallbacks = fallback_font_paths();
    let mut runs: Vec<(String, String)> = Vec::new();
    for mut ch in line.chars() {
        let mut chosen = None;
        for path in std::iter::once(main_font).chain(fallbacks.iter().map(String::as_str)) {
            if font::load(path)?.covers(ch) {
                chosen = Some(path);
                break;
            }
        }
        let path = match chosen {
            Some(path) => path,
            None => {
                ch = '?';
                main_font
            }
        };
        match runs.last_mut() {
            Some((last, text)) if last == path => text.push(ch),
            _ => runs.push((path.to_owned(), ch.to_string())),
        }
    }
    Ok(runs)
}

/// Largest font size whose ascent+descent fits line_height
/// (render._fit_font_size).
fn fit_font_size(font: &LoadedFont, line_height: i64) -> u32 {
    let mut lo: u32 = 4;
    let mut hi: u32 = (4 * line_height).max(4) as u32;
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let (ascent, descent) = font.metrics(mid);
        if i64::from(ascent) + i64::from(descent) <= line_height {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

fn threshold(canvas: &GrayImage) -> GrayImage {
    let mut out = canvas.clone();
    for pixel in out.pixels_mut() {
        *pixel = if pixel.0[0] >= 128 { WHITE } else { BLACK };
    }
    out
}

fn white_image(width: u32, height: u32) -> GrayImage {
    GrayImage::from_pixel(width.max(1), height.max(1), WHITE)
}

/// Copy src over dst at (x0, y0) — replacement, not blending, like
/// Image.paste — clipping anything out of bounds.
fn paste(dst: &mut GrayImage, src: &GrayImage, x0: i64, y0: i64) {
    for y in 0..src.height() {
        let dy = y0 + i64::from(y);
        if dy < 0 || dy >= i64::from(dst.height()) {
            continue;
        }
        for x in 0..src.width() {
            let dx = x0 + i64::from(x);
            if dx < 0 || dx >= i64::from(dst.width()) {
                continue;
            }
            dst.put_pixel(dx as u32, dy as u32, *src.get_pixel(x, y));
        }
    }
}

fn fill_rect(img: &mut GrayImage, x0: i64, y0: i64, x1: i64, y1: i64, color: Luma<u8>) {
    for y in y0.max(0)..=y1.min(i64::from(img.height()) - 1) {
        for x in x0.max(0)..=x1.min(i64::from(img.width()) - 1) {
            img.put_pixel(x as u32, y as u32, color);
        }
    }
}

/// render.render_text: auto-sized, centered, multi-line text with
/// per-character fallback, thresholded to 1-bit.
pub fn render_text(text: &str, height_px: u32, opts: &TextOptions) -> Result<GrayImage> {
    // Strip emoji variation selectors and ZWJ: they have no glyphs and
    // would otherwise render as "?" via the missing-glyph path.
    let text: String = text
        .chars()
        .filter(|c| !matches!(c, '\u{fe0e}' | '\u{fe0f}' | '\u{200d}'))
        .collect();
    let lines: Vec<&str> = text.split('\n').collect();
    let font_path = resolve_font(opts.font.as_deref())?;
    let n = lines.len() as i64;
    let line_gap = i64::from(opts.line_gap_px);
    let line_height = (i64::from(height_px) - line_gap * (n - 1)).div_euclid(n);
    if line_height < 4 {
        return Err(RenderError(format!(
            "{n} lines do not fit in {height_px} px of tape"
        )));
    }
    let main = font::load(&font_path)?;
    let size = match opts.fixed_size() {
        Some(size) => size,
        None => fit_font_size(&main, line_height),
    };

    let line_runs: Vec<Vec<(String, String)>> = lines
        .iter()
        .map(|line| split_runs(line, &font_path))
        .collect::<Result<_>>()?;
    let mut fonts: HashMap<String, Arc<LoadedFont>> = HashMap::new();
    let mut font_for = |path: &str| -> Result<Arc<LoadedFont>> {
        if let Some(f) = fonts.get(path) {
            return Ok(f.clone());
        }
        let f = font::load(path)?;
        fonts.insert(path.to_owned(), f.clone());
        Ok(f)
    };
    let mut line_widths: Vec<i64> = Vec::new();
    for runs in &line_runs {
        let mut width = 0i64;
        for (path, run_text) in runs {
            width += font_for(path)?.text_width(run_text, size).ceil() as i64;
        }
        line_widths.push(width);
    }
    let width = line_widths.iter().copied().max().unwrap_or(0).max(1);

    let canvas_w = width + 2 * i64::from(opts.margin_px);
    if canvas_w > MAX_RENDER_PX {
        return Err(RenderError(format!(
            "label is {canvas_w} px wide — too wide to render"
        )));
    }
    let mut canvas = Canvas::new(canvas_w as u32, height_px);
    let (ascent, descent) = main.metrics(size);
    let total = n * line_height + (n - 1) * line_gap;
    let y0 = (i64::from(height_px) - total).div_euclid(2);
    for (i, runs) in line_runs.iter().enumerate() {
        let cy = y0 as f64 + i as f64 * (line_height + line_gap) as f64 + line_height as f64 / 2.0;
        let baseline = cy - f64::from(ascent + descent) / 2.0 + f64::from(ascent);
        let mut x = f64::from(opts.margin_px) + (width - line_widths[i]) as f64 / 2.0;
        for (path, run_text) in runs {
            let f = font_for(path)?;
            f.draw(&mut canvas, x, baseline, run_text, size);
            x += f.text_width(run_text, size);
        }
    }

    let mut img = GrayImage::from_vec(canvas.width, canvas.height, canvas.buf)
        .expect("canvas buffer matches dimensions");
    if opts.hscale != 1.0 {
        let new_w = ((f64::from(img.width()) * opts.hscale).round_ties_even() as i64).max(1) as u32;
        img = imageops::resize(&img, new_w, height_px, imageops::FilterType::Lanczos3);
    }
    Ok(threshold(&img))
}

/// render.render_qr: modules sized to the tape with a 2-module quiet
/// zone, vertically centered.
pub fn render_qr(data: &str, height_px: u32) -> Result<GrayImage> {
    let code = qrcode::QrCode::new(data.as_bytes())
        .map_err(|e| RenderError(format!("cannot encode QR code: {e}")))?;
    let border: u32 = 2;
    let count = code.width() as u32;
    let modules = count + 2 * border;
    let box_px = (height_px / modules).max(1);
    if box_px == 1 && modules > height_px {
        return Err(RenderError(format!(
            "QR code needs {modules} px but tape is {height_px} px"
        )));
    }
    let side = modules * box_px;
    let mut canvas = white_image(side, height_px);
    let y0 = i64::from((height_px - side) / 2);
    let colors = code.to_colors();
    for my in 0..count {
        for mx in 0..count {
            if colors[(my * count + mx) as usize] == qrcode::Color::Dark {
                let x = i64::from((border + mx) * box_px);
                let y = y0 + i64::from((border + my) * box_px);
                fill_rect(
                    &mut canvas,
                    x,
                    y,
                    x + i64::from(box_px) - 1,
                    y + i64::from(box_px) - 1,
                    BLACK,
                );
            }
        }
    }
    Ok(canvas)
}

/// render.render_code128: bars drawn as exact module rectangles —
/// never resampled. At 180 dpi a 3 px module is ~0.42 mm, comfortably
/// phone-scannable.
pub fn render_code128(data: &str, height_px: u32, font_spec: Option<&str>) -> Result<GrayImage> {
    let module_px: u32 = 3;
    let pattern = code128::pattern(data)?;
    let quiet = 10 * module_px; // mandatory quiet zone, 10 modules per side
    let caption_h = if height_px >= 45 {
        (height_px / 3).clamp(14, 26)
    } else {
        0
    };
    let bar_h = height_px - caption_h;
    let mut img = white_image(pattern.len() as u32 * module_px + 2 * quiet, height_px);
    for (i, ch) in pattern.chars().enumerate() {
        if ch == '1' {
            let x = i64::from(quiet) + i as i64 * i64::from(module_px);
            fill_rect(
                &mut img,
                x,
                0,
                x + i64::from(module_px) - 1,
                i64::from(bar_h) - 1,
                BLACK,
            );
        }
    }
    if caption_h > 0 {
        let cap = render_text(
            data,
            caption_h,
            &TextOptions {
                font: font_spec.map(str::to_owned),
                margin_px: 0,
                ..TextOptions::default()
            },
        )?;
        if cap.width() > img.width() {
            let mut wider = white_image(cap.width(), height_px);
            let x = i64::from((cap.width() - img.width()) / 2);
            paste(&mut wider, &img, x, 0);
            img = wider;
        }
        let x = i64::from((img.width() - cap.width()) / 2);
        paste(&mut img, &cap, x, i64::from(bar_h));
    }
    Ok(img)
}

fn measure_line(line: &str, font_path: &str, size: u32) -> Result<i64> {
    let mut total = 0f64;
    for (path, run_text) in split_runs(line, font_path)? {
        total += font::load(&path)?.text_width(&run_text, size);
    }
    Ok(total.ceil() as i64)
}

fn fit_width(line: &str, font_path: &str, max_px: i64, hi: u32) -> Result<u32> {
    let mut lo: u32 = 4;
    let mut hi = hi;
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure_line(line, font_path, mid)? <= max_px {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    Ok(lo)
}

/// render.render_grid: one fixed-width strip divided into equal cells,
/// text centered in each. Cells share a uniform font size fitted to
/// the narrowest text.
const GRID_DIVIDER_PX: i64 = 1;
const GRID_PAD_PX: i64 = 4;

/// The shared strip geometry: (total_px, cell_w, avail), with
/// render.py's per-cell width check. Also used by try_grid BEFORE the
/// per-cell allocation, so an absurd cell count is rejected instead
/// of aborting the process on a failed allocation.
fn grid_geometry(width_mm: f64, n_cells: usize) -> Result<(i64, f64)> {
    // `as` on the f64 saturates, so absurd widths land in the guard.
    let total_px = (width_mm / 25.4 * f64::from(DPI)).round_ties_even() as i64;
    if total_px > MAX_RENDER_PX {
        return Err(RenderError(format!(
            "grid is {total_px} px wide — too wide to render"
        )));
    }
    let cell_w = total_px as f64 / n_cells as f64;
    let avail = cell_w as i64 - 2 * GRID_PAD_PX - GRID_DIVIDER_PX;
    if avail < 8 {
        return Err(RenderError(format!(
            "{n_cells} cells across {width_mm} mm leaves only {avail} px per cell"
        )));
    }
    Ok((total_px, cell_w))
}

pub fn render_grid(
    width_mm: f64,
    n_cells: usize,
    cells: &[String],
    height_px: u32,
    font_spec: Option<&str>,
    font_size: Option<u32>,
) -> Result<GrayImage> {
    let divider_px: i64 = GRID_DIVIDER_PX;
    let font_path = resolve_font(font_spec)?;
    let (total_px, cell_w) = grid_geometry(width_mm, n_cells)?;
    let avail = cell_w as i64 - 2 * GRID_PAD_PX - divider_px;
    let size = match font_size.filter(|&s| s > 0) {
        Some(size) => size,
        None => {
            let main = font::load(&font_path)?;
            let mut size = fit_font_size(&main, i64::from(height_px));
            for cell in cells {
                if !is_blank(cell) {
                    size = size.min(fit_width(cell, &font_path, avail, size)?);
                }
            }
            size
        }
    };
    let mut img = white_image(total_px.max(1) as u32, height_px);
    for (i, cell) in cells.iter().enumerate() {
        if is_blank(cell) {
            continue;
        }
        let cimg = render_text(
            cell,
            height_px,
            &TextOptions {
                font: font_spec.map(str::to_owned),
                size: Some(size),
                margin_px: 0,
                ..TextOptions::default()
            },
        )?;
        let x =
            (i as f64 * cell_w + (cell_w - f64::from(cimg.width())) / 2.0).round_ties_even() as i64;
        paste(&mut img, &cimg, x, 0);
    }
    for i in 1..n_cells {
        let x = (i as f64 * cell_w).round_ties_even() as i64;
        fill_rect(
            &mut img,
            x,
            0,
            x + divider_px - 1,
            i64::from(height_px) - 1,
            BLACK,
        );
    }
    Ok(img)
}

/// Case-insensitive ASCII prefix strip.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let n = prefix.len();
    if s.len() >= n && s.is_char_boundary(n) && s[..n].eq_ignore_ascii_case(prefix) {
        Some(&s[n..])
    } else {
        None
    }
}

enum Directive {
    Qr(String),
    Code(String),
}

/// The `qr:`/`code:` line syntax — mirrors `^(qr|code):\s*(.*\S)\s*$`,
/// case-insensitive: no leading whitespace, non-empty payload.
fn parse_directive(line: &str) -> Option<Directive> {
    for (prefix, is_qr) in [("qr:", true), ("code:", false)] {
        if let Some(rest) = strip_prefix_ci(line, prefix) {
            let payload = py_trim(rest);
            if !payload.is_empty() {
                return Some(if is_qr {
                    Directive::Qr(payload.to_owned())
                } else {
                    Directive::Code(payload.to_owned())
                });
            }
        }
    }
    None
}

/// The `grid:` directive on an already-stripped line — mirrors
/// `^grid:\s*([0-9]+(?:\.[0-9]+)?)\s*(u|mm)\s*/\s*([0-9]+)\s*$`,
/// case-insensitive. Returns (width_mm, n_cells).
fn parse_grid(line: &str) -> Option<(f64, usize)> {
    let rest = py_trim_start(strip_prefix_ci(line, "grid:")?);
    let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
    let int_len = digits(rest);
    if int_len == 0 {
        return None;
    }
    let mut num_len = int_len;
    let tail = &rest[int_len..];
    if let Some(frac) = tail.strip_prefix('.') {
        let frac_len = digits(frac);
        if frac_len == 0 {
            return None;
        }
        num_len = int_len + 1 + frac_len;
    }
    let width: f64 = rest[..num_len].parse().ok()?;
    let rest = py_trim_start(&rest[num_len..]);
    let (width_mm, rest) = if let Some(rest) = strip_prefix_ci(rest, "mm") {
        (width, rest)
    } else {
        let rest = strip_prefix_ci(rest, "u")?;
        (width * GRIDFINITY_U_MM, rest)
    };
    let rest = py_trim_start(py_trim_start(rest).strip_prefix('/')?);
    let cell_len = digits(rest);
    if cell_len == 0 || !is_blank(&rest[cell_len..]) {
        return None;
    }
    let n_cells: usize = rest[..cell_len].parse().ok()?;
    Some((width_mm, n_cells.max(1)))
}

/// render._try_grid: a leading grid: line turns the label into a grid.
fn try_grid(lines: &[&str], height_px: u32, opts: &TextOptions) -> Result<Option<GrayImage>> {
    let Some(first) = lines.iter().position(|l| !is_blank(l)) else {
        return Ok(None);
    };
    let Some((width_mm, n_cells)) = parse_grid(py_trim(lines[first])) else {
        return Ok(None);
    };
    let cell_lines = &lines[first + 1..];
    if cell_lines.iter().skip(n_cells).any(|l| !is_blank(l)) {
        return Err(RenderError(format!(
            "more cell lines than the {n_cells} declared cells"
        )));
    }
    // Python allocates [""] * n_cells here and dies with a catchable
    // MemoryError on absurd counts; a failed Vec allocation would
    // abort this process instead. Any count past MAX_RENDER_PX can
    // never pass the geometry checks, so reject it before allocating
    // (smaller counts keep render_grid's error precedence).
    if n_cells > MAX_RENDER_PX as usize {
        grid_geometry(width_mm, n_cells)?;
    }
    let cells: Vec<String> = (0..n_cells)
        .map(|i| expand_symbols(cell_lines.get(i).copied().unwrap_or("")))
        .collect();
    render_grid(
        width_mm,
        n_cells,
        &cells,
        height_px,
        opts.font.as_deref(),
        opts.size,
    )
    .map(Some)
}

/// render.render_label: compose a label from editor text.
///
/// Plain lines become stacked text; lines of the form `qr:DATA` or
/// `code:DATA` become QR / Code 128 segments, laid out left to right
/// in the order they appear. hscale applies to text segments only.
pub fn render_label(text: &str, height_px: u32, opts: &TextOptions) -> Result<GrayImage> {
    let all_lines: Vec<&str> = text.split('\n').collect();
    if let Some(grid) = try_grid(&all_lines, height_px, opts)? {
        return Ok(grid);
    }

    let mut segments: Vec<GrayImage> = Vec::new();
    let mut text_lines: Vec<String> = Vec::new();

    fn flush_text(
        segments: &mut Vec<GrayImage>,
        text_lines: &mut Vec<String>,
        height_px: u32,
        opts: &TextOptions,
    ) -> Result<()> {
        while text_lines.first().is_some_and(|l| is_blank(l)) {
            text_lines.remove(0);
        }
        while text_lines.last().is_some_and(|l| is_blank(l)) {
            text_lines.pop();
        }
        if !text_lines.is_empty() {
            segments.push(render_text(
                &text_lines.join("\n"),
                height_px,
                &TextOptions {
                    margin_px: 0,
                    ..opts.clone()
                },
            )?);
            text_lines.clear();
        }
        Ok(())
    }

    for line in &all_lines {
        match parse_directive(line) {
            Some(directive) => {
                flush_text(&mut segments, &mut text_lines, height_px, opts)?;
                segments.push(match directive {
                    Directive::Qr(data) => render_qr(&data, height_px)?,
                    Directive::Code(data) => {
                        render_code128(&data, height_px, opts.font.as_deref())?
                    }
                });
            }
            // :symbol expansion applies to text only — never to
            // qr:/code: payloads, where the caption must match the
            // encoded data.
            None => text_lines.push(expand_symbols(line)),
        }
    }
    flush_text(&mut segments, &mut text_lines, height_px, opts)?;
    if segments.is_empty() {
        return Err(RenderError("empty label".into()));
    }
    let out = if segments.len() == 1 {
        segments.pop().unwrap()
    } else {
        hstack(&segments, height_px, 10)
    };
    if opts.margin_px > 0 {
        let padded_w = i64::from(out.width()) + 2 * i64::from(opts.margin_px);
        if padded_w > MAX_RENDER_PX {
            return Err(RenderError(format!(
                "label is {padded_w} px wide — too wide to render"
            )));
        }
        let mut padded = white_image(padded_w as u32, height_px);
        paste(&mut padded, &out, i64::from(opts.margin_px), 0);
        return Ok(padded);
    }
    Ok(out)
}

/// render.load_image: grayscale, downscale to the tape height when
/// taller (never upscale), threshold at 128.
pub fn load_image(path: &str, height_px: u32) -> Result<GrayImage> {
    let img = image::open(path).map_err(|e| RenderError(format!("{path}: {e}")))?;
    let mut gray = img.to_luma8();
    if gray.height() > height_px {
        let w = (f64::from(gray.width()) * f64::from(height_px) / f64::from(gray.height()))
            .round_ties_even() as i64;
        gray = imageops::resize(
            &gray,
            w.max(1) as u32,
            height_px,
            imageops::FilterType::Lanczos3,
        );
    }
    Ok(threshold(&gray))
}

/// render.hstack: parts side by side, vertically centered.
pub fn hstack(parts: &[GrayImage], height_px: u32, gap_px: u32) -> GrayImage {
    let width: u32 =
        parts.iter().map(GrayImage::width).sum::<u32>() + gap_px * (parts.len() as u32 - 1);
    let mut canvas = white_image(width, height_px);
    let mut x: i64 = 0;
    for part in parts {
        let y = (i64::from(height_px) - i64::from(part.height())).div_euclid(2);
        paste(&mut canvas, part, x, y);
        x += i64::from(part.width()) + i64::from(gap_px);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directive_payload_is_not_symbol_expanded() {
        // :symbol expansion must never touch qr:/code: payloads.
        match parse_directive("qr::warning") {
            Some(Directive::Qr(data)) => assert_eq!(data, ":warning"),
            _ => panic!("expected a qr directive"),
        }
        match parse_directive("CODE: 12V DC ") {
            Some(Directive::Code(data)) => assert_eq!(data, "12V DC"),
            _ => panic!("expected a code directive"),
        }
        assert!(parse_directive("qr:").is_none());
        assert!(parse_directive(" qr:data").is_none()); // anchored at ^
        assert!(parse_directive("barcode:x").is_none());
    }

    #[test]
    fn grid_directive_forms() {
        assert_eq!(parse_grid("grid:1u/2"), Some((42.0, 2)));
        assert_eq!(parse_grid("grid: 3.5 mm / 4"), Some((3.5, 4)));
        assert_eq!(parse_grid("GRID:2U/3"), Some((84.0, 3)));
        assert_eq!(parse_grid("grid:10mm/0"), Some((10.0, 1))); // max(1, n)
        assert_eq!(parse_grid("grid:1u"), None);
        assert_eq!(parse_grid("grid:1x/2"), None);
        assert_eq!(parse_grid("grid:1./2"), None); // regex needs digits after '.'
        assert_eq!(parse_grid("grid:1u/2 extra"), None);
    }

    /// Python's \s includes the C0 information separators; directive
    /// payload trimming and blank-line checks must agree with it.
    #[test]
    fn c0_separators_count_as_whitespace() {
        match parse_directive("qr:\u{1c}DATA\u{1c}") {
            Some(Directive::Qr(data)) => assert_eq!(data, "DATA"),
            _ => panic!("expected a qr directive"),
        }
        assert!(is_blank("\u{1c}\u{1d} \u{1f}"));
        assert_eq!(parse_grid("grid:\u{1c}1u\u{1c}/\u{1c}2"), Some((42.0, 2)));
    }

    /// A grid with an absurd cell count must come back as a normal
    /// error (Python: MemoryError) — never an allocation abort.
    #[test]
    fn grid_with_absurd_cell_count_errors_cleanly() {
        let Some(system) = fontcache::system_fallback() else {
            return;
        };
        let opts = TextOptions {
            font: Some(system.to_owned()),
            ..TextOptions::default()
        };
        let err = render_label("grid:1mm/100000000000000\nA", 76, &opts).unwrap_err();
        assert!(err.0.contains("cells across"), "{err}");
        // And width alone past the render bound is rejected too.
        let err = render_label("grid:99999999u/1\nA", 76, &opts).unwrap_err();
        assert!(err.0.contains("too wide"), "{err}");
    }

    #[test]
    fn split_runs_uncovered_char_becomes_question() {
        let Some(system) = fontcache::system_fallback() else {
            return; // no known system font on this machine
        };
        // U+0378 is unassigned: no font anywhere maps it.
        let runs = split_runs("A\u{0378}", system).unwrap();
        let joined: String = runs.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(joined, "A?");
    }

    #[test]
    fn fit_font_size_is_maximal() {
        let Some(system) = fontcache::system_fallback() else {
            return;
        };
        let main = font::load(system).unwrap();
        let size = fit_font_size(&main, 76);
        let (ascent, descent) = main.metrics(size);
        assert!(ascent + descent <= 76);
        let (ascent, descent) = main.metrics(size + 1);
        assert!(ascent + descent > 76);
    }
}
