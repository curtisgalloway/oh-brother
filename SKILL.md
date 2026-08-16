---
name: label
description: Print physical tape labels on a Brother P-touch printer (PT-H500 or PT-18R over USB, or PT-P300BT Cube over Bluetooth) with the `label` CLI. Use when the user wants to print, make, or design a physical label — for jars, cables, bins, switches, anything. Auto-sizes to the loaded TZe tape.
---

<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Printing labels with `label`

`label` prints on a Brother P-touch label printer, auto-sized to
whatever TZe tape is loaded. This text is available from the tool
itself: `label --skill`.

## Workflow

1. `label --status` — confirms a printer is reachable and shows the
   tape (e.g. `PT-P300BT: 12 mm laminated (TZe), 64 px printable`).
2. `label --preview "text"` — renders to a PNG and opens it, printing
   nothing. Iterate here; printing consumes non-refundable tape.
3. `label "text"` — prints.

## Commands

```sh
label "GARAGE KEYS"                # text fills the tape height
label "line one\nline two"         # \n starts a new line
label --font jost --size 40 "12V"  # font id, alias, family name, or path
label --qr https://example.com "wiki"   # QR code + caption
label --code 4048999 "SKU"         # Code 128 barcode + caption
label --image logo.png             # image file instead of text
label "fuses :lightning 5A"        # :symbol names render inline
label --copies 3 --chain "SPICE"   # 3 copies, no feed between them
label --fonts                      # list the font catalog
label --width 150 "WIDE"           # horizontal stretch in percent
label --printers                   # list connected/paired printers
label --printer PT-P300BT2521 "X"  # print to a specific printer
```

- `:symbol` names (about 225: `:warning`, `:arrow-right`, `:mm2`, …)
  render as icons or expand to text like `mm²`. Unknown names print
  literally; check spelling against the web editor's autocomplete.
- Grid strips (e.g. for Gridfinity bins): first line `grid:5u/6` makes
  a 5-unit-wide strip split into 6 cells, following lines fill cells:
  `label "grid:5u/6\nM3\nM4\nM5\n\nnuts\nmisc"`.

## Printer facts that matter

- Tape is physical and finite. Preview first, keep labels short, and
  never print in a retry loop — if a print errors, diagnose before
  reprinting.
- The PT-P300BT Cube (Bluetooth) powers itself off when idle. If
  `label` reports it unreachable, ask the user to press its power
  button, then retry. Its mechanics also waste ~25 mm of blank lead
  tape per label (head-to-cutter distance — not fixable in software).
  By default the tape feeds the same amount after the label so the
  margins come out equal; `--save-tape` feeds only ~2 mm instead (the
  user trims the lead with scissors), and batching with
  `--copies N` / `--chain` amortizes the lead across labels.
- Tape width is auto-detected; never assume it. The Cube maxes out at
  12 mm tape, the PT-18R at 18 mm, the PT-H500 at 24 mm.
- With several printers available, the default is the USB printer,
  then the first Bluetooth Cube; `label --printers` lists them and
  `--printer ID` picks one.
- Only one process can talk to the printer at a time; if the label-web
  server is running, prefer its HTTP API (below) over the CLI.

## HTTP API (alternative)

If the label server is up (`label-web`, port 8763 — the "Oh Brother"
Mac app runs one), you can print without the CLI:

```sh
curl -s http://127.0.0.1:8763/api/status
curl -s -X POST http://127.0.0.1:8763/api/print \
  -H 'Content-Type: application/json' \
  -d '{"text": "GARAGE KEYS", "copies": 1}'
```

## Troubleshooting

- "no printer found" — USB printer unplugged/off, or the Cube isn't
  paired (System Settings ▸ Bluetooth) or auto-powered off.
- "timed out waiting for a status reply" — usually a stale Bluetooth
  session; wait a couple of seconds and retry once, then ask the user
  to power-cycle the printer.
- Printer errors ("end of tape", "cover open", …) are reported in the
  command output verbatim; relay them to the user.
