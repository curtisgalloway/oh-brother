// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! The shared font cache — fontcache.py's layout, download-on-demand
//! included.
//!
//! Every font is fetched from a commit-pinned public URL, verified
//! against the manifest's SHA-256, and stored under a per-user data
//! directory — nothing is vendored. Two transforms happen at cache
//! time (see transform.rs): `instance` entries are frozen to a static
//! instance of a variable font, and the `subset_to_symbols` entry (the
//! icon font) is reduced to exactly the codepoints the symbol catalog
//! uses. Cache file names encode the source hash and transform
//! (mirroring fontcache._cache_name exactly), so a manifest change
//! naturally invalidates stale files — and so the Python and Rust
//! sides share one cache.

use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The embedded font/symbol manifest — the same static/fonts.json the
/// Python side and the browser read.
pub const MANIFEST_BYTES: &[u8] = include_bytes!("../../../static/fonts.json");

const TIMEOUT: Duration = Duration::from_secs(60);
const USER_AGENT: &str = "oh-brother-label (+https://github.com/)";
// Far above any manifest font (largest is ~1.2 MB); a hard stop for a
// misbehaving server, not a tuning knob.
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;

/// A font could not be fetched or failed integrity verification
/// (fontcache.FontUnavailable). Callers fall back rather than fail a
/// print over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontUnavailable(pub String);

impl fmt::Display for FontUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FontUnavailable {}

pub struct FontCache {
    pub manifest: Value,
    icon_cps: Vec<u32>,
    icon_hash: String,
}

/// The process-wide cache view of the embedded manifest.
pub fn global() -> &'static FontCache {
    static CACHE: OnceLock<FontCache> = OnceLock::new();
    CACHE.get_or_init(|| FontCache::new(MANIFEST_BYTES))
}

impl FontCache {
    pub fn new(manifest_bytes: &[u8]) -> FontCache {
        let manifest: Value =
            serde_json::from_slice(manifest_bytes).expect("embedded fonts.json parses");
        // The icon codepoint set and its hash, mirroring fontcache.py:
        // the subsetted icon font's cache name varies with the catalog.
        let mut cps: Vec<i64> = manifest["symbols"]
            .as_array()
            .map(|symbols| {
                symbols
                    .iter()
                    .filter(|s| s["kind"] == "icon")
                    .filter_map(|s| s["cp"].as_i64())
                    .collect()
            })
            .unwrap_or_default();
        cps.sort_unstable();
        cps.dedup(); // Python hashes a frozenset — duplicates collapse
        let joined = cps.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let icon_hash = hex::<12>(&Sha256::digest(joined.as_bytes()));
        let icon_cps = cps
            .iter()
            .filter_map(|&cp| u32::try_from(cp).ok())
            .collect();
        FontCache {
            manifest,
            icon_cps,
            icon_hash,
        }
    }

    /// Codepoints of the `kind == "icon"` symbols, ascending
    /// (fontcache.icon_codepoints).
    pub fn icon_codepoints(&self) -> &[u32] {
        &self.icon_cps
    }

    pub fn fonts(&self) -> &[Value] {
        self.manifest["fonts"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn font_by_id(&self, id: &str) -> Option<&Value> {
        self.fonts().iter().find(|f| f["id"] == id)
    }

    pub fn visible_fonts(&self) -> impl Iterator<Item = &Value> {
        self.fonts()
            .iter()
            .filter(|f| f.get("hidden").and_then(Value::as_bool) != Some(true))
    }

    /// Sorted visible font ids, for "unknown font" error messages.
    pub fn visible_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .visible_fonts()
            .filter_map(|f| f["id"].as_str())
            .collect();
        ids.sort_unstable();
        ids
    }

    pub fn default_font_id(&self) -> &str {
        self.manifest["default_font"]
            .as_str()
            .expect("manifest default_font")
    }

    /// The font id an alias points at, if any (aliases are lowercase).
    pub fn alias_target(&self, alias: &str) -> Option<&str> {
        for font in self.fonts() {
            if let Some(aliases) = font.get("aliases").and_then(Value::as_array) {
                if aliases.iter().any(|a| a == alias) {
                    return font["id"].as_str();
                }
            }
        }
        None
    }

    pub fn symbols(&self) -> &[Value] {
        self.manifest["symbols"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn cache_name(&self, font: &Value) -> String {
        let id = font["id"].as_str().unwrap_or_default();
        let sha = font["sha256"].as_str().unwrap_or_default();
        let mut stem = format!("{}-{}", id, &sha[..12.min(sha.len())]);
        if let Some(instance) = font.get("instance").and_then(Value::as_object) {
            // serde_json objects iterate in insertion order; sort like
            // Python's sorted(items()).
            let mut axes: Vec<_> = instance.iter().collect();
            axes.sort_by_key(|(axis, _)| axis.as_str());
            for (axis, value) in axes {
                stem.push_str(&format!("-{axis}{value}"));
            }
        }
        if font.get("subset_to_symbols").and_then(Value::as_bool) == Some(true) {
            stem.push('-');
            stem.push_str(&self.icon_hash);
        }
        stem + ".ttf"
    }

    /// Where a manifest font id would be cached (whether or not the
    /// file exists) — fontcache._cache_name resolved against cache_dir.
    pub fn cache_path(&self, id: &str) -> Option<PathBuf> {
        let font = self.font_by_id(id)?;
        Some(cache_dir().join(self.cache_name(font)))
    }

    /// Cached path for a manifest font id, or None. Never networks.
    pub fn path_for(&self, id: &str) -> Option<PathBuf> {
        let path = self.cache_path(id)?;
        path.exists().then_some(path)
    }

    /// Any usable font path without touching the network: the cached
    /// default, any cached visible font, then a system font.
    pub fn best_effort_font(&self) -> Option<String> {
        if let Some(path) = self.path_for(self.default_font_id()) {
            return Some(path.to_string_lossy().into_owned());
        }
        for font in self.visible_fonts() {
            if let Some(path) = self.path_for(font["id"].as_str().unwrap_or_default()) {
                return Some(path.to_string_lossy().into_owned());
            }
        }
        system_fallback().map(str::to_owned)
    }

    /// raw.githubusercontent URL for a manifest-relative path under
    /// the font's commit pin (fontcache._source_url / _license_url).
    fn pinned_url(&self, font: &Value, path_key: &str) -> Result<String, FontUnavailable> {
        let pin_name = font["pin"].as_str().unwrap_or_default();
        let pin = &self.manifest["pins"][pin_name];
        let (Some(repo), Some(commit), Some(path)) = (
            pin["repo"].as_str(),
            pin["commit"].as_str(),
            font[path_key].as_str(),
        ) else {
            return Err(FontUnavailable(format!(
                "manifest entry {} has no usable {path_key}/pin",
                font["id"]
            )));
        };
        Ok(format!(
            "https://raw.githubusercontent.com/{repo}/{commit}/{}",
            percent_encode(path)
        ))
    }

    /// Fetch the manifest-relative file at `entry[path_key]` under the
    /// entry's pin: repo+commit pins fetch from raw.githubusercontent,
    /// url pins fetch a sha256-verified release archive and extract
    /// that member (needed for prebuilt static instances that upstream
    /// ships only inside release zips, e.g. Inter's static weights).
    fn fetch_pinned(&self, entry: &Value, path_key: &str) -> Result<Vec<u8>, FontUnavailable> {
        let pin = &self.manifest["pins"][entry["pin"].as_str().unwrap_or_default()];
        let Some(url) = pin["url"].as_str() else {
            return download(&self.pinned_url(entry, path_key)?);
        };
        let Some(member) = entry[path_key].as_str() else {
            return Err(FontUnavailable(format!(
                "manifest entry {} has no usable {path_key}",
                entry["id"]
            )));
        };
        let archive = archive_bytes(url, pin["sha256"].as_str().unwrap_or_default())?;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive.as_slice()))
            .map_err(|e| FontUnavailable(format!("cannot open archive {url}: {e}")))?;
        let mut file = zip
            .by_name(member)
            .map_err(|e| FontUnavailable(format!("{member} not in archive {url}: {e}")))?;
        let mut out = Vec::new();
        file.read_to_end(&mut out)
            .map_err(|e| FontUnavailable(format!("cannot extract {member}: {e}")))?;
        Ok(out)
    }

    /// Verify a downloaded blob against the manifest hash, apply the
    /// cache-time transform, and write it atomically. Factored out of
    /// `ensure` so the verify path is testable without a network.
    fn cache_verified(&self, entry: &Value, blob: Vec<u8>) -> Result<PathBuf, FontUnavailable> {
        let font_id = entry["id"].as_str().unwrap_or_default();
        let expected = entry["sha256"].as_str().unwrap_or_default();
        let digest = hex::<64>(&Sha256::digest(&blob));
        if digest != expected {
            return Err(FontUnavailable(format!(
                "{font_id}: hash mismatch (expected {}…, got {}…) — refusing to cache",
                &expected[..12.min(expected.len())],
                &digest[..12]
            )));
        }
        let data = crate::transform::transform(entry, blob, &self.icon_cps)?;
        let target = cache_dir().join(self.cache_name(entry));
        let parent = target.parent().expect("cache path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|e| FontUnavailable(format!("cannot create cache dir: {e}")))?;
        // .part + rename, like fontcache.py: never leave a partial or
        // unverified file at the final name.
        let tmp = target.with_extension("part");
        std::fs::write(&tmp, &data)
            .map_err(|e| FontUnavailable(format!("cannot write cache file: {e}")))?;
        std::fs::rename(&tmp, &target)
            .map_err(|e| FontUnavailable(format!("cannot finalize cache file: {e}")))?;
        Ok(target)
    }

    /// Best-effort license text alongside the cached font
    /// (fontcache._ensure_license): warns on stderr, never fails.
    fn ensure_license(&self, entry: &Value) {
        let font_id = entry["id"].as_str().unwrap_or_default();
        let lic_path = cache_dir().join("licenses").join(format!("{font_id}.txt"));
        if lic_path.exists() {
            return;
        }
        let fetched = self.fetch_pinned(entry, "license_path");
        let data = match fetched {
            Ok(data) => data,
            Err(e) => {
                eprintln!("warning: could not fetch license for {font_id}: {e}");
                return;
            }
        };
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(lic_path.parent().unwrap())?;
            let tmp = lic_path.with_extension("part");
            std::fs::write(&tmp, &data)?;
            std::fs::rename(&tmp, &lic_path)
        };
        if let Err(e) = write() {
            eprintln!("warning: could not store license for {font_id}: {e}");
        }
    }

    /// The cached path for a font id, downloading it if needed
    /// (fontcache.ensure). Never leaves a partial or unverified file
    /// in the cache.
    pub fn ensure(&self, font_id: &str) -> Result<PathBuf, FontUnavailable> {
        let Some(entry) = self.font_by_id(font_id) else {
            return Err(FontUnavailable(format!("unknown font id {font_id:?}")));
        };
        let target = cache_dir().join(self.cache_name(entry));
        if target.exists() {
            return Ok(target);
        }
        let blob = self.fetch_pinned(entry, "path")?;
        let target = self.cache_verified(entry, blob)?;
        self.ensure_license(entry);
        Ok(target)
    }

    /// The cached license text for a font id, fetching it if missing.
    /// Strict where `ensure` only warns: a redistributable export must
    /// not ship a font without its license text (OFL/Apache terms).
    pub fn license_path_strict(&self, font_id: &str) -> Result<PathBuf, FontUnavailable> {
        let Some(entry) = self.font_by_id(font_id) else {
            return Err(FontUnavailable(format!("unknown font id {font_id:?}")));
        };
        let path = cache_dir().join("licenses").join(format!("{font_id}.txt"));
        if !path.exists() {
            self.ensure_license(entry);
        }
        if path.exists() {
            Ok(path)
        } else {
            Err(FontUnavailable("license text unavailable".into()))
        }
    }

    /// Fetch every manifest font, reporting progress lines; returns
    /// the ids that could not be fetched (fontcache.ensure_all).
    pub fn ensure_all(&self, mut progress: impl FnMut(&str)) -> Vec<String> {
        let mut failed = Vec::new();
        for entry in self.fonts() {
            let id = entry["id"].as_str().unwrap_or_default();
            if entry.get("subset_to_symbols").and_then(Value::as_bool) == Some(true)
                && self.icon_cps.is_empty()
            {
                continue;
            }
            let cached = self.path_for(id).is_some();
            match self.ensure(id) {
                Ok(path) => {
                    let state = if cached { "cached" } else { "fetched" };
                    let kb = path.metadata().map(|m| m.len() / 1024).unwrap_or(0);
                    progress(&format!("{id}: {state} ({kb} KB)"));
                }
                Err(e) => {
                    failed.push(id.to_owned());
                    progress(&format!("{id}: FAILED — {e}"));
                }
            }
        }
        failed
    }
}

/// Percent-encode like urllib.parse.quote's default: RFC 3986
/// unreserved characters and '/' pass through, everything else is
/// %XX-escaped byte-wise.
fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// A release archive, downloaded and sha256-verified at most once per
/// process. Only the Inter release zip today (~34 MB); the memo holds
/// it for the process lifetime, which is acceptable at one archive —
/// revisit if the manifest ever grows several.
fn archive_bytes(
    url: &str,
    expected_sha: &str,
) -> Result<std::sync::Arc<Vec<u8>>, FontUnavailable> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    static ARCHIVES: OnceLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> = OnceLock::new();
    let memo = ARCHIVES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(bytes) = memo.lock().unwrap().get(url) {
        return Ok(bytes.clone());
    }
    let blob = download(url)?;
    let digest = hex::<64>(&Sha256::digest(&blob));
    if digest != expected_sha {
        return Err(FontUnavailable(format!(
            "archive {url}: hash mismatch (expected {}…, got {}…) — refusing to use",
            &expected_sha[..12.min(expected_sha.len())],
            &digest[..12]
        )));
    }
    let arc = Arc::new(blob);
    memo.lock().unwrap().insert(url.to_owned(), arc.clone());
    Ok(arc)
}

fn download(url: &str) -> Result<Vec<u8>, FontUnavailable> {
    // Air-gap/test switch: refuse the network outright (the Python
    // tests get the same behavior by monkeypatching _download).
    if std::env::var_os("OH_BROTHER_FONT_OFFLINE").is_some() {
        return Err(FontUnavailable(format!(
            "network disabled by OH_BROTHER_FONT_OFFLINE: {url}"
        )));
    }
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
    });
    let response = agent
        .get(url)
        .call()
        .map_err(|e| FontUnavailable(format!("cannot fetch {url}: {e}")))?;
    let mut data = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD)
        .read_to_end(&mut data)
        .map_err(|e| FontUnavailable(format!("cannot fetch {url}: {e}")))?;
    Ok(data)
}

pub fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OH_BROTHER_FONT_CACHE") {
        return PathBuf::from(dir);
    }
    // Matches platformdirs.user_data_path("oh-brother") on every
    // platform, so Python and Rust share one cache: macOS Application
    // Support and Linux ~/.local/share get one "oh-brother" segment;
    // Windows gets AppData\Local\oh-brother\oh-brother because
    // platformdirs defaults appauthor to the appname and appends both.
    let base = dirs::data_local_dir().expect("user data dir");
    #[cfg(target_os = "windows")]
    let base = base.join("oh-brother");
    base.join("oh-brother").join("fonts")
}

// Last-resort fonts when offline with an empty cache: never fail a
// print just because a download couldn't happen. Probed in order,
// first hit wins. Mirrors fontcache._SYSTEM_FALLBACKS.
const SYSTEM_FALLBACKS: &[&str] = &[
    // macOS
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    // Linux
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    // Windows
    "C:/Windows/Fonts/arial.ttf",
];

pub fn system_fallback() -> Option<&'static str> {
    SYSTEM_FALLBACKS
        .iter()
        .copied()
        .find(|path| PathBuf::from(path).exists())
}

fn hex<const N: usize>(digest: &[u8]) -> String {
    digest
        .iter()
        .flat_map(|b| [b >> 4, b & 0xf])
        .take(N)
        .map(|n| char::from_digit(u32::from(n), 16).unwrap())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{global, percent_encode};

    /// Cache filenames pinned against the retired Python side's
    /// fontcache._cache_name so existing user caches stay valid.
    /// Goldens cover each naming variant: plain, instanced, and
    /// subset-to-symbols (whose suffix hashes the icon catalog). A
    /// legitimate manifest change (font re-pin, icon catalog edit)
    /// updates these — recompute by hand per the scheme in
    /// `cache_name` above (id-sha12[-axisvalue…][-iconhash12].ttf).
    #[test]
    fn cache_names_match_python() {
        let cases = [
            ("inter", "inter-29160a80ff49.ttf"),
            // repinned 2026-08-15 to the official static Inter-Bold
            // (no instance transform, so no axis suffix)
            ("inter-bold", "inter-bold-288316099b1e.ttf"),
            ("icons", "icons-b5126c4655e0-6fffd1602a1e.ttf"),
        ];
        for (id, expected) in cases {
            let path = global().cache_path(id).expect("manifest id exists");
            assert_eq!(
                path.file_name().unwrap().to_str().unwrap(),
                expected,
                "cache name for {id}"
            );
        }
    }

    /// Source/license URLs pinned against Python's _source_url /
    /// _license_url output (which uses urllib.parse.quote).
    #[test]
    fn pinned_urls_match_python() {
        let cache = global();
        let inter = cache.font_by_id("inter").unwrap();
        assert_eq!(
            cache.pinned_url(inter, "path").unwrap(),
            "https://raw.githubusercontent.com/google/fonts/\
             7ff85c87f93ea6cca5f41c69f2e4edcb90240f26/ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf"
        );
        assert_eq!(
            cache.pinned_url(inter, "license_path").unwrap(),
            "https://raw.githubusercontent.com/google/fonts/\
             7ff85c87f93ea6cca5f41c69f2e4edcb90240f26/ofl/inter/OFL.txt"
        );
    }

    #[test]
    fn percent_encode_matches_urllib_quote() {
        assert_eq!(
            percent_encode("ofl/inter/Inter[opsz,wght].ttf"),
            "ofl/inter/Inter%5Bopsz%2Cwght%5D.ttf"
        );
        assert_eq!(percent_encode("a b~c-d_e.f/g"), "a%20b~c-d_e.f/g");
    }

    /// A blob that doesn't hash to the manifest value is refused
    /// before anything touches the cache directory.
    #[test]
    fn junk_blob_refused() {
        let cache = global();
        let inter = cache.font_by_id("inter").unwrap();
        let err = cache.cache_verified(inter, b"junk".to_vec()).unwrap_err();
        assert!(err.0.contains("hash mismatch"), "{err}");
        assert!(err.0.contains("refusing to cache"), "{err}");
    }

    #[test]
    fn ensure_rejects_unknown_id() {
        let err = global().ensure("no-such-font").unwrap_err();
        assert!(err.0.contains("unknown font id"), "{err}");
    }
}
