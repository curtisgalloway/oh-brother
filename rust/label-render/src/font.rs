// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Font loading, metrics, measuring, and glyph rasterization.
//!
//! The Python renderer drives FreeType through Pillow; this drives
//! ab_glyph. Sizing convention matches FreeType: a size of N px means
//! N pixels per em. Ascent/descent mirror Pillow's `getmetrics()`
//! (hhea values honoring OS/2 USE_TYPO_METRICS, ceiled to whole
//! pixels), which the auto-fit search depends on. No shaping and no
//! kerning — glyphs advance by their metrics, which keeps layout close
//! to (not identical with) Pillow's; pixel-exact parity is a non-goal.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use ab_glyph::{point, Font as _, FontVec, PxScale, ScaleFont as _};

use crate::{Canvas, RenderError};

/// Split an optional `#N` ttc face index off a font path
/// (render._path_and_index). Like Python's `int(idx)`, a `#` with a
/// non-numeric tail is an error, not a silently ignored suffix.
pub fn path_and_index(spec: &str) -> Result<(&str, u32), RenderError> {
    match spec.rsplit_once('#') {
        Some((path, idx)) => {
            let idx = idx.parse().map_err(|_| {
                RenderError(format!(
                    "invalid ttc face index {idx:?} in font spec {spec:?}"
                ))
            })?;
            Ok((path, idx))
        }
        None => Ok((spec, 0)),
    }
}

pub struct LoadedFont {
    font: FontVec,
    codepoints: HashSet<u32>,
    // Font-unit metrics captured at parse time (ttf-parser applies
    // OS/2 USE_TYPO_METRICS the same way FreeType does).
    ascent_unscaled: i64,
    descent_unscaled: i64, // negative
    units_per_em: i64,
}

/// Load (and process-wide cache) the font behind a resolved spec.
pub fn load(spec: &str) -> Result<Arc<LoadedFont>, RenderError> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<LoadedFont>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(font) = cache.lock().unwrap().get(spec) {
        return Ok(font.clone());
    }
    let font = Arc::new(LoadedFont::parse(spec)?);
    cache.lock().unwrap().insert(spec.to_owned(), font.clone());
    Ok(font)
}

impl LoadedFont {
    fn parse(spec: &str) -> Result<LoadedFont, RenderError> {
        let (path, index) = path_and_index(spec)?;
        let data = std::fs::read(path)
            .map_err(|e| RenderError(format!("cannot read font {path}: {e}")))?;
        let face = ttf_parser::Face::parse(&data, index)
            .map_err(|e| RenderError(format!("cannot parse font {spec}: {e}")))?;
        let mut codepoints = HashSet::new();
        if let Some(cmap) = face.tables().cmap {
            for subtable in cmap.subtables {
                if subtable.is_unicode() {
                    subtable.codepoints(|cp| {
                        codepoints.insert(cp);
                    });
                }
            }
        }
        let ascent_unscaled = i64::from(face.ascender());
        let descent_unscaled = i64::from(face.descender());
        let units_per_em = i64::from(face.units_per_em());
        let font = FontVec::try_from_vec_and_index(data, index)
            .map_err(|e| RenderError(format!("cannot load font {spec}: {e}")))?;
        Ok(LoadedFont {
            font,
            codepoints,
            ascent_unscaled,
            descent_unscaled,
            units_per_em,
        })
    }

    /// Whether the cmap covers this character (render.font_covers).
    pub fn covers(&self, ch: char) -> bool {
        self.codepoints.contains(&u32::from(ch))
    }

    /// (ascent, descent) in px like Pillow's getmetrics(), which
    /// reports FreeType's size metrics. FreeType works on the 26.6
    /// fixed-point grid: y_scale is a truncated 16.16 ratio (FT_DivFix
    /// of the 26.6 size by units-per-em), the font-unit value goes
    /// through FT_MulFix (round-half-up at the 16.16 boundary), and
    /// only THEN is the ascender ceiled / the descender floored to a
    /// whole pixel. Skipping the fixed-point quantization and ceiling
    /// the exact real value overshoots by a pixel whenever the product
    /// lands just above an integer (e.g. Tahoma's 2049/2048 ascender),
    /// which made the auto-fit search pick a size smaller than the
    /// Python oracle's. Descent is returned positive.
    pub fn metrics(&self, size: u32) -> (i32, i32) {
        // FT_DivFix(size << 6, upem): 16.16, truncated toward zero.
        let y_scale = (i64::from(size) * 64 * 65536) / self.units_per_em;
        // FT_MulFix: (a*b + 0x8000 - (a*b < 0)) >> 16, arithmetic
        // shift — negative products round their half-boundary the
        // other way (roboto-slab's descender at size 48 lands exactly
        // on one).
        let mul_fix = |units: i64| {
            let product = units * y_scale;
            (product + 0x8000 - i64::from(product < 0)) >> 16
        };
        let ascender = mul_fix(self.ascent_unscaled); // 26.6
        let descender = mul_fix(self.descent_unscaled); // 26.6, negative
        let ascent = (ascender + 63) >> 6; // FT_PIX_CEIL, then to px
        let descent = -(descender >> 6); // FT_PIX_FLOOR, negated
        (ascent as i32, descent as i32)
    }

    /// ab_glyph's PxScale for a FreeType-style px-per-em size.
    fn px_scale(&self, size: u32) -> PxScale {
        let factor =
            self.font.height_unscaled() / self.font.units_per_em().expect("scalable font has upem");
        PxScale::from(size as f32 * factor)
    }

    /// Advance width of `text` in px — the analog of Pillow's
    /// `textlength` (sum of advances; no kerning, see module docs).
    pub fn text_width(&self, text: &str, size: u32) -> f64 {
        let scaled = self.font.as_scaled(self.px_scale(size));
        text.chars()
            .map(|ch| f64::from(scaled.h_advance(self.font.glyph_id(ch))))
            .sum()
    }

    /// Draw `text` dark-on-light with `x` at the pen start and `y` at
    /// the baseline (Pillow anchor "ls"), anti-aliased.
    pub(crate) fn draw(&self, canvas: &mut Canvas, x: f64, baseline: f64, text: &str, size: u32) {
        let scale = self.px_scale(size);
        let scaled = self.font.as_scaled(scale);
        let mut pen = x as f32;
        for ch in text.chars() {
            let glyph_id = self.font.glyph_id(ch);
            let glyph = glyph_id.with_scale_and_position(scale, point(pen, baseline as f32));
            if let Some(outlined) = self.font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i64 + i64::from(gx);
                    let py = bounds.min.y as i64 + i64::from(gy);
                    canvas.darken(px, py, coverage);
                });
            }
            pen += scaled.h_advance(glyph_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{load, path_and_index};

    #[test]
    fn face_index_suffix_must_be_numeric() {
        assert_eq!(path_and_index("/a/b.ttf").unwrap(), ("/a/b.ttf", 0));
        assert_eq!(path_and_index("/a/b.ttc#2").unwrap(), ("/a/b.ttc", 2));
        // Python's int() raises on a non-numeric tail; so do we.
        let err = path_and_index("/a/Foo#Bar.ttf").unwrap_err();
        assert!(err.0.contains("invalid ttc face index"), "{err}");
    }

    /// The auto-fit divergence case from review: Tahoma's ascender is
    /// 2049/2048 upem, so exact-real ceiling gives 13 px at size 12
    /// while FreeType's 26.6 fixed-point pipeline (and therefore
    /// Pillow's getmetrics) gives 12. Values confirmed against Pillow.
    #[cfg(target_os = "macos")]
    #[test]
    fn metrics_match_freetype_grid_rounding() {
        let tahoma = "/System/Library/Fonts/Supplemental/Tahoma.ttf";
        if !std::path::Path::new(tahoma).exists() {
            return;
        }
        let font = load(tahoma).unwrap();
        assert_eq!(font.metrics(12), (12, 3));
        assert_eq!(font.metrics(24), (25, 5));
    }
}
