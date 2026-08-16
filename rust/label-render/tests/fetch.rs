// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Live-network fetch test, ignored by default — run explicitly with
//! `cargo test -p label-render --test fetch -- --ignored`. Its own
//! integration binary (= its own process) because it repoints
//! OH_BROTHER_FONT_CACHE.

use label_render::fontcache;

/// One real download end to end: fetch the smallest manifest font
/// into a scratch cache, verify the file and license landed, and that
/// a second ensure() is a cache hit.
#[test]
#[ignore = "touches the network"]
fn ensure_fetches_verifies_and_caches() {
    let dir = std::env::temp_dir().join(format!("label-render-fetch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("OH_BROTHER_FONT_CACHE", &dir);

    let cache = fontcache::global();
    let path = cache.ensure("atkinson").expect("fetch succeeds");
    assert!(path.exists());
    assert!(path.metadata().unwrap().len() > 10_000);
    // The license text lands alongside, like fontcache._ensure_license.
    assert!(dir.join("licenses").join("atkinson.txt").exists());
    // Fetched bytes parse as a font.
    let data = std::fs::read(&path).unwrap();
    assert!(ttf_parser::Face::parse(&data, 0).is_ok());
    // Idempotent: the second call returns the cached file.
    assert_eq!(cache.ensure("atkinson").unwrap(), path);

    let _ = std::fs::remove_dir_all(&dir);
}
