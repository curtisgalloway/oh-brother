// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Dev harness for tools/check_render_parity.py's metrics sweep:
//! print "size ascent descent" for a font over a size range, to be
//! diffed against Pillow's getmetrics (the FreeType oracle).
//!
//! Usage: metrics_dump FONT_PATH MIN_SIZE MAX_SIZE

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [font_path, min_size, max_size] = args.as_slice() else {
        eprintln!("usage: metrics_dump FONT_PATH MIN_SIZE MAX_SIZE");
        std::process::exit(2);
    };
    let min: u32 = min_size.parse().expect("MIN_SIZE is a number");
    let max: u32 = max_size.parse().expect("MAX_SIZE is a number");
    let font = label_render::load_font_for_tests(font_path).unwrap_or_else(|e| {
        eprintln!("cannot load {font_path}: {e}");
        std::process::exit(1);
    });
    for size in min..=max {
        let (ascent, descent) = font.metrics(size);
        println!("{size} {ascent} {descent}");
    }
}
