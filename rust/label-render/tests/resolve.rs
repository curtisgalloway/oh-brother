// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Ports of tests/test_render.py's font-resolution assertions. This
//! lives in its own integration binary (= its own process) because it
//! points OH_BROTHER_FONT_CACHE at scratch directories, and the env
//! var is process-global; the scenarios run inside one #[test] so the
//! mutations stay single-threaded.

use std::fs;
use std::path::PathBuf;

use label_render::{fontcache, resolve_font};

fn scratch_cache(name: &str) -> PathBuf {
    // Like the Python tests: no cache AND no network — resolution must
    // behave offline (the fixture there monkeypatches _download).
    std::env::set_var("OH_BROTHER_FONT_OFFLINE", "1");
    let dir = std::env::temp_dir()
        .join(format!("label-render-resolve-{}", std::process::id()))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    std::env::set_var("OH_BROTHER_FONT_CACHE", &dir);
    dir
}

/// Pretend font_id is cached (content junk — only paths are tested).
fn seed_cache(font_id: &str) -> String {
    let target = fontcache::global().cache_path(font_id).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"junk").unwrap();
    target.to_string_lossy().into_owned()
}

#[test]
fn resolve_font_ladder() {
    // Scenario: empty cache, no network — the default font falls back
    // loudly to a system font rather than failing the print.
    scratch_cache("empty");
    if let Some(system) = fontcache::system_fallback() {
        assert_eq!(resolve_font(None).unwrap(), system);

        // Explicit paths pass through untouched.
        assert_eq!(resolve_font(Some(system)).unwrap(), system);
    }

    // Missing files and unknown names fail with pointed messages.
    let err = resolve_font(Some("/no/such/font.ttf")).unwrap_err();
    assert!(err.0.contains("font file not found"), "{}", err.0);
    let err = resolve_font(Some("definitely-not-a-font-xyz")).unwrap_err();
    assert!(err.0.contains("unknown font"), "{}", err.0);

    // Scenario: seeded cache — ids and aliases resolve to cached paths.
    scratch_cache("seeded");
    let inter = seed_cache("inter");
    let plex = seed_cache("plex-mono");
    assert_eq!(resolve_font(Some("inter")).unwrap(), inter);
    // Brother device-font aliases…
    assert_eq!(resolve_font(Some("helsinki")).unwrap(), inter);
    assert_eq!(resolve_font(Some("HELSINKI")).unwrap(), inter);
    // …and legacy macOS shortcuts.
    assert_eq!(resolve_font(Some("menlo")).unwrap(), plex);
    assert_eq!(resolve_font(Some("courier")).unwrap(), plex);
}

/// The host-font rung of the ladder: a family that is installed on
/// every macOS box and is not a manifest id or alias.
#[cfg(target_os = "macos")]
#[test]
fn resolve_host_family() {
    let path = label_render::hostfonts::path_for_family("courier new")
        .expect("Courier New is installed on macOS");
    assert!(path.to_lowercase().contains("courier"), "{path}");
}
