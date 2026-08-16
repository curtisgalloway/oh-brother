<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Windows verification brief

For the agent (or human) verifying oh-brother on a real Windows
machine (the user's box: brik). Read `AGENTS.md` first for repo
conventions; fix-forward is expected — commit fixes with the usual
conventions rather than reporting back and waiting.

**Status 2026-08-15 (verified over SSH on brik):** the workspace
builds clean (after the hb-subset → skera swap; hb-subset 0.3.0 does
not compile on MSVC), `label.exe` fetches all 25 fonts (including the
Inter release-zip pin and the skera icon subset — icons render
correctly in previews), `label-web.exe` serves the UI with a
234-family host-font scan, and `build.ps1` assembles the dist folder.
Remaining items below need someone at the desktop (GUI) or hardware
(printer), or a decision (installing the .NET SDK for WiX — not
present on brik).

## What you are testing

`rust/label-app` is a tao/wry (WebView2) shell around the label-web
UI, replacing the retired Python/pywebview shell. `windows/build.ps1`
builds the whole workspace and assembles `Oh Brother.exe` (label-app
renamed) + `label-web.exe` + `label.exe` into one folder.

## Prerequisites

- Rust toolchain, MSVC flavor (`rustup default stable-msvc`).
- WebView2 runtime — preinstalled on Win10/11; wry needs it at run
  time.
- No uv/Python needed anymore, and no libclang/LLVM either (the
  hb-subset C++ dependency was replaced with the pure-Rust skera).

## Checklist

1. ~~**Workspace builds**~~ VERIFIED 2026-08-15 (cargo build --release
   clean, including the winreg/winresource label-app paths).
2. ~~**CLI works without a printer**~~ VERIFIED 2026-08-15 (--fetch-fonts
   25/25 into `%LOCALAPPDATA%\oh-brother\oh-brother`; previews render
   incl. skera-subset icons).
3. ~~**Server**~~ VERIFIED 2026-08-15 (`/api/meta` 20 fonts + 234 host
   fonts; `/api/status` reports the documented no-printer error). The
   page itself has not been eyeballed in a Windows browser yet.
4. **Shell** (needs the desktop): run `build\Oh Brother\Oh
   Brother.exe` (build.ps1 dist assembly itself is VERIFIED). Splash
   → UI, no console window flashes. Quitting the app kills the
   label-web it spawned (check Task Manager).
5. **Attach semantics**: start `label-web.exe` by hand, then launch
   the app — it must attach; quitting the app must leave the
   hand-started server running.
6. **CLI installer**: Tools → "Install 'label' Command in PATH".
   Verify `%LOCALAPPDATA%\oh-brother\bin\label.cmd` exists, a NEW
   terminal has it on PATH, and `label --help` works. Re-running the
   installer must not duplicate the PATH entry.
7. **Install**: `windows\build.ps1 install` → Start Menu shortcut
   works, app runs from `%LOCALAPPDATA%\Programs\Oh Brother`.
8. **Printing (if hardware present)**: the PT-H500/PT-18R need their
   USB interface bound to WinUSB with Zadig (https://zadig.akeo.ie).
   Then `label.exe --status`, `label.exe --preview` first, and a short
   real print. Tape is physical — keep test labels short.

## Known limitation (do not chase it as a bug)

The PT-P300BT Cube is **not reachable from Windows** in the Rust port:
the Bluetooth transport is macOS-only (`bt_macos.rs`). The retired
Python side used a pyserial Bluetooth COM port; porting that to Rust
(serialport crate + the outgoing-SPP COM discovery) is future work —
file it, don't improvise it mid-verification.

## When done

Update AGENTS.md: mark the Windows shell hardware-verified (or record
exactly what failed), and delete the items you verified from this
file — when everything is verified, delete this file and fold anything
durable into AGENTS.md.
