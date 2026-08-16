<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# Plan: replace Brother fonts with a dynamically fetched open-font set

> **Status: implemented, 2026-07-29** (phases 1–7, one commit each).
> Deltas from the plan as written:
>
> - Cmap validation during the symbol re-key found ten glyphs (incl.
>   the ground/AC/DC electrical symbols) missing from Noto Sans
>   Symbols 2 — **Noto Sans Symbols** (the original) and **Noto Emoji**
>   (monochrome; also gives typed emoji real glyphs) joined the hidden
>   fallback set.
> - CLI `:symbol` expansion shipped as part of phase 3 rather than as a
>   stretch item (`render.expand_symbols`).
> - The Code 128 cross-renderer test surfaced a pre-existing, accepted
>   divergence: python-barcode opens in Code C for leading digit pairs;
>   labelcore always uses Code B. Documented in AGENTS.md sync points.
> - A latent `render_qr` bug (box_size kwarg collision with qrcode>=7.4)
>   was caught by the new tests and fixed in phase 2.

## Goal

Remove every dependency on Brother's P-touch Editor fonts (the 14 device
faces and the 6 symbol fonts) and on macOS system font paths. Replace them
with a curated set of OFL/Apache-licensed fonts, downloaded on demand from
pinned public URLs into a per-user cache — never committed to the repo.
Add a preview-driven font picker to the web UI. After this lands, nothing
in the repo or its docs may reference Brother font files, the P-touch
installer, or any machine-specific path, so the repo is publishable as-is.

**Non-goals:** visual fidelity to Brother's original faces or symbol
glyphs (explicitly waived); Windows host-font polish beyond the scan-dir
list; CLI `:symbol` name expansion (stretch item only).

## Decisions already made (do not re-litigate)

- Fonts are **downloaded at runtime/deploy time, not vendored**. The repo
  contains only a manifest with pinned URLs + SHA-256 hashes.
- Symbols: **option 1** — re-key the curated `:name` catalog onto one icon
  font (Material Symbols Outlined, Apache-2.0), except entries that are
  really *text* (units, currency), which become plain Unicode expansions
  in the main font.
- Brother font names (`helsinki`, …) survive as **aliases** onto the new
  set; the picker may show a "≈ Helsinki" badge.
- Host-installed fonts are usable: server-side scan in server mode,
  `queryLocalFonts()` in standalone mode.
- Picker previews use the current label text, falling back to
  **per-category sample strings** when the label is empty.
- Static/WebUSB deploys get fonts via a **deploy-time fetch**, not a
  third-party CDN at page load.

## The font set

One `id` per face. `brother` marks the alias it absorbs. All from the
`google/fonts` GitHub repo (OFL) unless noted.

| id | family | category | brother alias | notes |
|---|---|---|---|---|
| `inter` | Inter | sans | helsinki | **new default font** |
| `inter-bold` | Inter Bold | sans | adams | static bold instance, own tile |
| `source-serif` | Source Serif 4 | serif | brussels | |
| `plex-mono` | IBM Plex Mono | mono | letter-gothic | Letter Gothic lineage |
| `libre-baskerville` | Libre Baskerville | serif | us | |
| `archivo-black` | Archivo Black | display | florida | |
| `varela-round` | Varela Round | rounded | belgium | |
| `bodoni-moda` | Bodoni Moda | serif | san-diego | |
| `roboto-slab` | Roboto Slab | slab | los-angeles | |
| `great-vibes` | Great Vibes | script | calgary | |
| `jost` | Jost | sans | atlanta | Futura revival |
| `pacifico` | Pacifico | script | brunei | |
| `grenze-gotisch` | Grenze Gotisch | blackletter | germany | |
| `barlow` | Barlow | sans | sofia | DIN flavor |
| `bebas-neue` | Bebas Neue | display | — | condensed display classic |
| `archivo-narrow` | Archivo Narrow | condensed | — | practical on 6/9 mm tape |
| `saira-stencil` | Saira Stencil One | stencil | — | |
| `caveat` | Caveat | handwriting | — | |
| `vt323` | VT323 | pixel | — | LCD look |
| `atkinson` | Atkinson Hyperlegible | sans | — | max legibility |
| `noto-symbols` | Noto Sans Symbols 2 | (fallback only) | — | Unicode symbol coverage |
| `icons` | Material Symbols Outlined | (symbols only) | — | Apache-2.0, subset at cache time |

Old non-Brother macOS shortcuts (`menlo`, `futura`, `impact`, …) are
**removed** as shortcuts — their roles are covered by the set above, and
the faces remain reachable via host-font discovery on Macs.

Legacy alias map kept in the manifest: `helvetica→inter`,
`arial→inter`, `menlo→plex-mono`, `monaco→plex-mono`,
`courier→plex-mono`, `futura→jost`, `impact→archivo-black`,
`din→barlow`, `din-condensed→barlow`, `avenir→jost`,
`typewriter→plex-mono`, plus the 14 Brother names per the table.

## Architecture

### `src/oh_brother/static/fonts.json` — single source of truth

Lives in `static/` so the browser fetches the same file Python reads
(replaces the three parallel dicts `_FONT_SHORTCUTS` / `CSS_FAMILIES` /
`CSS_WEIGHTS` and both symbol tables). Schema:

```jsonc
{
  "pins": {
    // immutable raw.githubusercontent.com bases, resolved at impl time
    "google_fonts_commit": "<sha>",
    "material_icons_commit": "<sha>"
  },
  "fonts": [
    {
      "id": "inter",
      "family": "Inter",                  // CSS family + display name
      "category": "sans",
      "tags": ["clean", "modern"],
      "aliases": ["helsinki", "helvetica", "arial"],
      "sample": null,                      // optional per-font override
      "file": {
        "url": "ofl/inter/…ttf",           // relative to pinned base
        "sha256": "<hex>",
        "license_url": "ofl/inter/OFL.txt"
      }
    }
  ],
  "samples": {                             // picker fallback text
    "sans": "GARAGE KEYS", "serif": "Pantry — Flour",
    "mono": "GPIO 14 · 3V3", "slab": "TOOL WALL",
    "display": "DANGER 240V", "script": "Happy Birthday!",
    "rounded": "Snack Box", "condensed": "SPICE RACK №4",
    "stencil": "FRAGILE", "blackletter": "Ye Olde Router",
    "handwriting": "leftovers 7/29", "pixel": "12V DC",
    "default": "THE QUICK BROWN FOX"
  },
  "symbols": [
    // kind "text": autocomplete inserts the string, renders in main font
    { "name": "mm2", "kind": "text", "text": "mm²", "keywords": "…" },
    // kind "icon": inserts the Material Symbols codepoint
    { "name": "plug", "kind": "icon", "cp": 57346, "keywords": "…" }
  ]
}
```

Notes:
- TTF/OTF only (Pillow-safe); browsers accept TTF in `@font-face`.
- Prefer static instances from google/fonts when available; use the
  variable font (default instance) otherwise, and note in the manifest
  entry which it is.
- Icon codepoints come from the published `.codepoints` file in the
  material-design-icons repo — never guessed.

### `src/oh_brother/fontcache.py` — fetch + cache (new module)

- Cache dir via **platformdirs** (new dep): `user_data_dir("oh-brother")/fonts`.
- `path_for(font_id) -> Path | None` — cached path or None, no network.
- `ensure(font_id) -> Path` — download (stdlib `urllib.request`), verify
  SHA-256, atomic tmp+rename, fetch the family's license file alongside
  into `fonts/licenses/`. Raises `FontUnavailable` on network/hash
  failure; **never** leaves a partial file.
- `ensure_all()` — prefetch everything (CLI + deploy tool).
- Icon font: after download, **subset** with fontTools (already a dep) to
  exactly the manifest's icon codepoints; cache file is keyed by a hash of
  the codepoint set so manifest edits regenerate it. The subset is what
  both Pillow and the browser get (~tens of KB).
- Emergency fallback when offline and uncached:
  `_SYSTEM_FALLBACKS` probe list covering macOS
  (`/System/Library/Fonts/Helvetica.ttc`), Linux (DejaVu/Liberation under
  `/usr/share/fonts`), Windows (`C:\Windows\Fonts\arial.ttf`). Policy: a
  print falls back loudly (warning to stderr / toast) but proceeds; it
  never fails solely because a font couldn't be fetched.

### render.py rewire

- `resolve_font(spec)`: manifest id → `fontcache.ensure()`; alias →
  its target id; existing path/`.ttf`/`.ttc` behavior unchanged; host
  family name → resolved via the host-font scan (below). On
  `FontUnavailable`, fall back (cached any-font → system probe list) with
  a warning.
- `FALLBACK_FONTS` becomes dynamic: `[icon subset, noto-symbols]` from
  cache, each silently skipped if not cached (no network I/O mid-render).
- **Delete**: `_BROTHER_ALIASES`, `_BROTHER_FONT_NAMES`,
  `DISCOVERED_BROTHER`, `_discover_brother_fonts`,
  `_register_brother_fonts`, `adapted_font_bytes`,
  `_BROTHER_SYMBOL_PAGES`, `_brother_page_font`, the `0xF000`
  Wingdings branch in `_split_runs`, and the symbol-cmap merge branch in
  `_codepoints`. Symbols are now ordinary characters caught by ordinary
  coverage fallback.
- New `hostfonts.py` (or a section of fontcache): scan platform font dirs
  (macOS: `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`;
  Linux: `/usr/share/fonts`, `~/.local/share/fonts`, `~/.fonts`;
  Windows: `C:\Windows\Fonts`), read family names via fontTools `name`
  table, memoize. Powers both `--font <family>` and the picker's host
  section.

### web.py

- Replace `/fonts/<int:page>.ttf` with `/fonts/<id>.ttf`: look up id in
  manifest, `fontcache.ensure()` (server-side fetch-on-demand is allowed
  here), serve the cached file. 404 unknown id, 503 + JSON error when
  offline-and-uncached. The localhost-only EULA warning is gone — these
  files are freely redistributable.
- `/api/meta`: fonts now come straight from the manifest —
  `{id, family, category, tags, aliases, cached}` — plus
  `host_fonts: [{family, path}]` from the scanner.
- Print/render endpoints unchanged (they already pass `font` through to
  `resolve_font`).

### labelcore.js + index.html

- **Delete** `BROTHER_FAMILIES`, `brotherChar`, `applyBrotherRuns`, the
  PUA constants, and `BROTHER_SYMBOLS` as a separate table.
- Symbol catalog now loads from `fonts.json` (fetched at boot alongside
  the meta probe; in standalone it's a plain static fetch). Autocomplete
  inserts `text` entries as their string and `icon` entries as the icon
  codepoint character. The editor textarea and autocomplete menu add the
  icon family to their `font-family` stack so inserted icons are visible
  while editing.
- Canvas render: one `@font-face` per font id pointing at `fonts/<id>.ttf`
  (same relative URL works in server mode — Flask route — and standalone —
  deploy-time files). Icon + Noto fallback appended to every run's family
  stack; the old `weight` plumbing (`CSS_WEIGHTS`) dies because each file
  is its own family. Markdown `**bold**` keeps canvas synthetic bold.
- `STANDALONE_FONTS` (generic CSS stacks) is deleted — standalone now has
  the real set.

### Old-label compatibility

The PUA page convention (U+E100–E6FF) is **retired without a shim**.
Material Symbols codepoints occupy the same PUA neighborhood, so a
translation layer would collide; and per decision, glyph fidelity is
waived. `:name`s are stable, so anything saved as `:name` text re-expands
correctly; raw PUA characters saved from the old UI render as `?` (one
line in the release notes / commit message).

### Picker UI (index.html)

Replace the `<select>` with a button opening a panel:

- **Tiles = miniature labels**: each font renders the current label text
  (first line, markdown stripped) on a small tape-strip canvas via
  labelcore; empty label → `samples[category]` → `samples.default`.
  Symbols get a tile row showing a handful of icon glyphs.
- **Interactions**: category filter chips; search over family + tags +
  aliases; shuffle button; recents + favorites in localStorage;
  arrow-key/hover scrub live-applies to the main preview, Enter/click
  commits, Esc reverts.
- **Fetch states**: tile renders with `document.fonts.load()`; while a
  server-side fetch is in flight show a spinner on the tile; on 503 show
  an "offline — using fallback" badge (reuse the offline-chip pattern).
- **Host fonts section**: server mode — populated from `meta.host_fonts`,
  selected value posts the font *path* (already supported by
  `resolve_font`); standalone mode — a "Show my computer's fonts" button
  gates `queryLocalFonts()` (Chromium-only, permission-prompted; same
  browser constraint WebUSB already imposes). Canvas uses the local
  family name directly.
- Selected font travels to the server as the manifest **id** (or host
  path); `currentFont()` and the grid-strip controls keep working off the
  same `state.fonts` shape.

### CLI

- `--font` accepts: manifest id, alias (Brother or legacy macOS name),
  host family name, or file path.
- `--fonts`: list the catalog (id, category, tags, alias, cached?) and
  detected host families.
- `--fetch-fonts`: `ensure_all()` for offline prep.
- Stretch (skippable): `--specimen` renders a PNG sheet of every cached
  font via the preview path.
- Portability nit while here: `--preview` uses `open` (macOS-only);
  switch to `webbrowser.open(file://…)` or platform dispatch.

### Static deploy

New `tools/deploy_static.py <outdir>`: copies `static/*`, runs
`ensure_all()` into `<outdir>/fonts/` (including `licenses/` and the icon
subset), done. The output directory is fully self-contained — no CDN, no
Python — and everything in it is redistributable.

## Work breakdown (phases = commits)

1. **Manifest + fetcher.** Write `fonts.json` (fonts, aliases, samples;
   symbols section stubbed), `fontcache.py`, add `platformdirs`,
   CLI `--fonts` / `--fetch-fonts`. *Pin step:* choose current
   `google/fonts` and `material-design-icons` commits, record per-file
   sha256 by downloading each once and hashing; verify each family's
   static-vs-variable availability while doing so.
2. **render.py rewire.** Resolution via manifest + cache, dynamic
   fallbacks, host-font scanner, delete all Brother machinery. `label
   --preview` works with zero cache (system fallback + warning), with
   cache, and with `--font <hostfamily>`.
3. **Symbol re-key.** Fill `symbols` in fonts.json: map each existing
   `:name` (the ~80 Unicode `SYMBOLS` + ~140 Brother names) to `text` or
   `icon` entries; icon codepoints from the pinned `.codepoints` file;
   add the subset step to fontcache. Keep every existing `:name` working —
   a name may not silently disappear; if an icon has no good Material
   match, map to the closest and note it in the commit message.
4. **web.py + labelcore.js.** New `/fonts/<id>.ttf`, manifest-driven
   `/api/meta` + host fonts; labelcore loads fonts.json, PUA machinery
   deleted, @font-face wiring, editor icon-font stack.
5. **Picker UI.** Panel, tiles, samples, scrub/commit/revert, search +
   chips + shuffle + recents/favorites, fetch states, host-fonts section
   (both modes).
6. **Static deploy tool.** `tools/deploy_static.py`; delete
   `tools/extract_ptouch_fonts.py`.
7. **Docs + cleanup.** See below. Update the agent memory
   (`oh-brother-eula-publication-gate`) to record that the gate is moot.

Each phase leaves the app working; phases 2–4 must land before the old
`/fonts/<page>` route is deleted or the web UI breaks against a stale
server (server and page ship together, so within-repo atomicity per
phase is sufficient).

## Docs updates (phase 7, but drafted alongside code)

**README.md**
- Delete: the entire "Reinstalling from scratch (fonts and symbols)"
  section, the P-touch installer sha/steps, and the NAS safekeeping
  paragraph (machine-specific; must not survive into a public repo).
- Rewrite the intro line "keeper of the macOS font mappings" → the server
  is now just the default transport + USB owner.
- New "Fonts" section: curated OFL/Apache set fetched on first use from
  pinned URLs; cache location per-OS (platformdirs path); `label
  --fetch-fonts` for offline prep; offline behavior (loud fallback, never
  a failed print); host fonts usable by family name; licenses cached
  alongside the files.
- Web app section: describe the picker in one or two lines; note
  standalone hosting now includes real fonts via `tools/deploy_static.py`.

**AGENTS.md**
- Delete the "Brother EULA constraints (binding)" section entirely —
  no Brother assets remain in any form. Replace with a short "Font
  licensing" note: manifest-pinned OFL/Apache fonts, license files must
  be fetched and served alongside, never add a font without a
  redistributable license.
- Layout section: update `render.py` description (drop symbol-cmap /
  lookalike language), add `fontcache.py`, `static/fonts.json`,
  `tools/deploy_static.py`.
- Sync points: replace the Brother note with "fonts.json is the single
  source of truth for fonts, aliases, and symbols — render.py and
  labelcore.js must both read it, never fork it".
- Standalone-mode line: "features that need the server (macOS font
  mapping) degrade" → host-font browsing degrades to `queryLocalFonts()`.

**Memory (not in repo):** rewrite
`oh-brother-eula-publication-gate.md` → Brother fonts fully removed on
<date>; no EULA blocker on publication; extractor tool deleted.

## Public-repo / local-setup independence checklist

Verify before calling phase 7 done:

- [x] Greps for Brother font/P-touch references return only protocol and
      product references (printer model, raster docs), no font usage.
- [x] Greps for the owner's homelab hostnames, NAS paths, and absolute
      home-directory paths: no hits outside git history.
- [x] No macOS-only paths outside the labeled `_SYSTEM_FALLBACKS` probe
      list, the host-font scan-dir table (both include Linux/Windows),
      and the inherently macOS-only `macos/` app tooling.
- [x] Fresh-cache test: with an empty cache, the network path fetches
      Inter; the offline path warns and uses the system fallback.
- [x] `tools/deploy_static.py` output is self-contained (page + 24 fonts
      + licenses, ~9 MB), no further fetches at page load.
- [x] Every cached font has its license text in `fonts/licenses/`; the
      deploy tool ships them and refuses to ship a font without one.

## Verification

- Minimal pytest (new `tests/`): manifest integrity — unique ids/aliases,
  every alias target exists, every symbol name unique and `cp` values
  inside the icon subset, sha256 fields well-formed. No network in tests.
- Manual: `--preview` renders per phase gate above; web UI in server and
  standalone modes (standalone via `python -m http.server` on a deploy
  dir + WebUSB); one real print at the end — keep it short, tape is
  physical.

## To resolve at implementation time (not guessable)

- Exact pinned commit SHAs and per-file sha256 values.
- Static-instance availability per family in google/fonts (fall back to
  variable file + default-instance render check in Pillow).
- Exact path of the Material Symbols variable font + `.codepoints` file
  inside material-design-icons.
- `queryLocalFonts()` exact API shape (verify against MDN when writing
  the picker's host-font section).
