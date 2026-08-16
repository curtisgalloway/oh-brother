<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
-->

# oh-brother

Print labels on a Brother PT-H500 (USB), PT-18R (USB), or PT-P300BT
P-touch Cube (Bluetooth) from the command line, without the Brother app. Talks the
documented Brother raster protocol directly and renders labels
auto-sized to whatever TZe tape is loaded.

The tools are two self-contained Rust binaries — `label` (CLI) and
`label-web` (the web UI server) — with no runtime dependencies:

```sh
cd rust && cargo build --release
./target/release/label --status
```

On a Mac, the easier route is the [Mac app](#mac-app), which bundles
both binaries and can install `label` into your PATH from its menu.

## Usage

```sh
label "GARAGE KEYS"                 # print, text fills the tape height
label "line one\nline two"          # multi-line
label --font jost --size 40 "12V"   # font id, alias, family name, or path
label --qr https://example.com "wiki"   # QR code + caption
label --image logo.png              # print an image file
label --preview "test"              # render to a PNG and open it, no printing
label "fuses :lightning 5A"         # :symbol names work inline
label --status                      # what tape is loaded?
label --fonts                       # list the font catalog
label --fetch-fonts                 # prefetch everything for offline use
label --copies 3 --chain "SPICE"    # 3 labels, no feed between them
label --printers                    # list printers; --printer ID picks one
label --skill                       # usage guide for AI agents
```

With both printers reachable, the default is USB first; `--printer`
overrides, and the web UI grows a printer picker next to the status
chip whenever there is more than one to choose from.

`--skill` prints a self-contained guide (workflow, examples, printer
gotchas) so an AI agent that finds `label` on PATH can teach itself to
use it.

## Fonts

Every font is open-licensed (OFL or Apache 2.0) and **downloaded on first
use** — nothing is vendored in the repo. `static/fonts.json`
pins each file to a specific commit of the public `google/fonts` and
`material-design-icons` repos with a SHA-256; the fetcher verifies the hash
and caches files in the per-user data directory (macOS:
`~/Library/Application Support/oh-brother/fonts`, Linux:
`~/.local/share/oh-brother/fonts`), with each family's license text
alongside.

The curated set covers a wide range: grotesque/geometric/legible sans
(Inter, Jost, Barlow, Atkinson Hyperlegible), serifs (Source Serif 4, Libre
Baskerville, Bodoni Moda), slab (Roboto Slab), mono (IBM Plex Mono), display
(Archivo Black, Bebas Neue), condensed (Archivo Narrow), rounded (Varela
Round), scripts (Great Vibes, Pacifico), handwriting (Caveat), stencil
(Saira Stencil One), pixel (VT323), and blackletter (Grenze Gotisch).

`--font` accepts:

- a manifest id (`inter`, `bebas-neue`, …),
- an alias — the PT-H500 device-font names (`helsinki`, `brussels`,
  `letter-gothic`, …) and older shortcuts (`menlo`, `futura`, `impact`, …)
  map to the closest open face,
- the family name of any font installed on this machine (`--font "Comic
  Sans MS"`), or
- a path to a `.ttf`/`.otf`/`.ttc` file (`#N` selects a face in a
  collection).

Offline with an empty cache, printing falls back loudly to a system font
rather than failing. The ~225 `:symbol` names (type `:` in the web editor,
or inline in CLI text) render from a Material Symbols subset plus
monochrome Noto symbol/emoji fonts; unit symbols like `:mm2` expand to real
text (`mm²`) in whatever font the label uses.

## Web app

`label-web` serves a live-preview label editor at
http://127.0.0.1:8763/ — multi-line editor, `:symbol` autocomplete, a font
picker where every font previews as a miniature label of your current text
(searchable by name/style/vibe, category chips, favorites, shuffle; arrow
keys try fonts live against the preview), fonts installed on the computer,
size/width controls, `code:`/`qr:` directives, tape auto-detection, and an
optional markdown mode (`**bold**`, `*italic*`, `` `mono` ``,
`~~strike~~`, `\*` escapes). Markdown is a web-UI feature only; the CLI
prints text literally.

Grid labels for Gridfinity bins (and anything else linear): first line
`grid:5u/6` makes a strip exactly 5 Gridfinity units (42 mm each) wide,
split into 6 equal cells with hairline dividers; each following line is
one cell's text (blank line = empty cell). `grid:210mm/6` for explicit
widths. Works in the CLI too: `label "grid:5u/6\nM3\nM4\nM5\n\nnuts\nmisc"`.

The same page also runs **without any server**: build a self-contained
bundle with

```sh
label-web --export-static ./site
```

and copy `site/` to any HTTPS static host — it prints over WebUSB directly
(Chromium browsers only; on Windows the printer must be rebound to WinUSB)
and ships the full font set and licenses, no third-party requests at page
load. Labels render in the browser either way; the `label-web` server is
just the default transport and USB owner.

## Mac app

`macos/build.sh install` builds a native shell app into
`/Applications/Oh Brother.app` — a WKWebView window around the web UI
that starts the bundled label server on launch (or attaches to one
already running) and stops what it started on quit. Requires the
Xcode command-line tools and a Rust toolchain. The app is
self-contained: the release
`label-web` and `label` binaries live in `Contents/Resources/bin`, so
the installed app doesn't need the repo. Rebuild after pulling to
pick up any changes.

The **Install 'label' Command in PATH…** menu item symlinks the
bundled CLI into `~/.local/bin` so `label` works from any shell (and
any AI agent).

## Windows app

*Written but not yet exercised on a real Windows machine — expect
first-run fixes.*

`windows\build.ps1 install` builds a clickable **Oh Brother.exe** — a
Rust WebView2 shell (`rust/label-app`) with `label-web.exe` and
`label.exe` bundled alongside — installs it under
`%LOCALAPPDATA%\Programs\Oh Brother`, and adds a Start Menu shortcut:
the Windows counterpart of `macos/build.sh install`. Requires only a
Rust toolchain (MSVC) on PATH. Like the Mac app, it's a thin shell
that starts the bundled `label-web.exe` (or attaches to one already
running). The **Tools** menu installs a `label.cmd` shim on the user
PATH, the counterpart of the Mac app's symlink installer.
`windows/TESTING.md` is the verification checklist for the first
build on real Windows.

Printers on Windows: the PT-H500 and PT-18R over USB need their
interface bound to WinUSB with [Zadig](https://zadig.akeo.ie/) — the
same rebind the WebUSB static deploy needs. The PT-P300BT Cube is not
reachable from Windows yet (the Rust Bluetooth transport is
macOS-only for now).

## Setup

```sh
cd rust && cargo build --release
./target/release/label --status
./target/release/label --fetch-fonts   # optional: warm the font cache for offline use
```

**PT-H500 (USB):** just plug it in — the binaries link libusb
statically, no system install needed.

**PT-P300BT Cube (Bluetooth):** pair it once (System Settings ▸
Bluetooth on macOS; `bluetoothctl` + `rfcomm bind` on Linux) and it just
works — `label` auto-discovers it when no USB printer is present. On
macOS the connection goes through IOBluetooth RFCOMM directly (the
Bluetooth serial `/dev/cu.*` devices no longer function on current
macOS). Cube notes: tops out at 12 mm tape (64 px printable), no
auto-cutter, and the mechanics waste ~25 mm of tape before the printed
area on every label — chain copies (`--copies N`) to amortize it. The
Cube powers itself off when idle; press its power button if `label`
reports it unreachable. Printing over WebUSB from the static deploy
does not apply to the Cube (it has no USB port).

## Development

```sh
cd rust && cargo test               # unit + golden tests
node tests/labelcore_test.js        # browser-renderer logic tests
```

The project began life in Python; the Rust port was verified against
that implementation (byte-identical protocol golden tests, a full
render-parity sweep against Pillow) before the Python side was
removed in August 2026 — it lives on in git history.

## License

Apache 2.0 — see [LICENSE](LICENSE).
