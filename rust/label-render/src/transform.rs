// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Cache-time font transforms, via skera (Google Fonts' pure-Rust
//! subsetter from the fontations project — no C, no bindgen):
//!
//! - `subset_to_symbols`: reduce the icon font to exactly the symbol
//!   catalog's codepoints, keeping the .notdef outline. Variation
//!   tables are kept (unlike the retired hb-subset/fontTools
//!   pipeline, which pinned axes to their defaults first) — every
//!   consumer renders the default instance, so the output is
//!   equivalent, just not axis-stripped.
//!
//! The old `instance` transform (freeze a variable font to a static
//! instance) is gone: the one manifest entry that used it now pins an
//! official prebuilt static file instead, and skera does not instance.
//! A manifest entry that still asks for it gets a loud error rather
//! than a silently un-instanced font.

use serde_json::Value;
use skera::{subset_font, Plan, SubsetFlags, DEFAULT_DROP_TABLES, DEFAULT_LAYOUT_FEATURES};
use write_fonts::read::collections::IntSet;
use write_fonts::read::types::{NameId, Tag};
use write_fonts::read::FontRef;

use crate::fontcache::FontUnavailable;

/// Apply the manifest-declared cache-time transform, if any.
pub fn transform(
    entry: &Value,
    blob: Vec<u8>,
    icon_cps: &[u32],
) -> Result<Vec<u8>, FontUnavailable> {
    let font_id = entry["id"].as_str().unwrap_or_default();
    if entry.get("instance").is_some() {
        return Err(FontUnavailable(format!(
            "{font_id}: the manifest asks for an `instance` transform, which is no \
             longer supported — pin a prebuilt static font file instead"
        )));
    }
    if entry.get("subset_to_symbols").and_then(Value::as_bool) != Some(true) {
        return Ok(blob);
    }
    if icon_cps.is_empty() {
        return Err(FontUnavailable(format!(
            "font {font_id} is subset_to_symbols but the manifest has no icon symbols"
        )));
    }
    subset_icons(font_id, &blob, icon_cps)
}

/// Subset to exactly `cps`, .notdef outline retained. The remaining
/// Plan inputs replicate skera's own CLI defaults (which mirror
/// hb-subset's): drop the default table list, keep all layout
/// scripts, the default layout-feature set, name ids 0–6, and
/// English (0x409) name records.
fn subset_icons(font_id: &str, data: &[u8], cps: &[u32]) -> Result<Vec<u8>, FontUnavailable> {
    let font = FontRef::new(data)
        .map_err(|e| FontUnavailable(format!("{font_id}: cannot parse font: {e}")))?;
    let unicodes: IntSet<u32> = cps.iter().copied().collect();
    let drop_tables: IntSet<Tag> = DEFAULT_DROP_TABLES.iter().copied().collect();
    let mut layout_scripts = IntSet::<Tag>::empty();
    layout_scripts.invert(); // all scripts
    let layout_features: IntSet<Tag> = DEFAULT_LAYOUT_FEATURES.iter().copied().collect();
    let mut name_ids = IntSet::<NameId>::empty();
    name_ids.insert_range(NameId::from(0)..=NameId::from(6));
    let mut name_languages = IntSet::<u16>::empty();
    name_languages.insert(0x0409);
    let plan = Plan::new(
        &IntSet::empty(),
        &unicodes,
        &font,
        SubsetFlags::SUBSET_FLAGS_NOTDEF_OUTLINE,
        &drop_tables,
        &layout_scripts,
        &layout_features,
        &name_ids,
        &name_languages,
    );
    subset_font(&font, &plan)
        .map_err(|e| FontUnavailable(format!("{font_id}: icon subsetting failed: {e}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::transform;

    /// The cached Inter (a variable font, cached untransformed)
    /// doubles as subset input; tests bail quietly when the machine
    /// has no cache yet.
    fn variable_font() -> Option<Vec<u8>> {
        let path = crate::fontcache::global().path_for("inter")?;
        std::fs::read(path).ok()
    }

    #[test]
    fn no_transform_passes_blob_through() {
        let entry = json!({"id": "t"});
        assert_eq!(transform(&entry, b"blob".to_vec(), &[]).unwrap(), b"blob");
    }

    #[test]
    fn instance_transform_is_refused_loudly() {
        let entry = json!({"id": "t", "instance": {"wght": 700}});
        let err = transform(&entry, b"blob".to_vec(), &[]).unwrap_err();
        assert!(err.0.contains("no longer supported"), "{err}");
    }

    /// Icon-style subsetting: exactly the requested codepoints remain.
    /// Variation tables are deliberately retained (default-instance
    /// rendering is what every consumer does).
    #[test]
    fn subsetting_keeps_exactly_the_catalog() {
        let Some(data) = variable_font() else { return };
        let entry = json!({"id": "t", "subset_to_symbols": true});
        let out = transform(&entry, data, &[65, 66]).unwrap();
        let face = ttf_parser::Face::parse(&out, 0).expect("output parses");
        assert!(face.glyph_index('A').is_some());
        assert!(face.glyph_index('B').is_some());
        assert!(face.glyph_index('C').is_none());
        assert!(face.is_variable(), "variation tables should survive");
    }

    #[test]
    fn subsetting_without_symbols_is_refused() {
        let entry = json!({"id": "t", "subset_to_symbols": true});
        let err = transform(&entry, b"blob".to_vec(), &[]).unwrap_err();
        assert!(err.0.contains("no icon symbols"), "{err}");
    }
}
