// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// labelcore: browser-side label renderer + Brother PT-H500 raster protocol.
//
// This is the single renderer for the web UI in BOTH run modes. The only
// difference between modes is the transport: ServerTransport POSTs the
// rendered bitmap to the Python server (which owns USB), WebUsbTransport
// claims the printer directly from the browser (Chromium only).
//
// The Python side (render.py) has a parallel Pillow renderer used by the
// CLI and the /api/render HTTP API; keep the qr:/code: directive syntax
// in sync between the two.

"use strict";

const LabelCore = (() => {
  const HEAD_PX = 128;
  const DPI = 180;
  const TAPE_PX = { 4: 24, 6: 32, 9: 52, 12: 76, 18: 120, 24: 128 };

  // ---- Code 128 (pattern table extracted from python-barcode) ----
  const CODE128_PATTERNS = ["11011001100", "11001101100", "11001100110", "10010011000", "10010001100", "10001001100", "10011001000", "10011000100", "10001100100", "11001001000", "11001000100", "11000100100", "10110011100", "10011011100", "10011001110", "10111001100", "10011101100", "10011100110", "11001110010", "11001011100", "11001001110", "11011100100", "11001110100", "11101101110", "11101001100", "11100101100", "11100100110", "11101100100", "11100110100", "11100110010", "11011011000", "11011000110", "11000110110", "10100011000", "10001011000", "10001000110", "10110001000", "10001101000", "10001100010", "11010001000", "11000101000", "11000100010", "10110111000", "10110001110", "10001101110", "10111011000", "10111000110", "10001110110", "11101110110", "11010001110", "11000101110", "11011101000", "11011100010", "11011101110", "11101011000", "11101000110", "11100010110", "11101101000", "11101100010", "11100011010", "11101111010", "11001000010", "11110001010", "10100110000", "10100001100", "10010110000", "10010000110", "10000101100", "10000100110", "10110010000", "10110000100", "10011010000", "10011000010", "10000110100", "10000110010", "11000010010", "11001010000", "11110111010", "11000010100", "10001111010", "10100111100", "10010111100", "10010011110", "10111100100", "10011110100", "10011110010", "11110100100", "11110010100", "11110010010", "11011011110", "11011110110", "11110110110", "10101111000", "10100011110", "10001011110", "10111101000", "10111100010", "11110101000", "11110100010", "10111011110", "10111101110", "11101011110", "11110101110", "11010000100", "11010010000", "11010011100"];
  const CODE128_STOP = "1100011101011";
  const CODE128_START_B = 104;

  function code128Pattern(data) {
    const vals = [];
    for (const ch of data) {
      const v = ch.charCodeAt(0) - 32;
      if (v < 0 || v > 94) throw new Error(`character ${JSON.stringify(ch)} not encodable in Code 128 B`);
      vals.push(v);
    }
    let check = CODE128_START_B;
    vals.forEach((v, i) => { check += v * (i + 1); });
    check %= 103;
    return [CODE128_START_B, ...vals, check].map(v => CODE128_PATTERNS[v]).join("") + CODE128_STOP;
  }

  // ---- symbol catalog + fallback fonts (injected from fonts.json) ----
  // configure() receives the manifest's symbol catalog (text entries
  // insert their string; icon entries insert the icon-font codepoint)
  // and the fallback font families appended to every canvas font stack
  // so icons/symbols/emoji resolve per glyph, mirroring render.py.
  let SYMBOLS = [];
  let FALLBACK_FAMILIES = [];

  function configure({ symbols = [], fallbackFamilies = [] } = {}) {
    SYMBOLS = symbols.map(s => ({
      name: s.name,
      char: s.kind === "icon" ? String.fromCodePoint(s.cp) : s.text,
      keywords: s.keywords || "",
    }));
    FALLBACK_FAMILIES = fallbackFamilies;
  }

  // Force text presentation so canvas does not draw color emoji glyphs,
  // which threshold into unreadable blobs.
  function textPresentation(s) {
    return s.replace(/[\u2190-\u21FF\u2300-\u27BF\u2B00-\u2BFF]/g, "$&\uFE0E");
  }

  // ---- inline markdown ----
  const MD_ESCAPES = { "*": "\uE000", "_": "\uE001", "`": "\uE002", "~": "\uE003", "\\": "\uE004" };
  const MD_UNESCAPES = { "\uE000": "*", "\uE001": "_", "\uE002": "`", "\uE003": "~", "\uE004": "\\" };
  // Content must start and end with non-space (CommonMark-ish flanking),
  // so "a * b * c" and "5 * 3" stay literal.
  const MD_PATTERN = /\*\*\*(\S(?:.*?\S)?)\*\*\*|\*\*(\S(?:.*?\S)?)\*\*|\*(\S(?:.*?\S)?)\*|_(\S(?:.*?\S)?)_|`(\S(?:.*?\S)?)`|~~(\S(?:.*?\S)?)~~/;

  function parseInline(text, style) {
    style = style || {};
    const runs = [];
    let rest = text;
    while (rest) {
      const m = MD_PATTERN.exec(rest);
      if (!m) { runs.push({ ...style, text: rest }); break; }
      if (m.index > 0) runs.push({ ...style, text: rest.slice(0, m.index) });
      const [boldItalic, bold, star, under, mono, strike] = m.slice(1);
      if (boldItalic !== undefined) runs.push(...parseInline(boldItalic, { ...style, bold: true, italic: true }));
      else if (bold !== undefined) runs.push(...parseInline(bold, { ...style, bold: true }));
      else if (star !== undefined) runs.push(...parseInline(star, { ...style, italic: true }));
      else if (under !== undefined) runs.push(...parseInline(under, { ...style, italic: true }));
      else if (mono !== undefined) runs.push({ ...style, mono: true, text: mono });  // code spans do not nest
      else runs.push(...parseInline(strike, { ...style, strike: true }));
      rest = rest.slice(m.index + m[0].length);
    }
    return runs;
  }

  // Unmatched markers stay literal (the regex only consumes closed pairs);
  // backslash-escaped markers are hidden from the parser and restored after.
  function markdownRuns(line) {
    const escaped = line.replace(/\\([*_`~\\])/g, (_, ch) => MD_ESCAPES[ch]);
    return parseInline(escaped, {})
      .map(r => ({ ...r, text: r.text.replace(/[\uE000-\uE004]/g, ch => MD_UNESCAPES[ch]) }))
      .filter(r => r.text);
  }

  // ---- rendering ----
  function makeCanvas(w, h) {
    const c = document.createElement("canvas");
    c.width = Math.max(1, Math.round(w));
    c.height = Math.max(1, Math.round(h));
    const ctx = c.getContext("2d", { willReadFrequently: true });
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, c.width, c.height);
    ctx.fillStyle = "#000";
    return c;
  }

  function familyStack(primary) {
    const fams = [primary, ...FALLBACK_FAMILIES].map(f => `"${f}"`);
    return fams.join(", ");
  }

  function runFontString(sizePx, font, run) {
    const style = run.italic ? "italic " : "";
    const weight = run.bold ? "bold " : font.weight ? font.weight + " " : "";
    const fam = run.mono
      ? familyStack("IBM Plex Mono") + ", monospace"
      : familyStack(font.family);
    return `${style}${weight}${sizePx}px ${fam}`;
  }

  function fontString(sizePx, font) {
    return runFontString(sizePx, font, {});
  }

  function fontMetrics(ctx, sizePx, font) {
    ctx.font = fontString(sizePx, font);
    const m = ctx.measureText("Mg");
    const asc = m.fontBoundingBoxAscent ?? m.actualBoundingBoxAscent;
    const desc = m.fontBoundingBoxDescent ?? m.actualBoundingBoxDescent;
    return { asc, desc };
  }

  function fitFontSize(ctx, lineHeight, font) {
    let lo = 4, hi = 4 * lineHeight;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      const { asc, desc } = fontMetrics(ctx, mid, font);
      if (asc + desc <= lineHeight) lo = mid; else hi = mid - 1;
    }
    return lo;
  }

  function renderTextSegment(lines, heightPx, font, fontSize, lineGap, hscale, markdown) {
    const n = lines.length;
    const lineHeight = Math.floor((heightPx - lineGap * (n - 1)) / n);
    if (lineHeight < 4) throw new Error(`${n} lines do not fit in ${heightPx} px of tape`);
    const probe = makeCanvas(1, 1).getContext("2d");
    const size = fontSize || fitFontSize(probe, lineHeight, font);
    const lineRuns = lines.map(l =>
      (markdown ? markdownRuns(l) : [{ text: l }])
        .map(r => ({ ...r, text: textPresentation(r.text) }))
    );
    const measure = runs => runs.reduce((a, r) => {
      probe.font = runFontString(size, font, r);
      return a + probe.measureText(r.text).width;
    }, 0);
    const widths = lineRuns.map(rs => Math.ceil(measure(rs)));
    const width = Math.max(1, ...widths);
    const canvas = makeCanvas(width * hscale, heightPx);
    const ctx = canvas.getContext("2d");
    ctx.setTransform(hscale, 0, 0, 1, 0, 0);
    ctx.fillStyle = "#000";
    const { asc, desc } = fontMetrics(ctx, size, font);
    const total = n * lineHeight + (n - 1) * lineGap;
    const y0 = Math.floor((heightPx - total) / 2);
    lineRuns.forEach((runs, i) => {
      const cy = y0 + i * (lineHeight + lineGap) + lineHeight / 2;
      const baseline = cy - (asc + desc) / 2 + asc;
      let x = (width - widths[i]) / 2;
      for (const r of runs) {
        ctx.font = runFontString(size, font, r);
        ctx.fillText(r.text, x, baseline);
        const w = ctx.measureText(r.text).width;
        if (r.strike) {
          ctx.fillRect(x, baseline - size * 0.28, w, Math.max(1.5, size / 14));
        }
        x += w;
      }
    });
    return canvas;
  }

  function renderCode128(data, heightPx, font) {
    const pattern = code128Pattern(data);
    const modulePx = 3, quiet = 10 * modulePx;
    const capH = heightPx >= 45 ? Math.max(14, Math.min(26, Math.floor(heightPx / 3))) : 0;
    const barH = heightPx - capH;
    let canvas = makeCanvas(pattern.length * modulePx + 2 * quiet, heightPx);
    const ctx = canvas.getContext("2d");
    for (let i = 0; i < pattern.length; i++) {
      if (pattern[i] === "1") ctx.fillRect(quiet + i * modulePx, 0, modulePx, barH);
    }
    if (capH) {
      const cap = renderTextSegment([data], capH, font, null, 0, 1);
      if (cap.width > canvas.width) {
        const wider = makeCanvas(cap.width, heightPx);
        wider.getContext("2d").drawImage(canvas, (cap.width - canvas.width) / 2, 0);
        canvas = wider;
      }
      canvas.getContext("2d").drawImage(cap, (canvas.width - cap.width) / 2, barH);
    }
    return canvas;
  }

  function renderQr(data, heightPx) {
    const qr = qrcode(0, "M");
    qr.addData(data);
    qr.make();
    const modules = qr.getModuleCount();
    const border = 2;
    const box = Math.max(1, Math.floor(heightPx / (modules + 2 * border)));
    if (box === 1 && modules + 2 * border > heightPx) {
      throw new Error(`QR code needs ${modules + 2 * border} px but tape is ${heightPx} px`);
    }
    const side = (modules + 2 * border) * box;
    const canvas = makeCanvas(side, heightPx);
    const ctx = canvas.getContext("2d");
    const y0 = Math.floor((heightPx - side) / 2);
    for (let r = 0; r < modules; r++) {
      for (let c = 0; c < modules; c++) {
        if (qr.isDark(r, c)) {
          ctx.fillRect((border + c) * box, y0 + (border + r) * box, box, box);
        }
      }
    }
    return canvas;
  }

  const DIRECTIVE_RE = /^(qr|code):\s*(.*\S)\s*$/i;

  // grid:<width>/<cells> — a fixed-width strip of equal cells, e.g.
  // grid:5u/6 (Gridfinity units, 42 mm each) or grid:210mm/6. The lines
  // after the directive are the cell texts, one per cell.
  const GRID_RE = /^grid:\s*([0-9]+(?:\.[0-9]+)?)\s*(u|mm)\s*\/\s*([0-9]+)\s*$/i;
  const GRIDFINITY_U_MM = 42;

  function cellRuns(line, font, markdown) {
    return (markdown ? markdownRuns(line) : [{ text: line }])
      .map(r => ({ ...r, text: textPresentation(r.text) }));
  }

  function measureRuns(probe, runs, size, font) {
    return runs.reduce((a, r) => {
      probe.font = runFontString(size, font, r);
      return a + probe.measureText(r.text).width;
    }, 0);
  }

  function renderGrid(widthMm, nCells, cells, opts) {
    const { heightPx, font, fontSize = null, markdown = false } = opts;
    const totalPx = Math.round(widthMm / 25.4 * DPI);
    const cellW = totalPx / nCells;
    const pad = 4, dividerPx = 1;
    const avail = Math.floor(cellW) - 2 * pad - dividerPx;
    if (avail < 8) {
      throw new Error(`${nCells} cells across ${widthMm} mm leaves only ${avail} px per cell`);
    }
    const probe = makeCanvas(1, 1).getContext("2d");
    let size = fontSize;
    if (!size) {
      size = fitFontSize(probe, heightPx, font);
      for (const cell of cells) {
        if (!cell.trim()) continue;
        const runs = cellRuns(cell, font, markdown);
        let lo = 4, hi = size;
        while (lo < hi) {
          const mid = (lo + hi + 1) >> 1;
          if (measureRuns(probe, runs, mid, font) <= avail) lo = mid; else hi = mid - 1;
        }
        size = lo;
      }
    }
    const canvas = makeCanvas(totalPx, heightPx);
    const ctx = canvas.getContext("2d");
    cells.forEach((cell, i) => {
      if (!cell.trim()) return;
      const seg = renderTextSegment([cell], heightPx, font, size, 0, 1, markdown);
      ctx.drawImage(seg, Math.round(i * cellW + (cellW - seg.width) / 2), 0);
    });
    for (let i = 1; i < nCells; i++) {
      ctx.fillRect(Math.round(i * cellW), 0, dividerPx, heightPx);
    }
    return canvas;
  }

  function tryGrid(lines, opts) {
    const first = lines.findIndex(l => l.trim());
    if (first < 0) return null;
    const m = GRID_RE.exec(lines[first].trim());
    if (!m) return null;
    let widthMm = parseFloat(m[1]);
    if (m[2].toLowerCase() === "u") widthMm *= GRIDFINITY_U_MM;
    const nCells = Math.max(1, parseInt(m[3], 10));
    const cells = lines.slice(first + 1);
    if (cells.slice(nCells).some(l => l.trim())) {
      throw new Error(`more cell lines than the ${nCells} declared cells`);
    }
    while (cells.length < nCells) cells.push("");
    return renderGrid(widthMm, nCells, cells.slice(0, nCells), opts);
  }

  // Compose a label from editor text — same conventions as render.py:
  // plain lines stack as text, qr:/code: lines become code segments,
  // laid out left to right; hscale applies to text segments only.
  function renderLabel(text, opts) {
    const { heightPx, font, fontSize = null, marginPx = 8, lineGapPx = 2,
            hscale = 1, markdown = false } = opts;
    const grid = tryGrid(text.split("\n"), opts);
    if (grid) return grid;
    const segments = [];
    let textLines = [];
    const flush = () => {
      while (textLines.length && !textLines[0].trim()) textLines.shift();
      while (textLines.length && !textLines[textLines.length - 1].trim()) textLines.pop();
      if (textLines.length) {
        segments.push(renderTextSegment(
          textLines, heightPx, font, fontSize, lineGapPx, hscale, markdown));
        textLines = [];
      }
    };
    for (const line of text.split("\n")) {
      const m = DIRECTIVE_RE.exec(line);
      if (m) {
        flush();
        if (m[1].toLowerCase() === "qr") segments.push(renderQr(m[2], heightPx));
        else segments.push(renderCode128(m[2], heightPx, font));
      } else {
        textLines.push(line);
      }
    }
    flush();
    if (!segments.length) throw new Error("empty label");
    const gap = 10;
    const width = segments.reduce((a, s) => a + s.width, 0) + gap * (segments.length - 1) + 2 * marginPx;
    const canvas = makeCanvas(width, heightPx);
    const ctx = canvas.getContext("2d");
    let x = marginPx;
    for (const s of segments) {
      ctx.drawImage(s, x, 0);
      x += s.width + gap;
    }
    return canvas;
  }

  // ---- raster packing + protocol ----
  function packColumns(canvas) {
    const w = canvas.width, h = canvas.height;
    if (h > HEAD_PX) throw new Error(`image height ${h} exceeds head (${HEAD_PX} px)`);
    const { data } = canvas.getContext("2d").getImageData(0, 0, w, h);
    const offset = (HEAD_PX - h) >> 1;
    const cols = new Uint8Array(w * 16);
    for (let x = 0; x < w; x++) {
      for (let i = 0; i < h; i++) {
        const y = h - 1 - i;
        const j = (y * w + x) * 4;
        const lum = 0.299 * data[j] + 0.587 * data[j + 1] + 0.114 * data[j + 2];
        if (lum < 128) {
          const p = offset + i;
          cols[x * 16 + (15 - (p >> 3))] |= 1 << (p & 7);
        }
      }
    }
    return { cols, width: w };
  }

  const INIT_BYTES = (() => {
    const b = new Uint8Array(102);
    b[100] = 0x1b; b[101] = 0x40;
    return b;
  })();
  const STATUS_REQUEST = new Uint8Array([0x1b, 0x69, 0x53]);

  function printJobBytes(packed, chain) {
    const { cols, width } = packed;
    const out = new Uint8Array(2 + 4 + width * 20 + 1);
    let o = 0;
    out[o++] = 0x4d; out[o++] = 0x02;              // packbits compression
    out[o++] = 0x1b; out[o++] = 0x69; out[o++] = 0x52; out[o++] = 0x01;  // raster mode
    for (let x = 0; x < width; x++) {
      out[o++] = 0x47; out[o++] = 17; out[o++] = 0;  // G, 16-bit LE payload length
      out[o++] = 15;                                  // packbits: literal run of 16
      out.set(cols.subarray(x * 16, x * 16 + 16), o);
      o += 16;
    }
    out[o++] = chain ? 0x0c : 0x1a;
    return out;
  }

  const MEDIA_TYPES = { 0x00: "no tape", 0x01: "laminated (TZe)", 0x03: "non-laminated", 0x11: "heat-shrink tube", 0xFF: "incompatible tape" };
  const ERRORS_8 = { 0x01: "no tape", 0x02: "end of tape", 0x04: "cutter jam", 0x08: "weak batteries" };
  const ERRORS_9 = { 0x01: "replace the tape", 0x04: "communication error", 0x10: "cover open" };

  function parseStatus(bytes) {
    if (bytes.length !== 32 || bytes[0] !== 0x80 || bytes[1] !== 0x20) {
      throw new Error("unexpected status reply from printer");
    }
    const errors = [];
    for (const [bit, name] of Object.entries(ERRORS_8)) if (bytes[8] & bit) errors.push(name);
    for (const [bit, name] of Object.entries(ERRORS_9)) if (bytes[9] & bit) errors.push(name);
    const mm = bytes[10];
    return {
      connected: true,
      // parseStatus only serves the WebUSB path, which only speaks
      // PT-H500; its ~24.5 mm lead is the head-to-cutter distance per
      // the raster reference ("minimum length of tape that can be fed
      // out").
      model: "PT-H500",
      lead_mm: 24.5,
      tape_mm: mm,
      tape_px: TAPE_PX[mm] || 0,
      media: MEDIA_TYPES[bytes[11]] || `unknown (0x${bytes[11].toString(16)})`,
      errors,
    };
  }

  // ---- transports ----
  class ServerTransport {
    static async probe() {
      try {
        const rsp = await fetch("api/meta");
        return rsp.ok ? await rsp.json() : null;
      } catch { return null; }
    }
    async status(printer) {
      const q = printer ? `?printer=${encodeURIComponent(printer)}` : "";
      return await (await fetch("api/status" + q)).json();
    }
    async print(canvas, { copies, chain, save_tape, printer }) {
      const rsp = await fetch("api/print-raw", {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          png: canvas.toDataURL("image/png"), copies, chain,
          save_tape: save_tape || undefined,
          printer: printer || undefined,
        }),
      });
      const data = await rsp.json();
      if (!rsp.ok) throw new Error(data.error || "print failed");
      return data;
    }
  }

  class WebUsbTransport {
    static supported() { return !!navigator.usb; }

    async connect() {
      this.device = await navigator.usb.requestDevice({
        filters: [{ vendorId: 0x04f9, productId: 0x205e }],
      });
      await this.device.open();
      if (!this.device.configuration) await this.device.selectConfiguration(1);
      await this.device.claimInterface(0);
    }

    get connected() { return !!(this.device && this.device.opened); }

    async _write(bytes) {
      for (let i = 0; i < bytes.length; i += 16384) {
        const r = await this.device.transferOut(2, bytes.subarray(i, i + 16384));
        if (r.status !== "ok") throw new Error(`USB write failed: ${r.status}`);
      }
    }

    async status() {
      if (!this.connected) return { connected: false, error: "not connected" };
      try {
        await this._write(INIT_BYTES);
        await this._write(STATUS_REQUEST);
        const r = await this.device.transferIn(1, 32);
        return parseStatus(new Uint8Array(r.data.buffer));
      } catch (e) {
        try { await this.device.close(); } catch { /* already gone */ }
        this.device = null;
        return { connected: false, error: String(e) };
      }
    }

    async print(canvas, { copies, chain }) {
      const st = await this.status();
      if (!st.connected) throw new Error(st.error || "printer not connected");
      if (st.errors.length) throw new Error("printer reports: " + st.errors.join(", "));
      if (canvas.height !== st.tape_px) throw new Error(`label is ${canvas.height} px but tape is ${st.tape_px} px — re-render`);
      const packed = packColumns(canvas);
      for (let copy = 0; copy < copies; copy++) {
        const last = copy === copies - 1;
        await this._write(printJobBytes(packed, chain || !last));
      }
      return { ok: true, mm: Math.round(canvas.width / DPI * 25.4 * 10) / 10, copies };
    }
  }

  return {
    HEAD_PX, DPI, TAPE_PX, configure,
    get SYMBOLS() { return SYMBOLS; },
    renderLabel, code128Pattern, packColumns, printJobBytes, parseStatus,
    parseInline, markdownRuns,
    ServerTransport, WebUsbTransport,
  };
})();
