// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Enumerate fonts installed on this machine, by family name.
//!
//! Port of hostfonts.py: lets `--font "Some Family"` and the web UI's
//! host-fonts section address whatever the OS has installed. Faces
//! inside a .ttc collection are addressed as `path#N`, the same
//! convention the renderer's font loader understands.

use std::path::PathBuf;
use std::sync::OnceLock;

/// One host font family: name, loadable spec (path or path#N), and
/// the writing systems its cmap covers.
#[derive(Clone)]
pub struct HostFont {
    pub family: String,
    pub spec: String,
    pub scripts: Vec<&'static str>,
}

/// Writing systems a host font can be probed for: a face "supports" a
/// script when its cmap has every probe character. Mirrors
/// hostfonts.SCRIPT_PROBES and index.html's language→script map —
/// keep all three identical.
const SCRIPT_PROBES: &[(&str, &str)] = &[
    ("latin", "Ag"),
    ("cyrillic", "Яж"),
    ("greek", "Ωλ"),
    ("arabic", "عب"),
    ("hebrew", "אב"),
    ("devanagari", "कह"),
    ("han", "永中"),
    ("kana", "あア"),
    ("hangul", "한글"),
    ("thai", "กข"),
];

/// All host fonts on this machine; first occurrence of a family wins.
/// Scanned once per process, like hostfonts._scan.
fn scan() -> &'static [HostFont] {
    static SCAN: OnceLock<Vec<HostFont>> = OnceLock::new();
    SCAN.get_or_init(|| {
        let mut seen: Vec<HostFont> = Vec::new();
        for base in font_dirs() {
            let mut paths = Vec::new();
            collect_font_files(&base, &mut paths);
            paths.sort();
            for path in paths {
                for font in families_in_file(&path) {
                    let key = font.family.to_lowercase();
                    if !seen.iter().any(|f| f.family.to_lowercase() == key) {
                        seen.push(font);
                    }
                }
            }
        }
        seen
    })
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/System/Library/Fonts/Supplemental"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join("Library/Fonts"));
        }
    } else if cfg!(target_os = "windows") {
        dirs.push(PathBuf::from("C:/Windows/Fonts"));
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Microsoft/Windows/Fonts"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        dirs.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".local/share/fonts"));
            dirs.push(home.join(".fonts"));
        }
    }
    dirs.retain(|d| d.is_dir());
    dirs
}

fn collect_font_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, out);
        } else if matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref(),
            Some("ttf" | "otf" | "ttc")
        ) {
            out.push(path);
        }
    }
}

/// The face's family name, mirroring fontTools getBestFamilyName /
/// getDebugName: WWS family (21) beats typographic family (16) beats
/// family (1); within a name id, an English record — Mac platform
/// language 0 or Windows language 0x409 — wins outright, otherwise
/// the last decodable record does.
fn family_name(face: &ttf_parser::Face) -> Option<String> {
    for name_id in [
        ttf_parser::name_id::WWS_FAMILY,
        ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
        ttf_parser::name_id::FAMILY,
    ] {
        let mut some_name = None;
        for record in face.names() {
            if record.name_id != name_id {
                continue;
            }
            let decoded = if record.is_unicode() {
                record.to_string()
            } else if record.platform_id == ttf_parser::PlatformId::Macintosh {
                // Mac Roman: ASCII is the common case; skip anything else.
                decode_ascii(record.name)
            } else {
                None
            };
            let Some(name) = decoded else { continue };
            let name = name.trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let english = (record.platform_id == ttf_parser::PlatformId::Macintosh
                && record.language_id == 0)
                || (record.platform_id == ttf_parser::PlatformId::Windows
                    && record.language_id == 0x409);
            if english {
                return Some(name);
            }
            some_name = Some(name);
        }
        if some_name.is_some() {
            return some_name;
        }
    }
    None
}

fn decode_ascii(bytes: &[u8]) -> Option<String> {
    bytes
        .iter()
        .all(u8::is_ascii)
        .then(|| String::from_utf8_lossy(bytes).into_owned())
}

/// Scripts whose probe characters are all in the face's cmap.
fn scripts(face: &ttf_parser::Face) -> Vec<&'static str> {
    SCRIPT_PROBES
        .iter()
        .filter(|(_, chars)| chars.chars().all(|c| face.glyph_index(c).is_some()))
        .map(|(script, _)| *script)
        .collect()
}

/// Host fonts in one file; spec is path or path#N.
///
/// Families starting with "." are skipped: those are hidden system
/// fonts (macOS ".SF NS" and friends) that Font Book hides and that
/// CSS cannot address by family name, so listing them only produces
/// picker entries that render blank.
fn families_in_file(path: &PathBuf) -> Vec<HostFont> {
    let Ok(data) = std::fs::read(path) else {
        return Vec::new();
    };
    let path_str = path.to_string_lossy();
    let mut out = Vec::new();
    let faces = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    for i in 0..faces {
        // Unreadable or non-sfnt faces: not candidates, skip.
        let Ok(face) = ttf_parser::Face::parse(&data, i) else {
            continue;
        };
        if let Some(family) = family_name(&face) {
            if family.starts_with('.') {
                continue;
            }
            let spec = if i == 0 {
                path_str.to_string()
            } else {
                format!("{path_str}#{i}")
            };
            out.push(HostFont {
                family,
                spec,
                scripts: scripts(&face),
            });
        }
    }
    out
}

pub fn path_for_family(name: &str) -> Option<String> {
    let target = name.trim().to_lowercase();
    scan()
        .iter()
        .find(|f| f.family.to_lowercase() == target)
        .map(|f| f.spec.clone())
}

/// Sorted host fonts for UI listings, original capitalization.
pub fn list_families() -> Vec<HostFont> {
    let mut families: Vec<HostFont> = scan().to_vec();
    families.sort_by_key(|f| f.family.to_lowercase());
    families
}
