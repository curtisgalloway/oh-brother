// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Dev harness for tools/check_render_parity.py: render one label to
//! a PNG and report its geometry. Not part of the shipped CLI.
//!
//! Usage: render_dump HEIGHT_PX OUT.png TEXT [FONT]
//! (literal \n in TEXT starts a new label line, like the CLI)

use label_render::{render_label, TextOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (height, out, text, font) = match args.as_slice() {
        [height, out, text] => (height, out, text, None),
        [height, out, text, font] => (height, out, text, Some(font.clone())),
        _ => {
            eprintln!("usage: render_dump HEIGHT_PX OUT.png TEXT [FONT]");
            std::process::exit(2);
        }
    };
    let height: u32 = height.parse().expect("HEIGHT_PX is a number");
    let img = match render_label(
        &text.replace("\\n", "\n"),
        height,
        &TextOptions {
            font,
            ..TextOptions::default()
        },
    ) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("render failed: {e}");
            std::process::exit(1);
        }
    };
    img.save(out).expect("PNG save");
    let ink = img.pixels().filter(|p| p.0[0] == 0).count();
    println!("{} {} {}", img.width(), img.height(), ink);
}
