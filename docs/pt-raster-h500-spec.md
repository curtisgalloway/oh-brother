<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Brother P-touch Raster Protocol — PT-H500/P700/E500 Family Specification

## 1. Usage notice (read this first)

This reference was produced by re-expressing the protocol facts in Brother's
*Raster Command Reference PT-H500/P700/E500 Version 1.11* (© 2014 Brother
Industries, Ltd. — "the databook") in original words and structure, plus
hardware observations recorded by the oh-brother project. The databook's
license permits using its information but forbids copying, adapting, or
redistributing the document itself. Therefore:

- **Do not paste databook text, tables, or diagrams into this repo.** Facts
  (byte values, encodings, ranges, sequences) are fine; the document's own
  prose and layout are not.
- **Cite THIS document** when extending the protocol code, not the PDF.
- **Do not consult GPL driver sources** (ptouch-print, printer-driver-ptouch,
  Ircama's PT-P300BT scripts, or any other third-party driver code) for
  protocol facts.
- If this spec lacks a fact you need: append a line to
  `docs/spec-gaps/pt-raster.md` in the form
  `- [open] <date> <section> <question>`, mark the code site
  `TODO(spec-gap)`, and continue. The gap gets resolved later from the PDF
  or on hardware — never from third-party driver source.

Provenance tags used throughout:

- `[databook §N.N]` / `[databook p.NN]` — stated in the Brother reference.
- `[hardware-verified]` — observed on real hardware by this project
  (recorded in AGENTS.md or protocol.py comments).
- `[unknown — spec-gap candidate]` — not found in either source.

## 2. Scope and identity

This document covers the raster (binary bitmap) printing command language of
the Brother **PT-H500**, **PT-P700**, and **PT-E500** label printers, which
lets a host print without Brother's driver by sending initialization
commands, control codes, and raster data directly [databook, Introduction /
About Raster Commands]. The documented transport is USB 1.1 full speed
(see §10) [databook, Appendix A].

The oh-brother project also drives the **PT-P300BT "P-touch Cube"** with the
same raster language over Bluetooth RFCOMM (SPP, channel 1). The Cube is
documented by a *different* Brother reference (the PT-E550W/P750W/P710BT
family databook), which is **not** a source for this spec. Every
Cube-specific fact in this document is therefore tagged `[hardware-verified]`
only — it carries no databook citation here.

## 3. Canonical references

| Reference | Identity | Where the project keeps it |
|---|---|---|
| Brother databook | "Raster Command Reference PT-H500/P700/E500 Version 1.11", © 2014 Brother Industries, Ltd. | User's Google Drive, file id `1UAp_Efs6NSkBN737CMb4R8nodjWTqq1A`. Must NOT be copied into this repo. |
| Hardware notes | oh-brother `AGENTS.md`, sections "Hardware notes" and "PT-P300BT Bluetooth gotchas" | `/Users/curtisg/src/oh-brother/AGENTS.md` |
| Verified implementation | `src/oh_brother/protocol.py` (Python oracle), `rust/pt-protocol/` (shipping) | this repo |
| Provenance map | source-pin record for this spec | `docs/provenance/pt-raster-h500-map.txt` |

## 4. Print data lifecycle

The databook prescribes this per-job procedure [databook §1]:

1. **Open the port** (USB; port mechanics are out of the databook's scope).
2. **Request and parse status**: send the status information request
   (ESC i S), receive the 32-byte reply, and confirm that compatible media
   is loaded and no error is flagged before sending anything else
   [databook §1, §4 ESC i S].
3. **Send the print data**: initialization commands once per job, then per
   page: control codes, raster data, and a print command (see §8 for the
   full ordered sequence) [databook §2.1].
4. **The printer prints.** Over USB with *uncompressed* raster data the
   printer starts printing as data arrives ("concurrent printing") instead
   of waiting for the print command; otherwise it buffers a full page first
   ("buffered printing") [databook §1 note, §5].
5. **Await completion**: the printer sends status blocks on its own after a
   print command — a phase change to the printing state, then a
   "printing completed" status (status type 01h), then a phase change back
   to the receiving state. One page is finished when "printing completed"
   is confirmed. Repeat steps 2–4 for additional pages [databook §1, §5.1].
6. **Close the port** when the job is done [databook §1].

**No-commands-during-printing rule**: after print data is transmitted, no
command at all — including ESC i S — may be sent until the printer confirms
printing is complete; the printer pushes error status unsolicited during
printing [databook §1 note, §4 ESC i S note].

**Completion status arrives early**: the flow charts show the printer
reporting "printing completed" and the "waiting to receive" phase for page
N *before* page N is mechanically finished, so the host may begin streaming
page N+1 while page N is still printing [databook §5.1].

**Error recovery**: on an error status, the printer clears all received
data; the host restarts with invalidate + initialize + status request and
resends starting from the first page whose "printing" phase change was
never received [databook §5.2, §5.3].

**Cooling**: during long jobs the printer may interleave cooling-started /
cooling-finished notification statuses between "printing" and
"printing completed"; these can repeat several times in one print
[databook §5.8]. The numeric notification codes for cooling are not legible
in the extraction — table (7) as extracted lists only 00h/01h/02h (see §5,
ESC i S, notification byte) [unknown — spec-gap candidate].

The Cube addendum [hardware-verified]: the PT-P300BT aborts the job with an
error blink if the host closes the connection before "printing completed"
arrives, so a Cube driver must block on that status before disconnecting
(AGENTS.md "PT-P300BT Bluetooth gotchas"; `confirm_print` in
protocol.py:120–124).

## 5. Command reference

Complete command list for the family [databook §3]:

| Bytes (hex) | Mnemonic | Purpose |
|---|---|---|
| `00` | NULL | Invalidate |
| `1B 40` | ESC @ | Initialize |
| `1B 69 53` | ESC i S | Status information request |
| `1B 69 61` | ESC i a | Switch dynamic command mode |
| `1B 69 7A` | ESC i z | Print information |
| `1B 69 4D` | ESC i M | Various mode settings |
| `1B 69 4B` | ESC i K | Advanced mode settings |
| `1B 69 64` | ESC i d | Margin amount (feed amount) |
| `4D` | M | Select compression mode |
| `67` | g | Raster graphics transfer |
| `5A` | Z | Zero raster graphics |
| `0C` | FF | Print (non-last page) |
| `1A` | Control-Z | Print with feeding (last page) |

### 5.1 NULL — Invalidate (`00`)

A `00` byte the printer skips. To abort a transmission mid-stream, send
enough invalidate bytes to flush the remainder, then ESC @ to return the
printer to the receiving state with a cleared print buffer
[databook §4 NULL]. Job start convention: a run of **100** invalidate bytes
precedes ESC @ (see §8) [databook §2.1].

### 5.2 ESC @ — Initialize (`1B 40`)

Resets mode settings; also cancels a print in progress
[databook §4 ESC @]. Fixed two-byte form, sent once at job start after the
invalidate run [databook §2.1].

### 5.3 ESC i S — Status information request (`1B 69 53`)

Asks the printer to send its 32-byte status block [databook §4 ESC i S].
Send it once before print data; never during printing (the printer pushes
status unsolicited then) [databook §4 ESC i S note].

The same 32-byte layout is used for solicited replies and unsolicited
(phase/error/completion) pushes; the *status type* byte distinguishes them
[databook §4 ESC i S].

**32-byte status block layout** [databook §4 ESC i S]:

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 1 | Head mark | always 80h |
| 1 | 1 | Size | always 20h (32) |
| 2 | 1 | Brother code | 'B' (42h) |
| 3 | 1 | Series code | '0' (30h) |
| 4 | 1 | Model code | PT-H500: 'd' (64h); PT-E500: 'e' (65h); PT-P700: 'g' (67h) |
| 5 | 1 | Country code | '0' (30h) |
| 6–7 | 2 | Reserved | 00h |
| 8 | 1 | Error information 1 | bitfield, below |
| 9 | 1 | Error information 2 | bitfield, below |
| 10 | 1 | Media width | mm, below |
| 11 | 1 | Media type | below |
| 12 | 1 | Number of colors | 00h |
| 13 | 1 | Fonts | 00h |
| 14 | 1 | Japanese fonts | 00h |
| 15 | 1 | Mode | echoes the ESC i M setting; 00h if unset |
| 16 | 1 | Density | 00h |
| 17 | 1 | Media length | mm; 00h for TZe tape |
| 18 | 1 | Status type | below |
| 19 | 1 | Phase type | below |
| 20 | 1 | Phase number, high byte | below |
| 21 | 1 | Phase number, low byte | below |
| 22 | 1 | Notification number | below |
| 23 | 1 | Expansion area byte count | 00h |
| 24 | 1 | Tape color | below |
| 25 | 1 | Text color | below |
| 26–29 | 4 | Hardware settings | default hardware info used for checking (no further detail legible in extraction) |
| 30–31 | 2 | Reserved | 00h |

**Error information 1** (offset 8) [databook §4, table (1)]:
bit 0 (01h) no media; bit 2 (04h) cutter jam; bit 3 (08h) weak batteries;
bit 6 (40h) high-voltage adapter; bits 1, 4, 5, 7 unused.

**Error information 2** (offset 9) [databook §4, table (2)]:
bit 0 (01h) replace media / wrong media (the databook associates this flag
with a serial connection context — the extraction's phrasing is truncated;
see §9.1); bit 4 (10h) cover open; bit 5 (20h) overheating; bits 1–3, 6, 7
unused.

**Media width** (offset 10) [databook §4, table (3)]: tape width in
integer millimeters, 0 = no tape. TZe widths report 4 (3.5 mm tape reports
itself as **4**), 6, 9, 12, 18, 24. Media length (offset 17) is always 00h
for TZe tape.

**Media type** (offset 11) [databook §4, table (4)]:

| Value | Meaning |
|---|---|
| 00h | no media |
| 01h | laminated tape |
| 03h | non-laminated tape |
| 11h | heat-shrink tube, 2:1 shrink |
| 17h | heat-shrink tube, 3:1 shrink |
| FFh | incompatible tape |

**Status type** (offset 18) [databook §4, table (5)]:

| Value | Meaning |
|---|---|
| 00h | reply to a status request |
| 01h | printing completed |
| 02h | error occurred |
| 03h | exit IF mode (not used) |
| 04h | powered off |
| 05h | notification |
| 06h | phase change |
| 07h–20h | not used; 21h–FFh reserved |

**Phase type / phase number** (offsets 19–21) [databook §4, table (6)]:
phase type 00h = editing state (able to receive), 01h = printing state;
both phase-number bytes are 00h when unused. Within the editing state:
phase 0 (00 00) = receiving-capable, phase 1 (00 01) = feed. Within the
printing state: phase 0 (00 00) = printing, phase 20 (00 14h) = cover open
while receiving; 10 and 25 are listed as unused.

**Notification number** (offset 22) [databook §4, table (7)]:
00h none, 01h cover open, 02h cover closed. Cooling notifications appear in
the flow charts (§4 above) but their numeric codes are not legible in the
extraction [unknown — spec-gap candidate].

**Tape color** (offset 24) [databook §4, table (8)]: 01h white, 02h other,
03h clear, 04h red, 05h blue, 06h yellow, 07h green, 08h black, 09h clear
with white text, 20h matte white, 21h matte clear, 22h matte silver,
23h satin gold, 24h satin silver, 30h blue(D), 31h red(D), 40h fluorescent
orange, 41h fluorescent yellow, 50h berry pink(S), 51h light gray(S),
52h lime green(S), 60h yellow(F), 61h pink(F), 62h blue(F), 70h white
heat-shrink, 90h white flexible-ID, 91h yellow flexible-ID, F0h cleaning,
F1h stencil, FFh incompatible.

**Text color** (offset 25) [databook §4, table labeled (10) in the source;
the layout column calls it (9) — see §9.1]: 01h white, 02h other, 04h red,
05h blue, 08h black, 0Ah gold, 62h blue(F), F0h cleaning, F1h stencil,
FFh incompatible.

`[hardware-verified]` subset: oh-brother validates offset 0 == 80h and
offset 1 == 20h, reads error bytes 8–9, media width at offset 10, media
type at offset 11, and treats status type 01h at offset 18 as print
completion (protocol.py:417–436, 536–559). The status reply's media width
drives automatic tape sizing; printing code must never assume a width
(AGENTS.md "Hardware notes").

### 5.4 ESC i a — Switch dynamic command mode (`1B 69 61 n1`)

One parameter byte selecting the active command interpreter: 0 = ESC/P
(power-on default), 1 = raster, 3 = P-touch Template. The selection sticks
until power-off. Raster data requires raster mode, so send `1B 69 61 01`
before any raster transfer [databook §4 ESC i a].

### 5.5 ESC i z — Print information (`1B 69 7A n1..n10`)

Ten parameter bytes describing the page [databook §4 ESC i z]:

- **n1 — valid-flag bitmask**, declaring which later fields the printer
  should check:
  - 02h — media type (n2) is valid
  - 04h — media width (n3) is valid
  - 08h — media length (n4) is valid
  - 40h — priority to print quality (marked "not used")
  - 80h — printer recovery always on
- **n2 — media type**: same encoding as status offset 11 (00h no tape,
  01h laminated, 03h non-laminated, 11h HS 2:1, 17h HS 3:1,
  FFh incompatible).
- **n3 — media width** in mm (24 mm tape → 18h).
- **n4 — media length** in mm; normally 00h regardless of actual length
  (24 mm example uses n4 = 00h).
- **n5–n8 — raster line count** of the page, unsigned 32-bit
  little-endian: count = n5 + n6·256 + n7·256² + n8·256³.
- **n9 — page position**: 0 for the first page, 1 for every other page.
- **n10** — fixed 0.

If the KIND/WIDTH/LENGTH valid flags are set and the loaded media does not
match, the printer replies with an error status with error-information-2
bit 0 set [databook §4 ESC i z].

Worked encoding from the databook's overview: 100 mm of print on 24 mm tape
at 180 dpi → `1B 69 7A 84 00 18 00 9C 02 00 00 00 00` (n1 = 84h =
width-valid + recovery; n3 = 18h = 24; raster count 029Ch = 668 lines)
[databook §2.1].

`[hardware-verified]` (Cube only): the PT-P300BT requires ESC i z before
raster data; oh-brother sends n1 = C4h (width 04h | quality 40h |
recovery 80h), n2 = 01h, n3 = status-reported width, n4 = 0, n5–n8 =
raster-line count, n9 = 0, n10 = 0 (protocol.py:438–457).

### 5.6 ESC i M — Various mode settings (`1B 69 4D n1`)

One bitfield byte [databook §4 ESC i M]:

- bit 6 (40h): auto-cut — 1 cuts automatically, 0 does not.
- bit 7 (80h): mirror printing — 1 mirrors, 0 normal.
- bits 0–5: unused.

The value chosen here is echoed back in status byte 15 [databook §4
ESC i S]. The margins/feed added at the ends of the printed area are
associated with this mode setting in the raster-line discussion
[databook §2.3.5].

### 5.7 ESC i K — Advanced mode settings (`1B 69 4B n1`)

One bitfield byte [databook §4 ESC i K]:

- bit 3 (08h): **no chain printing** — 1 feeds and cuts after the last
  label of a multi-copy print; 0 (chain printing) suppresses the final
  feed-and-cut.
- bit 4 (10h): **special tape, no cutting** — 1 suppresses cutting when
  special tape is installed.
- bit 7 (80h): **no buffer clearing when printing** — the machine's
  expansion buffer is not cleared. When sent for the first label (between
  ESC @ and the print data), printing proceeds only once a print command
  arrives with the second or a later label. (The extraction of this
  paragraph is awkwardly phrased; the bit's exact operational effect
  should be re-checked against the PDF before relying on it — see §9.1.)
- bits 0–2, 5, 6: unused.

### 5.8 ESC i d — Margin amount / feed amount (`1B 69 64 n1 n2`)

Sets the margin (feed) in dots: **margin = n1 + 256·n2** (16-bit
little-endian) [databook §4 ESC i d]. The databook presents the margin as
the feed applied on continuous tape around the print area relative to the
cut line, and its feed-amount table describes the margin as the tape-axis
("left and right", i.e. leading/trailing along the feed direction) margins
[databook §2.3.3, §4 ESC i d]. Full geometry semantics: §6.

### 5.9 M — Select compression mode (`4D n`)

One parameter byte: 0 = no compression (enabled), 1 = reserved (disabled),
2 = TIFF/PackBits (enabled). Compression applies only to raster-transfer
data [databook §4 M].

**PackBits framing** (mode 2) [databook §4 M]:

- Operates on 1-byte units within one raster line.
- A run of the same byte is encoded as a count byte followed by the one
  repeated byte; the count is (run length − 1) negated (two's complement).
- A stretch of differing bytes is encoded as a count byte (stretch
  length − 1, positive) followed by the literal bytes.
- If compression would exceed **16 bytes** of output on this family, the
  line is emitted as one all-literal stretch instead — 17 bytes total
  including the leading count byte.
- Compression must cover the full line: trailing 00h bytes may not be
  omitted.
- Compressed lines always expand to the full **16-byte / 128-pin** line in
  the printer, regardless of tape width — i.e. under compression the data
  includes the unused-pin regions, whereas the uncompressed layout is
  described in terms of offset pins + print-area pins [databook §4 M
  "Explanation of TIFF compression mode"].

Worked example from the databook [databook §4 M]: an uncompressed line
beginning with twenty 00h bytes, then 22h 22h, then 23h BAh BFh A2h 22h 2Bh
… compresses to `ED 00` (20 repeats → 19 → 13h → negated EDh), `FF 22`
(2 repeats → 1 → negated FFh), `05 23 BA BF A2 22 2B` (6 literals → count
5), continuing likewise.

### 5.10 g — Raster graphics transfer (`67 n1 n2 d1..dk`)

Transfers k = n1 + 256·n2 bytes of raster data for one line
[databook §4 g]:

- The data is expanded into the line buffer starting at the
  margin-adjusted position; a short line is zero-filled to the end of the
  buffer, and excess data past the buffer end is discarded.
- k may range from 0 up to the head pin count divided by 8, rounded up.
- With no compression selected, the data length is fixed at **16 bytes**
  for this family.

Note the command byte: the databook specifies lowercase `g` (**67h**) for
this family [databook §3, §4 g]. oh-brother instead transmits uppercase
`G` (**47h**) followed by a 16-bit LE payload length and a PackBits
payload, and this works on both the PT-H500 and the Cube
`[hardware-verified]` (protocol.py:527–529). A `47h` raster command does
not appear anywhere in this databook — its documentation belongs to the
other-family reference [unknown — spec-gap candidate as far as this
databook is concerned].

### 5.11 Z — Zero raster graphics (`5A`)

Emits one raster line of all zeroes; a single fixed byte
[databook §4 Z]. The overview marks it valid only when TIFF compression is
selected [databook §2.1]. Verified working on the Cube
`[hardware-verified]` (protocol.py:521–526); oh-brother deliberately does
not use it on the PT-H500 path.

### 5.12 FF — Print command (`0C`)

Ends a page that is *not* the last page of the job: print without the
final feed [databook §4 FF].

### 5.13 Control-Z — Print with feeding (`1A`)

Ends the *last* page: print, then feed (and cut, subject to the mode
settings) [databook §4 Control-Z].

## 6. Margins and feed geometry

### 6.1 The margin setting (ESC i d)

Margin in dots = n1 + 256·n2 (§5.8) [databook §4 ESC i d]. At this
family's 180 dpi (1:1 aspect) [databook §2.3.1], 1 mm ≈ 7.087 dots.

Documented margin limits [databook §2.3.3] — note the source table is
partially garbled in extraction; the value pairs below reassemble
unambiguously because each mm figure matches exactly one dot figure:

| Setting | Value |
|---|---|
| Minimum margin | 2 mm = **14 dots** (0.08 in) |
| Minimum margin with no precut ("unrelated to driver") | 24.3 mm = **172 dots** (0.96 in) |
| Maximum margin | 127 mm = **900 dots** (5 in) |

The precise meaning of the middle row ("minimum margin setting with no
precut") is not fully legible in the extraction; treat its interpretation
(beyond the raw 24.3 mm / 172 dots pairing) as unresolved
[unknown — spec-gap candidate].

The databook's own examples: 2 mm margin encodes as `1B 69 64 0E 00`
(14 dots) [databook §2.1]; the driver's test page uses a **15-dot** margin
[databook §2.2.3].

### 6.2 The 24.5 mm mechanical minimum feed

Because of the cutter's position relative to the print head, the shortest
piece of tape the machine can feed out is **24.5 mm**. Any print whose
total data length (margins + print area) is 24.5 mm or less still comes
out on 24.5 mm of tape — e.g. the 4.4 mm minimum print still yields a
24.5 mm label [databook §2.3.4 note]. This is machine geometry, not a
software setting; the driver's minimum print-data length (2 mm margin × 2
+ minimum print area) is derived from it [databook §2.3.4 note].

Cube counterpart `[hardware-verified]`: the PT-P300BT mechanically wastes
about **25 mm** of lead tape per label (head-to-cutter distance), measured
on hardware; oh-brother surfaces it as `lead_margin_mm` (25.0 for the
Cube, 24.5 for the PT-H500 per this databook) (protocol.py:110–116,
127–141; AGENTS.md "Hardware notes").

`[hardware-verified]` (Cube only, no databook citation): on the Cube,
ESC i d behaves as the **end margin** — the feed past the last printed
column. 0 makes the cut land flush against the print; the leading blank is
the fixed mechanical lead and is not software-controllable. oh-brother's
default feeds the lead amount for equal margins, with a ~2 mm save-tape
mode (14 dots) and 0 for chained pages (protocol.py:66–71, 458–476;
AGENTS.md "Hardware notes"). Whether the H500 family applies the ESC i d
amount to one end or both ends of the printed area is not unambiguous in
the extraction (the feed-amount section speaks of margins at both ends)
[unknown — spec-gap candidate; resolve on H500 hardware].

### 6.3 Label length limits

At 180 dpi [databook §2.3.4]:

| Media | Minimum length | Maximum length |
|---|---|---|
| TZe tape | 4.4 mm / 31 dots (0.18 in) | 1000 mm / 7086 dots (39.37 in) |
| Heat-shrink tube | 4.4 mm / 31 dots | 500 mm / 3543 dots (19.69 in) |

### 6.4 Raster line geometry

The head has **128 pins**; one raster line is 128 dots = **16 bytes**
[databook §2.3.5]. A line is transmitted with the leftmost byte first; in
each byte the most significant bit is the earlier pin (MSB-first pixel
order) [databook §2.3.5 diagram]. Print data rows run across the tape and
raster lines advance along the feed direction; lines with any dark pixel
go out as raster-graphics transfers and blank lines as zero-raster lines
(TIFF mode), with the ESC i d/mode-setting margins added at the two ends
of the printed area on tape [databook §2.3.5].

Narrower tapes use a centered window of the 128 pins. Per-tape print-area
pin counts for TZe tape [databook §2.3.5]:

| Tape | Left-margin pins | Print-area pins | Right-margin pins | Bytes/line |
|---|---|---|---|---|
| 3.5 mm | 52 | 24 | 52 | 16 |
| 6 mm | 48 | 32 | 48 | 16 |
| 9 mm | 39 | 50 | 39 | 16 |
| 12 mm | 29 | 70 | 29 | 16 |
| 18 mm | 8 | 112 | 8 | 16 |
| 24 mm | 0 | 128 | 0 | 16 |

Heat-shrink tube [databook §2.3.5]:

| Tube | Left pins | Print-area pins | Right pins |
|---|---|---|---|
| HS 5.8 mm | 50 | 28 | 50 |
| HS 8.8 mm | 40 | 48 | 40 |
| HS 11.7 mm | 31 | 66 | 31 |
| HS 17.7 mm | 11 | 106 | 11 |
| HS 23.6 mm | 0 | 128 | 0 |
| HS 5.2 mm | 54 | 20 | 54 |
| HS 9.0 mm | 42 | 44 | 42 |
| HS 11.2 mm | 39 | 50 | 39 |
| HS 21.0 mm | 4 | 120 | 4 |

The page-size section gives matching physical print-area widths and
offsets per media ID (TZe IDs 257–263, heat-shrink IDs 415–423 where
415–419 are "HS 2:1" and 420–423 are "HS 3:1"), e.g. 24 mm tape: 24.0 mm
overall, 18.1 mm / 128-dot print area, 2.96 mm / 21-dot width offset;
12 mm tape: 11.9 mm overall, 9.90 mm / 70-dot print area, 0.98 mm / 7-dot
offset [databook §2.3.2(a)]. Split ("×N") media IDs 279–293 describe 12,
18 and 24 mm tapes printed as 2–4 parallel bands, with overall print width
= band width × N (+ offsets) [databook §2.3.2(b); the extraction of this
table is interleaved — re-verify any split-media value against the PDF
before use].

Note: oh-brother's PT-H500 printable-height table differs from the
databook's print-area pin counts for the middle widths — see §9.4.

## 7. Prescribed command sequence

The databook's structural template [databook §2.1]: initialization
commands once per job, then per page control codes + raster data + a print
command.

Its test-page walkthrough shows this exact order [databook §2.2.3]:

| # | Step | Notes |
|---|---|---|
| 1 | Invalidate × 100 bytes (`00` × 100) | job start [databook §2.1, §2.2.3] |
| 2 | Initialize `1B 40` | [databook §2.2.3] |
| — per page — | | |
| 3 | Switch dynamic command mode `1B 69 61 01` | required before raster data [databook §2.2.3, §4 ESC i a] |
| 4 | Job-ID setting commands | **internal Brother-driver commands; the databook says users need not send them** [databook §2.2.3] |
| 5 | Print information `1B 69 7A …` | media info for the page [databook §2.2.3] |
| 6 | Various mode `1B 69 4D 00` | nothing set in the walkthrough [databook §2.2.3] |
| 7 | Advanced mode `1B 69 4B …` | walkthrough enables no-chain-printing [databook §2.2.3] |
| 8 | Margin `1B 69 64 …` | walkthrough uses 15 dots [databook §2.2.3] |
| 9 | Compression select `4D 02` | TIFF [databook §2.2.3] |
| 10 | Raster data (g / Z lines) | [databook §2.2.3] |
| 11 | Print `0C` (non-last page) | [databook §2.2.3] |
| 12–19 | Steps 3–10 repeated for the next page | control codes are resent per page [databook §2.1, §2.2.3] |
| 20 | Print with feeding `1A` (last page) | [databook §2.2.3] |

Ordering caveat: the overview's control-code list places Various mode
(ESC i M) before Advanced mode (ESC i K) [databook §2.1]; nothing in the
databook states whether the relative order of the control codes matters
[unknown — spec-gap candidate]. (oh-brother's Cube path sends ESC i K
before ESC i M and prints fine `[hardware-verified]`.)

## 8. Target mapping (current oh-brother implementation)

Authoritative code paths, cited file:line as of this writing:

### 8.1 Python oracle — `src/oh_brother/protocol.py`

**Shared status path** (`Printer.status`, protocol.py:417–436): drain →
100 × `00` + `1B 40` (protocol.py:420) → `1B 69 61 01` only when
`needs_print_info` (Cube; protocol.py:421–422) → `1B 69 53`
(protocol.py:423) → parse the 32-byte reply (protocol.py:424–436).

**PT-H500 minimal print path** (`print_image`, protocol.py:504–508 else
branch): `4D 02` (compression), then `1B 69 52 01`, then optional
`1B 69 4D 40` when precut is requested, then raster lines as `47`-framed
PackBits (protocol.py:527–529), then `0C`/`1A` (protocol.py:532). No
completion wait (`confirm_print=False`, protocol.py:127–133).

**PT-P300BT full-preamble path** (`_write_print_info`,
protocol.py:438–476 + print_image:501–503): `1B 69 7A` with flags C4h,
type 01h, status width, length 0, 32-bit LE raster count, page 0, 0 →
`1B 69 4B` 08h (or 00h chained) → `1B 69 4D 00` → `1B 69 64` u16-LE end
margin (0 chained / 14 save-tape / round(lead_margin_mm · 180 / 25.4)
default) → `4D 02` → raster (`5A` for blank columns, `47`-framed
otherwise) → `0C`/`1A` → block for status type 01h
(`_wait_print_done`, protocol.py:536–559).

### 8.2 Rust mirror — `rust/pt-protocol/src/lib.rs`

Same byte streams: model specs at lib.rs:107–118, status at
lib.rs:243–246, Cube preamble `write_print_info` at lib.rs:267–308, print
path at lib.rs:313–366 (H500 branch lib.rs:326–329, Z shortcut
lib.rs:343–348, `47`-framing lib.rs:350–356, print command
lib.rs:359–361). Transports: `rust/pt-protocol/src/usb.rs` (rusb),
`rust/pt-protocol/src/bt_macos.rs` + `swift/ptbt.swift` (Cube RFCOMM).

### 8.3 Byte-identical golden tests

`tests/test_protocol.py` — `test_cube_print_stream_is_exact` (line 103),
`test_h500_print_stream_is_exact` (line 154), plus save-tape (line 123)
and chain (line 134) margin encodings. Mirrored in
`rust/pt-protocol/src/tests.rs` — `cube_print_stream_is_exact` (line
122), `h500_print_stream_is_exact` (line 184), save-tape (line 143),
chain (line 155). Protocol changes land in Python first and the Rust
goldens must stay byte-identical (AGENTS.md, Rust port section). The
browser transport mirrors the same bytes in
`src/oh_brother/static/labelcore.js` (AGENTS.md "Sync points").

### 8.4 Delta: current H500 path vs the databook's prescribed sequence

Stated factually; no recommendation implied:

1. **No per-page control-code preamble.** The H500 path sends none of
   ESC i a, ESC i z, ESC i M (unless precut), ESC i K, or ESC i d; the
   databook's walkthrough sends all of them each page [databook §2.2.3].
   Consequence: the H500 job currently has no software margin setting at
   all — AGENTS.md records this as the known loose end ("the end-margin
   control (ESC i d) is wired for the Cube only — the PT-H500 path never
   sends it").
2. **`1B 69 52 01` is not a databook command.** The H500 path's
   raster-mode switch uses ESC i R, which does not appear in the
   databook's command list [databook §3]; the databook's mode switch is
   ESC i a (`1B 69 61 01`). ESC i R works on the H500
   `[hardware-verified]` but is undocumented by this source
   [unknown — spec-gap candidate].
3. **Raster framing byte.** The repo sends `47h` ('G') + u16-LE length +
   PackBits payload on both models `[hardware-verified]`; the databook
   documents `67h` ('g') + u16-LE count + data for this family
   [databook §3, §4 g] (§5.10).
4. **Compression-select position.** The repo sends `4D 02` before the
   mode switch on the H500 path; the walkthrough places compression
   select last among the control codes [databook §2.2.3].
5. **Invalidate/initialize placement.** The repo sends invalidate +
   ESC @ inside `status()`, so a typical print session does execute them
   before print data, matching the databook's job-start steps
   [databook §2.1] — but `print_image` itself does not resend them, and
   nothing enforces that `status()` ran first for the H500 (the Cube path
   calls `status()` implicitly via `_write_print_info` when the width is
   unknown, protocol.py:442–443).
6. **No completion wait on H500.** The databook's lifecycle confirms
   completion status per page [databook §1]; the H500 path returns
   immediately after the print command (`confirm_print=False`)
   `[hardware-verified]` as working for single-page jobs.

## 9. Gotchas, ambiguities, and per-fact confidence

### 9.1 Extraction ambiguities (re-verify against the PDF before relying)

- The §2.3.3 margin-limits table is scrambled in extraction; the
  2 mm/14-dot, 24.3 mm/172-dot and 127 mm/900-dot pairings are recovered
  by unit matching, but the "no precut" row's meaning is unclear (§6.1).
- Cooling notification codes: shown in flow chart §5.8 but absent from
  the extracted notification table (§5.3).
- ESC i K bit 7 ("no buffer clearing when printing") — the extracted
  paragraph is self-referential and awkward; the operational description
  in §5.7 is a best reading.
- Error-information-2 bit 0's "(with a serial connecting)" qualifier is
  truncated in extraction (§5.3).
- The text-color table is numbered (10) in the source body but referenced
  as (9) from the layout table (§5.3) — a databook-internal inconsistency.
- The split-media (×N) page-size table (§6.4) is interleaved in
  extraction; individual split-media values should be reconfirmed.
- Hardware-settings field (status offsets 26–29): no per-byte definition
  legible [unknown — spec-gap candidate].
- Flow-chart section numbering in the source skips 5.4/5.5 (sections run
  5.1, 5.2, 5.3, 5.6, 5.7, 5.8).

### 9.2 Facts that are [hardware-verified] only (no databook citation)

- Everything Cube/PT-P300BT-specific: RFCOMM SPP channel 1; ~25 mm
  mechanical lead; ESC i z + ESC i a required before raster data; 64-dot
  printable window centered in the 128-dot line; no auto-cutter; ESC i d
  behaving as the end margin; job abort when the host disconnects before
  completion; the ~1 s reopen gap; the stale-baseband-link failure mode;
  main-thread-only IOBluetooth (AGENTS.md "Hardware notes" and
  "PT-P300BT Bluetooth gotchas"; protocol.py comments).
- H500 endpoint addresses 0x02 OUT / 0x81 IN (AGENTS.md; the databook
  gives endpoint numbers 1 IN / 2 OUT without the address bytes —
  consistent, but the concrete addresses come from hardware).
- ESC i R (`1B 69 52 01`) as an H500 raster-mode switch (§8.4).
- The `47h`-framed raster transfer working on both models (§5.10).

### 9.3 H500-family vs Cube differences

| Fact | H500 family | PT-P300BT Cube |
|---|---|---|
| Transport | USB 1.1 full speed [databook App. A] | Bluetooth RFCOMM ch. 1 [hardware-verified] |
| Mechanical lead | 24.5 mm [databook §2.3.4] | ~25 mm measured [hardware-verified] |
| Printable dots | up to 128 (24 mm tape) [databook §2.3.5] | middle 64 (12 mm tape max) [hardware-verified] |
| Tape widths | 3.5–24 mm [databook §2.3.2] | 3.5–12 mm [hardware-verified] |
| ESC i z before raster | prescribed by walkthrough [databook §2.2.3]; H500 prints without it [hardware-verified] | required [hardware-verified] |
| Z zero-raster | valid under TIFF [databook §2.1] | verified in use [hardware-verified] |
| Auto-cutter | present (ESC i M bit 6) [databook §4] | absent (manual lever) [hardware-verified] |
| Completion wait | prescribed per page [databook §1] | mandatory — job aborts otherwise [hardware-verified] |

### 9.4 Repo values that differ from this databook (flagged, not resolved)

- **Printable pixels per tape width.** protocol.py:129 has PTH500
  `tape_px = {4:24, 6:32, 9:52, 12:76, 18:120, 24:128}`; the databook's
  print-area pin counts are 24/32/**50**/**70**/**112**/128
  [databook §2.3.5]. The 9/12/18 mm values disagree (52 vs 50, 76 vs 70,
  120 vs 112). Which is correct for driving the H500 is unresolved here
  [unknown — spec-gap candidate; the repo values predate this spec and
  print successfully, but exceeding the databook's print-area pins may
  spill ink positions outside the guaranteed area].
- **Error-byte decode tables.** protocol.py:84–95 decodes byte 8 bit 1
  (02h) as "end of tape" and byte 9 bit 2 (04h) as "communication error";
  the databook marks both bits unused for this family and instead defines
  byte 8 bit 6 (40h) as high-voltage adapter and byte 9 bit 5 (20h) as
  overheating [databook §4 tables (1)/(2)]. The repo tables likely track
  the other-family reference [unknown — spec-gap candidate for this
  family].

### 9.5 Attestation

- Source document: Brother *Raster Command Reference PT-H500/P700/E500
  Version 1.11*, © 2014 Brother Industries, Ltd. (Google Drive file id
  `1UAp_Efs6NSkBN737CMb4R8nodjWTqq1A`).
- Spec drafted: 2026-08-15, from the project's text extraction of that
  PDF plus repo-recorded hardware notes only. No third-party driver
  source was consulted.
- Verification record (verifier: fill in after checking flagged items
  against the PDF/hardware):
  - [ ] §6.1 margin-limits table readings confirmed against the PDF
  - [ ] §5.7 ESC i K bit 7 semantics confirmed
  - [ ] §9.4 tape_px discrepancy resolved (PDF re-read and/or H500 test)
  - [ ] §9.4 error-bit tables resolved for this family
  - [ ] §6.2 ESC i d one-end vs both-ends semantics resolved on H500
  - Verified by: ____________  Date: ____________

## 10. USB identity (Appendix A of the databook)

USB 1.1, full speed, printer class, self-powered (bus-power flag also
set), one interface with no alternates [databook App. A]:

- Vendor ID **04F9h**; Product IDs: PT-H500 **205Eh**, PT-E500 **205Fh**,
  PT-P700 **2061h**.
- Manufacturer string "Brother"; serial-number string = last nine digits
  of the printer serial.
- Endpoint 1: bulk IN, 64-byte max packet (status to host). Endpoint 2:
  bulk OUT, 64-byte max packet (commands/data to printer).
- Concrete endpoint addresses used by oh-brother: OUT 0x02, IN 0x81
  `[hardware-verified]` (AGENTS.md "Hardware notes").
