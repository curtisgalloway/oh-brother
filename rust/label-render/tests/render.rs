// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Ports of tests/test_render.py's rendering assertions, plus goldens
//! generated from the Python renderer's libraries (qrcode,
//! python-barcode) — Python stays the oracle.

use label_render::{
    expand_symbols, fontcache, render_code128, render_label, render_qr, render_text, TextOptions,
};

/// Like the Python tests: use a known system font so nothing depends
/// on the user's font cache. Tests bail quietly when absent.
fn system_font() -> Option<TextOptions> {
    Some(TextOptions {
        font: Some(fontcache::system_fallback()?.to_owned()),
        ..TextOptions::default()
    })
}

fn is_binary(img: &image::GrayImage) -> bool {
    img.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255)
}

fn ink(img: &image::GrayImage) -> usize {
    img.pixels().filter(|p| p.0[0] == 0).count()
}

#[test]
fn render_text_produces_binary_image() {
    let Some(opts) = system_font() else { return };
    let img = render_text("HELLO", 76, &opts).unwrap();
    assert_eq!(img.height(), 76);
    assert!(img.width() > 20);
    assert!(is_binary(&img));
    assert!(ink(&img) > 0);
}

#[test]
fn render_label_directive_and_text() {
    let Some(opts) = system_font() else { return };
    let img = render_label("qr:test\nHI", 76, &opts).unwrap();
    assert_eq!(img.height(), 76);
    assert!(img.width() > 76);
}

#[test]
fn render_label_grid_width_is_exact() {
    let Some(opts) = system_font() else { return };
    let img = render_label("grid:1u/2\nA\nB", 76, &opts).unwrap();
    // round(42.0 / 25.4 * 180) = 298
    assert_eq!(img.width(), 298);
    assert_eq!(img.height(), 76);
}

#[test]
fn render_label_empty_raises() {
    let Some(opts) = system_font() else { return };
    let err = render_label("   \n ", 76, &opts).unwrap_err();
    assert!(err.0.contains("empty label"), "{}", err.0);
}

/// Absurd margins/sizes (the HTTP API accepts any u32) must come back
/// as clean errors — never u32 wraparound (wrong-but-200 labels) or
/// allocation aborts. Python survives these with MemoryError -> 500.
#[test]
fn oversized_labels_error_cleanly() {
    let Some(opts) = system_font() else { return };
    let huge_margin = TextOptions {
        margin_px: u32::MAX,
        ..opts.clone()
    };
    let err = render_label("HI", 76, &huge_margin).unwrap_err();
    assert!(err.0.contains("too wide"), "{err}");

    // A size this large is refused by the size cap before the width
    // check (rasterizing costs size² pixels per glyph, so the cap
    // matters even for sizes that would still fit the width bound).
    let huge_size = TextOptions {
        size: Some(1_000_000_000),
        ..opts
    };
    let err = render_label("HI", 76, &huge_size).unwrap_err();
    assert!(err.0.contains("px limit"), "{err}");
}

#[test]
fn render_label_too_many_lines_raises() {
    let Some(opts) = system_font() else { return };
    let text = vec!["x"; 40].join("\n");
    let err = render_label(&text, 76, &opts).unwrap_err();
    assert!(err.0.contains("do not fit"), "{}", err.0);
}

#[test]
fn expand_symbols_text_entries() {
    assert_eq!(expand_symbols(":warning"), "⚠");
    assert_eq!(expand_symbols(":mm2 of tape"), "mm² of tape");
    assert_eq!(expand_symbols("caution :celsius"), "caution °C");
}

#[test]
fn expand_symbols_icon_entry() {
    let icon = fontcache::global()
        .symbols()
        .iter()
        .find(|s| s["kind"] == "icon")
        .expect("manifest has icon symbols");
    let name = icon["name"].as_str().unwrap();
    let cp = char::from_u32(icon["cp"].as_u64().unwrap() as u32).unwrap();
    assert_eq!(expand_symbols(&format!(":{name}")), cp.to_string());
}

#[test]
fn expand_symbols_leaves_ordinary_colons_alone() {
    assert_eq!(expand_symbols("12:30"), "12:30");
    assert_eq!(expand_symbols(":not-a-symbol-xyz"), ":not-a-symbol-xyz");
    assert_eq!(expand_symbols("ratio 3:4"), "ratio 3:4");
}

#[test]
fn render_label_expands_symbols_in_text() {
    let Some(opts) = system_font() else { return };
    let img = render_label(":warning HOT :mm2", 76, &opts).unwrap();
    assert_eq!(img.height(), 76);
    assert!(img.width() > 20);
}

/// Module counts golden-tested against Python `qrcode` (border=2,
/// fit=True, default error correction M): version selection must
/// match for the sizing math to stay in sync with render.py.
#[test]
fn qr_module_counts_match_python_qrcode() {
    let cases: &[(&str, u32)] = &[
        ("test", 21),
        ("https://example.com/abc", 25),
        ("12V DC PSU", 21),
        ("1234567890123456789012345", 21),
    ];
    for &(data, modules) in cases {
        let code = qrcode_modules(data);
        assert_eq!(code, modules, "data {data:?}");
    }
    assert_eq!(qrcode_modules(&"A".repeat(60)), 29);
}

fn qrcode_modules(data: &str) -> u32 {
    // A 45 px tape forces 1 px boxes for every case here, so the image
    // width reads back as module count + the 4 border modules.
    let img = render_qr(data, 45).unwrap();
    img.width() - 4
}

/// render_qr sizing math, mirrored from render.py: 21+4 modules on
/// 76 px tape gives 3 px boxes and a 75 px square, centered.
#[test]
fn qr_sizing_math() {
    let img = render_qr("test", 76).unwrap();
    assert_eq!((img.width(), img.height()), (75, 76));
    let err = render_qr("test", 12).unwrap_err();
    assert!(err.0.contains("QR code needs"), "{}", err.0);
}

/// Bar geometry: pattern length * 3 px modules + 2 * 30 px quiet
/// zones; captioned below 25 px on 76 px tape.
#[test]
fn code128_geometry() {
    let Some(opts) = system_font() else { return };
    let img = render_code128("HELLO", 76, opts.font.as_deref()).unwrap();
    // python-barcode's pattern for HELLO is 90 modules.
    assert_eq!(img.width(), 90 * 3 + 2 * 30);
    assert_eq!(img.height(), 76);
    let caption_rows = 25; // max(14, min(26, 76 // 3))
    let below_bars: u32 = 76 - caption_rows;
    let caption_ink: usize = (below_bars..76)
        .flat_map(|y| (0..img.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[0] == 0)
        .count();
    assert!(caption_ink > 0, "caption should render below the bars");

    let short = render_code128("HELLO", 32, opts.font.as_deref()).unwrap();
    assert_eq!(short.height(), 32); // too short for a caption
}
