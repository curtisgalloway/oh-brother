<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# oh-brother

CLI (`label`) for printing labels on a Brother PT-H500 (USB), PT-18R
(USB), or PT-P300BT P-touch Cube (Bluetooth), replacing the Brother
P-touch Editor app.

## Architecture: one renderer, two transports

The web UI renders labels **in the browser** (`static/labelcore.js`, canvas)
and prints through a pluggable transport:

- **Server mode (default)**: the page is served by `label-web` and POSTs
  its rendered bitmap to `/api/print-raw`; the server owns USB. Works in
  any browser.
- **Standalone mode**: the same page served from any static HTTPS host (no
  server at all) claims the printer via WebUSB. Chromium-only; Windows
  needs a WinUSB driver swap. Server-only features degrade gracefully:
  host-installed fonts come from `queryLocalFonts()` (permission-gated)
  instead of the server's scan — accepted policy, not a bug.

The page probes `/api/meta` at boot to pick its mode. Deploying standalone =
`label-web --export-static OUTDIR`, which writes the embedded page and
fetches the full font set + licenses into `OUTDIR/fonts/`.

## Layout

- `rust/` — the implementation, a cargo workspace (see "Rust
  workspace" below for the per-crate map). Protocol references:
  `docs/pt-raster-h500-spec.md` for the PT-H500 family; for the Cube,
  the "PT-E550W/P750W/P710BT" reference, verified byte-by-byte on
  hardware; the PT-18R is empirical (AGENTS "Hardware notes").
- `static/fonts.json` — THE font/symbol manifest: pinned sources +
  hashes, categories/tags/aliases, picker sample strings, and the
  225-entry `:symbol` catalog. Served to the browser as-is; embedded
  into the binaries at compile time.
- `static/labelcore.js` — canvas renderer (text/QR/Code 128,
  directives), raster packing, protocol bytes, and both transports; the
  symbol catalog and fallback families are injected from fonts.json via
  `LabelCore.configure()`. Vanilla JS, no build step — keep it that
  way. Unit-tested by `tests/labelcore_test.js` (run
  `node tests/labelcore_test.js`).
- `SKILL.md` — the agent-facing usage guide embedded into the `label`
  binary (`label --skill`). Keep it in sync when CLI flags, directive
  syntax, or printer behavior change; keep it compact — it loads into
  agent context.
- `macos/` — the "Oh Brother.app" native shell: a WKWebView window that
  spawns the bundled Rust `label-web` on launch (attaches instead if one
  is already running; only terminates what it spawned). `build.sh
  [install]` runs `cargo build --release`, copies the `label-web` and
  `label` binaries into `Contents/Resources/bin`, compiles the shell
  with swiftc, and signs the nested binaries before sealing the app
  (unsigned nested binaries get killed on launch). The app is
  self-contained — no repo/uv paths baked in. The "Install 'label'
  Command in PATH…" menu item symlinks the bundled `label` binary into
  `~/.local/bin`.
- `rust/label-app` + `windows/` — the Windows shell (tao/wry WebView2,
  Rust), same thin-shell architecture as macos/: spawn the
  `label-web.exe` sitting next to the shell exe or attach, terminate
  only what it spawned, Tools menu installs a `label.cmd` PATH shim
  that runs the bundled `label.exe`. Also compiles and runs on
  macOS/Linux for development (the Mac app proper is macos/).
  `windows/build.ps1 [install]` runs `cargo build --release`,
  assembles "Oh Brother.exe" (label-app renamed) + `label-web.exe` +
  `label.exe` into one folder, and creates a Start Menu shortcut. No
  Python/uv anywhere in the Windows build. Partially verified on real
  Windows (brik, 2026-08-15): workspace build, CLI + font cache,
  server, and the build.ps1 assembly all pass; the GUI shell and
  printing await desktop/hardware time — `windows/TESTING.md` tracks
  the remainder (known limitation: the Cube's Bluetooth transport is
  macOS-only in the Rust port, so Windows is USB printers only for
  now).
- `windows/installer/` + `windows/INSTALLER-PLAN.md` — the Windows
  installer: WiX v7 authoring for a per-user MSI (`Package.wxs`) and a
  Burn bundle that chains the WebView2 runtime (`Bundle.wxs`), built by
  `windows\build.ps1 msi|bundle`; INSTALLER-PLAN.md is the research and
  decision record (format choice, Azure Artifact Signing, winget updates) and
  ends with the one-time setup checklist. The release pipelines live at
  `.github/workflows/release-windows.yml` (MSI + bundle + winget; needs
  the WiX v7 OSMF EULA acceptance — `wix eula accept wix7`, error
  WIX7015 otherwise) and `.github/workflows/release-macos.yml` (arm64
  DMG via `macos/build.sh dmg`; ad-hoc signed on runners until
  notarization secrets exist — downloaders right-click-Open past
  Gatekeeper). The UpgradeCode GUIDs in both .wxs files are permanent
  identity — never regenerate them.
- `macos/AppIcon.icns` + `windows/AppIcon.ico` — the app icon,
  committed as built artifacts (the Pillow generator that drew them
  was retired with the Python side; regenerate from git history's
  `tools/gen_icon.py` if the icon ever needs to change).
- `docs/pt-raster-h500-spec.md` — verified clean-room re-expression of
  Brother's Raster Command Reference (PT-H500/P700/E500 v1.11): full
  command set, 32-byte status layout, margin/feed geometry, and the
  prescribed print sequence, every fact provenance-tagged. Cite THIS
  document for protocol work — do not paste Brother PDF text into the
  repo and do not consult GPL driver sources for protocol facts; file
  gaps in `docs/spec-gaps/pt-raster.md`. Verification record in
  `docs/provenance-ledger.md`.

## Rust workspace

`rust/` is the implementation — the project began as Python, was
ported crate by crate with the Python side as a hardware-verified
oracle, and the Python package was deleted on 2026-08-15 once the
port was complete (git history has it; the golden tests and the
render.py-derived layout math carry its verified behavior forward).
Protocol facts cite `docs/pt-raster-h500-spec.md`, never a driver
source. The Mac shell is hardware-verified; the Windows shell awaits
its first real-Windows build (`windows/TESTING.md`).

- `rust/pt-protocol/` — the raster protocol + transports. USB is rusb
  with vendored (statically linked) libusb, so no Homebrew dependency.
  The macOS Bluetooth transport is a **Swift C-ABI shim**
  (`swift/ptbt.swift`, compiled by build.rs) — per user rule, always
  prefer a Swift shim over objc2 bindings for macOS frameworks. All
  the hardware-learned Cube behavior lives there: main-thread-only,
  reopen gap, stale-link retry, and confirm-print in printer logic.
- `rust/label-render/` — the render.py port: text (auto-fit,
  multi-line, per-char fallback via cmap), `:symbol` expansion, QR
  (`qrcode` crate — module counts golden-tested against Python
  qrcode), Code 128 (ported from python-barcode's tables; golden
  bit-patterns in `code128.rs`), `qr:`/`code:`/`grid:` directives,
  font resolution (id → alias → host family → path, loud offline
  fallback), plus `fontcache.rs` (the full download-on-demand cache:
  commit-pinned URLs, SHA-256 verify, atomic writes, license files;
  cache names keep the retired Python side's `_cache_name` scheme so
  existing user caches stay valid; `OH_BROTHER_FONT_OFFLINE=1`
  disables the network; release-archive pins fetch a sha256-verified
  zip and extract a member — how inter-bold gets its official
  prebuilt static file), `transform.rs` (icon subsetting via skera,
  Google Fonts' pure-Rust subsetter — no C, no bindgen, no libclang),
  and `hostfonts.rs` (family scan following fontTools
  getBestFamilyName's record priority). Layout math preserves
  render.py's numbers (including Python's round-half-even); pixels
  come from ab_glyph (no shaping/kerning/hinting).
- `rust/label-cli/` — the `label` binary:
  TEXT/`--qr`/`--code`/`--image`, `--font`, `--size`,
  `--margin`, `--width`, `--preview`, `--tape-mm`, `--copies`,
  `--chain`, `--save-tape`, `--printers`, `--printer`, `--status`,
  `--skill` (embedded SKILL.md), `--fonts`, `--fetch-fonts`.
- `rust/label-server/` — the `label-web` binary: axum, embeds the
  static/ UI at compile time (single shippable binary), printer jobs
  on the main thread via a channel (macOS Bluetooth constraint).
  `/api/render` + `/api/print` render via label-render; `/api/meta`
  includes the host-font scan; `/fonts/<id>.ttf` fetches on demand
  (503 only when the network is down and the font is uncached);
  `--export-static DIR` writes the standalone WebUSB deploy.
- `cargo fmt` + `cargo clippy --all-targets` before committing; run
  from `rust/`.

Known, accepted divergences from the retired Python implementation
(kept as the behavior contract's history; the Python side and its
parity harness live in git history):

- Text rasterization: ab_glyph doesn't shape, kern, or hint, so text
  runs measure/render a pixel or two different from Pillow
  (FreeType+Raqm) and small glyphs are a shade lighter. Structural
  layout — including auto-fit font sizes, whose metrics replicate
  FreeType's 26.6 fixed-point pipeline exactly (`font.rs metrics`) —
  was verified identical against Pillow for every cached font at
  every probeable size before the Python side was removed.
- Absurd inputs fail politely instead of mirroring CPython's memory
  behavior: labels/grids wider than `MAX_RENDER_PX`, huge grid cell
  counts, and out-of-range `size`/`margin`/`tape_px`/`copies` values
  return a 400/error (Python variously MemoryErrors into a 500,
  renders a cropped label for negative margins, or 500s on uncaught
  int() failures). Same intent, safer failure mode — Rust's allocator
  aborts the process where CPython raises, so the bound is
  load-bearing, and error TEXTS for bad values differ from CPython's.
- A `#` face-index suffix that isn't a number errors on both sides,
  but with different messages and timing (Rust at spec parse, Python
  from int() wherever the spec is first touched).
- Code 128: python-barcode's start-fold optimization deletes a
  leading "99" digit pair (it mistakes the 99 pair for a TO_C switch
  code — "9912" encodes as "12"). The Rust port keeps the pair
  (`code128.rs` explains why the guarded fold is provably safe);
  labelcore.js is immune since it always opens in Code B.
- Host fonts: Apple's bitmap-only fonts (`bhed` table, e.g. "GB18030
  Bitmap") don't parse with ttf-parser and are absent from the Rust
  scan; ab_glyph couldn't render them anyway. Name decoding is also
  narrower than fontTools: Mac-platform records decode only as ASCII
  and legacy Windows CJK codepages (ShiftJIS/PRC/Big5/Wansung) are
  skipped, so a font whose family name exists ONLY in such records is
  missing from the scan — in practice those fonts carry a Windows
  Unicode record too.
- Cache transforms: the pipeline has been through three subsetters —
  fontTools (Python), hb-subset, and now skera — with outputs that
  differ byte-wise but are equivalent (verified at each hand-off:
  identical icon cmaps, bold renders bold). skera keeps the icon
  font's variation tables where its predecessors pinned axes to
  defaults; every consumer renders the default instance, so cached
  files from any era satisfy the same cache name.

## Font licensing (binding)

The app contains NO Brother assets. Every font comes from fonts.json:
OFL or Apache 2.0 only, pinned to a public repo commit with a SHA-256.
When adding a font: it must carry a redistribution-safe license, its
`license_path` must point at the license text, and anything that
re-serves or re-ships font files (the `/fonts/` endpoint, the static
deploy) must keep the license texts alongside. Never vendor font
binaries into the repo — the manifest + fetcher is the only channel.

## Sync points (drift here breaks user expectations)

- The `qr:`/`code:` directive syntax and layout rules exist in TWO
  places: `labelcore.js` (`renderLabel`) and
  `rust/label-render/src/render.rs` — change both together.
- `static/fonts.json` is the single source of truth for fonts, aliases,
  samples, and `:symbol`s — labelcore.js and the Rust crates (which
  embed it at compile time) all read it; never fork the catalog into
  code. The fallback-font id order is mirrored in index.html's
  `FALLBACK_FONT_IDS` and `render.rs`'s `fallback_font_paths`.
- The Code 128 encoders in labelcore.js and
  `rust/label-render/src/code128.rs` derive from python-barcode's
  tables, with golden tests on the Rust side and
  `tests/labelcore_test.js` on the JS side. Known, accepted
  divergences: the Rust encoder opens in Code C for digit-pair data
  where labelcore always uses Code B — both scan identically; and
  both are immune to python-barcode's leading-"99" fold bug (see the
  divergence list above).
- The raster protocol bytes in labelcore.js mirror `pt-protocol` —
  change both together, citing `docs/pt-raster-h500-spec.md`.

## Conventions

- `cargo fmt` + `cargo clippy --all-targets` clean before each commit
  (run from `rust/`); `cargo test` and `node tests/labelcore_test.js`
  stay green.
- Apache 2.0 headers on every source file, including Markdown instruction files.
- Hardware facts (endpoints, command bytes, tape-width table) must be
  verified against `docs/pt-raster-h500-spec.md` / the manufacturer
  references it cites, or on hardware — never guessed, and never taken
  from GPL driver sources (file gaps in `docs/spec-gaps/`).

## Hardware notes

- PT-H500 is USB ID `04f9:205e`, bulk OUT `0x02`, bulk IN `0x81`, 128-px head,
  180 dpi, TZe tape up to 24 mm.
- PT-P300BT (P-touch Cube) is Bluetooth-only: SPP on RFCOMM channel 1,
  same 16-byte raster lines at 180 dpi but only the middle 64 dots are
  printable (12 mm tape max), requires ESC i a + ESC i z before raster
  data, has no auto-cutter (manual lever only), and mechanically
  wastes ~25 mm of lead tape per label (head-to-cutter distance; the
  PT-H500's is 24.5 mm per its raster reference). ESC i d is the END
  margin — the feed past the last printed column — hardware-verified
  on BOTH printers (see `docs/pt-raster-h500-spec.md` §6 and
  `docs/spec-gaps/pt-raster.md`). With 0 the cut lands flush on the
  print; the app defaults to feeding the lead amount for equal
  margins, with save_tape (~2 mm) and chain (0) as the tape-saving
  modes on both models.
- PT-18R is USB ID `04f9:201a`, same endpoints as the H500 (bulk OUT
  `0x02`, IN `0x81`), 128-px head at 180 dpi. Brought up empirically
  2026-08-15 (no raster reference exists for it; official spec sheet +
  hardware measurement only — see the PT18R ModelSpec comments): it
  speaks the H500 minimal dialect verbatim, honors ESC i d as the end
  margin (firmware default is ~1 mm, so equal margins REQUIRE it),
  max print height 15.8 mm = 112 px (18 mm tape, spec sheet), tape
  center sits ~4 dots above head center (`tape_center_offset`), lead
  ≈24 mm, auto full cutter that chain (`0x0C`) correctly suppresses.
  Quirk: the first status after another process held the interface can
  report "communication error"; the init in a retry clears it.
- The status reply (ESC i S) reports the loaded tape width; the CLI auto-sizes
  to it. Never assume a tape width in printing code paths.
- Printing consumes physical tape — during development prefer `--preview`, and
  keep test labels short.

## PT-P300BT Bluetooth gotchas (all verified on hardware, macOS 26)

- The paired Cube's `/dev/{cu,tty}.PT-P300BT*` serial devices are dead:
  they open instantly without dialing the link and writes vanish, even
  while the baseband link is up. Drive it via IOBluetooth RFCOMM.
- IOBluetooth RFCOMM only works from the process **main thread**
  (`openRFCOMMChannelSync` returns kIOReturnError elsewhere; the async
  variant's callbacks never fire). `label-web` therefore runs Flask in
  a daemon thread and printer jobs on the main thread — keep it that
  way.
- The Cube **aborts the job with an error blink** if the host closes
  the connection before "printing completed" arrives — `print_image`
  blocks on that status (`confirm_print` in the model spec). Do not
  "optimize" the wait away.
- Reopening a session within ~1 s of closing one attaches to the dying
  session and times out; `_RfcommTransport` enforces a reopen gap.
- A stale half-open baseband link makes every open fail until dropped
  (`blueutil --disconnect` clears it manually); the transport closes
  the connection on failed opens and retries once.
