// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Render label images: text, QR codes, Code 128, grids, and image
//! files — the Rust port of oh-brother's `render.py`, which remains
//! the reference implementation until the port reaches parity.
//!
//! Fonts come from the manifest-driven cache (populated by the Python
//! side until the Rust font-cache phase), host-installed fonts by
//! family name, or explicit file paths. Text falls back per character
//! to the cached icon and symbol fonts so `:symbols` work in any main
//! font. Layout math mirrors render.py number for number; rasterized
//! pixels come from ab_glyph instead of FreeType, so glyph edges may
//! differ slightly — sizes, layout boxes, and structure do not.

use std::fmt;

pub mod fontcache;
pub mod hostfonts;

mod code128;
mod font;
mod render;
mod transform;

pub use render::{
    expand_symbols, hstack, load_image, render_code128, render_grid, render_label, render_qr,
    render_text, resolve_font, TextOptions,
};

/// Dev-harness access to the font loader for the metrics_dump example
/// (tools/check_render_parity.py's Pillow cross-check); not part of
/// the supported API.
#[doc(hidden)]
pub use font::LoadedFont;
#[doc(hidden)]
pub fn load_font_for_tests(spec: &str) -> Result<std::sync::Arc<font::LoadedFont>> {
    font::load(spec)
}

/// Matches protocol DPI; duplicated (as in render.py) to keep the
/// renderer free of any printer dependency.
pub const DPI: u32 = 180;

/// Any rendering failure a caller could have caused: bad input, a
/// missing font, text that cannot fit. Maps to HTTP 400 in the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

pub type Result<T> = std::result::Result<T, RenderError>;

/// A grayscale working surface (255 = white) that text draws onto
/// anti-aliased before the final threshold — Pillow's "L" mode stage.
pub(crate) struct Canvas {
    pub width: u32,
    pub height: u32,
    pub buf: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            width,
            height,
            // usize multiply: a u32 product could wrap for huge
            // widths and desync the buffer from the dimensions.
            buf: vec![255; width as usize * height as usize],
        }
    }

    /// Composite `coverage` (0..=1) of black over the pixel, clipping
    /// out-of-bounds coordinates like Pillow's draw calls do.
    pub fn darken(&mut self, x: i64, y: i64, coverage: f32) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = (y as u32 * self.width + x as u32) as usize;
        let value = 255 - (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.buf[i] = self.buf[i].min(value);
    }
}
