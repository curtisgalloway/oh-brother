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

// Node-side unit tests for labelcore.js's DOM-free logic. Run directly
// (`node tests/labelcore_test.js`) or via pytest, which also feeds
// argv[2] through code128Pattern to compare against python-barcode.

"use strict";
const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const src = fs.readFileSync(
  path.join(__dirname, "..", "static", "labelcore.js"),
  "utf8"
);
const ctx = {};
vm.createContext(ctx);
vm.runInContext(src + "\n;globalThis.LabelCoreExport = LabelCore;", ctx);
const LC = ctx.LabelCoreExport;

// cross-renderer mode: print the Code 128 pattern for argv[2] and exit,
// so pytest can compare it byte-for-byte with python-barcode's build().
if (process.argv[2]) {
  process.stdout.write(LC.code128Pattern(process.argv[2]));
  process.exit(0);
}

// vm-context objects live in another realm, so compare via JSON, not
// deepStrictEqual (which insists on same-realm prototypes).
const json = v => JSON.stringify(v);

assert.strictEqual(LC.TAPE_PX[12], 76);

// configure(): text entries keep their string, icon entries become the
// icon-font codepoint, and the SYMBOLS getter sees updates.
assert.strictEqual(LC.SYMBOLS.length, 0);
LC.configure({
  symbols: [
    { name: "mm2", kind: "text", text: "mm²", keywords: "unit" },
    { name: "plug", kind: "icon", cp: 0xe63c, keywords: "power" },
  ],
  fallbackFamilies: ["Oh Brother Icons"],
});
assert.strictEqual(LC.SYMBOLS.length, 2);
assert.strictEqual(LC.SYMBOLS[0].char, "mm²");
assert.strictEqual(LC.SYMBOLS[1].char, String.fromCodePoint(0xe63c));

// markdown runs
assert.strictEqual(
  json(LC.markdownRuns("**hot** stuff")),
  json([{ bold: true, text: "hot" }, { text: " stuff" }])
);
assert.strictEqual(
  json(LC.markdownRuns("a \\*b\\* c")), json([{ text: "a *b* c" }])
);
assert.strictEqual(
  json(LC.markdownRuns("5 * 3 * 2")), json([{ text: "5 * 3 * 2" }])
);

// Code 128: check digit and framing for a known value.
// START_B(104) + "A"(33) -> check (104 + 33*1) % 103 = 34
const expected =
  "11010010000" + "10100011000" + "10001011000" + "1100011101011";
assert.strictEqual(LC.code128Pattern("A"), expected);
assert.throws(() => LC.code128Pattern("é"), /not encodable/);

// status parsing: 12 mm tape, no errors
const status = new Uint8Array(32);
status[0] = 0x80; status[1] = 0x20; status[10] = 12; status[11] = 0x01;
const st = LC.parseStatus(status);
assert.strictEqual(st.tape_px, 76);
assert.strictEqual(st.errors.length, 0);
assert.strictEqual(st.media, "laminated (TZe)");

console.log("labelcore tests ok");
