// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Code 128 encoding, ported from python-barcode's `Code128` (the
//! Python renderer's engine, and the source of labelcore.js's
//! extracted pattern table). The charset-switching walk mirrors
//! python-barcode exactly — including opening in Code C for data that
//! starts with a digit pair — so the two renderers emit identical
//! bars; golden tests below pin that.

use crate::RenderError;

// The 106 code patterns, index = code value (python-barcode CODES).
const CODES: [&str; 106] = [
    "11011001100",
    "11001101100",
    "11001100110",
    "10010011000",
    "10010001100",
    "10001001100",
    "10011001000",
    "10011000100",
    "10001100100",
    "11001001000",
    "11001000100",
    "11000100100",
    "10110011100",
    "10011011100",
    "10011001110",
    "10111001100",
    "10011101100",
    "10011100110",
    "11001110010",
    "11001011100",
    "11001001110",
    "11011100100",
    "11001110100",
    "11101101110",
    "11101001100",
    "11100101100",
    "11100100110",
    "11101100100",
    "11100110100",
    "11100110010",
    "11011011000",
    "11011000110",
    "11000110110",
    "10100011000",
    "10001011000",
    "10001000110",
    "10110001000",
    "10001101000",
    "10001100010",
    "11010001000",
    "11000101000",
    "11000100010",
    "10110111000",
    "10110001110",
    "10001101110",
    "10111011000",
    "10111000110",
    "10001110110",
    "11101110110",
    "11010001110",
    "11000101110",
    "11011101000",
    "11011100010",
    "11011101110",
    "11101011000",
    "11101000110",
    "11100010110",
    "11101101000",
    "11101100010",
    "11100011010",
    "11101111010",
    "11001000010",
    "11110001010",
    "10100110000",
    "10100001100",
    "10010110000",
    "10010000110",
    "10000101100",
    "10000100110",
    "10110010000",
    "10110000100",
    "10011010000",
    "10011000010",
    "10000110100",
    "10000110010",
    "11000010010",
    "11001010000",
    "11110111010",
    "11000010100",
    "10001111010",
    "10100111100",
    "10010111100",
    "10010011110",
    "10111100100",
    "10011110100",
    "10011110010",
    "11110100100",
    "11110010100",
    "11110010010",
    "11011011110",
    "11011110110",
    "11110110110",
    "10101111000",
    "10100011110",
    "10001011110",
    "10111101000",
    "10111100010",
    "11110101000",
    "11110100010",
    "10111011110",
    "10111101110",
    "11101011110",
    "11110101110",
    "11010000100",
    "11010010000",
    "11010011100",
];

const STOP: &str = "11000111010";

const START_A: u8 = 103;
const START_B: u8 = 104;
const START_C: u8 = 105;
const TO_A_FROM_B_OR_C: u8 = 101;
const TO_B_FROM_A_OR_C: u8 = 100;
const TO_C_FROM_A_OR_B: u8 = 99;

#[derive(Clone, Copy, PartialEq)]
enum Charset {
    A,
    B,
    C,
}

/// Code value of `ch` in charset A (python-barcode's `code128.A`).
/// The four accented letters are python-barcode's stand-ins for the
/// FNC1–FNC4 function codes.
fn value_a(ch: char) -> Option<u8> {
    match ch {
        ' '..='_' => Some(ch as u8 - 0x20),
        '\u{00}'..='\u{1f}' => Some(ch as u8 + 64),
        'ó' => Some(96),
        'ò' => Some(97),
        'ô' => Some(101),
        'ñ' => Some(102),
        _ => None,
    }
}

/// Code value of `ch` in charset B (python-barcode's `code128.B`).
fn value_b(ch: char) -> Option<u8> {
    match ch {
        ' '..='\u{7f}' => Some(ch as u8 - 0x20),
        'ó' => Some(96),
        'ò' => Some(97),
        'ô' => Some(100),
        'ñ' => Some(102),
        _ => None,
    }
}

struct Encoder {
    code: Vec<char>,
    charset: Charset,
    buffer: Option<char>, // an unpaired digit while in charset C
}

impl Encoder {
    /// The charset-switch code emitted while still in the old charset
    /// (python-barcode `_new_charset`).
    fn new_charset(&mut self, which: Charset) -> u8 {
        let code = match which {
            Charset::A => TO_A_FROM_B_OR_C,
            Charset::B => TO_B_FROM_A_OR_C,
            Charset::C => TO_C_FROM_A_OR_B,
        };
        self.charset = which;
        code
    }

    /// More than three consecutive digits ahead (python-barcode's
    /// `look_next`, window of 10).
    fn look_next(&self, pos: usize) -> bool {
        self.code[pos..(pos + 10).min(self.code.len())]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
            > 3
    }

    /// python-barcode `_maybe_switch_charset`, verbatim logic.
    fn maybe_switch_charset(&mut self, pos: usize) -> Vec<u8> {
        let ch = self.code[pos];
        let mut codes = Vec::new();
        match self.charset {
            Charset::C if !ch.is_ascii_digit() => {
                if value_b(ch).is_some() {
                    codes.push(self.new_charset(Charset::B));
                } else if value_a(ch).is_some() {
                    codes.push(self.new_charset(Charset::A));
                }
                if let Some(digit) = self.buffer.take() {
                    codes.push(self.convert(digit).expect("digit encodes"));
                }
            }
            Charset::B => {
                if self.look_next(pos) {
                    codes.push(self.new_charset(Charset::C));
                } else if value_b(ch).is_none() && value_a(ch).is_some() {
                    codes.push(self.new_charset(Charset::A));
                }
            }
            Charset::A => {
                if self.look_next(pos) {
                    codes.push(self.new_charset(Charset::C));
                } else if value_a(ch).is_none() && value_b(ch).is_some() {
                    codes.push(self.new_charset(Charset::B));
                }
            }
            Charset::C => {}
        }
        codes
    }

    /// python-barcode `_convert`: None means "digit buffered" in C.
    fn convert(&mut self, ch: char) -> Option<u8> {
        match self.charset {
            Charset::A => value_a(ch),
            Charset::B => value_b(ch),
            Charset::C => {
                if ch == 'ñ' {
                    return Some(102);
                }
                match self.buffer.take() {
                    Some(first) => {
                        let value = (first as u8 - b'0') * 10 + (ch as u8 - b'0');
                        Some(value)
                    }
                    None => {
                        self.buffer = Some(ch);
                        None
                    }
                }
            }
        }
    }

    fn build(&mut self) -> Vec<u8> {
        let mut encoded = vec![START_C];
        for pos in 0..self.code.len() {
            encoded.extend(self.maybe_switch_charset(pos));
            if let Some(value) = self.convert(self.code[pos]) {
                encoded.push(value);
            }
        }
        // Finally look in the buffer
        if let Some(digit) = self.buffer.take() {
            encoded.push(self.new_charset(Charset::B));
            encoded.push(self.convert(digit).expect("digit encodes"));
        }
        // _try_to_optimize: a switch right after the start code folds
        // into starting in that charset directly. Deliberate divergence
        // from python-barcode: it also folds TO_C (99), but encoding
        // always STARTS in Code C, so a 99 at position 1 can only ever
        // be the data pair "99" — python-barcode deletes it ("9912"
        // encodes as "12", "99" as nothing). Values 100/101 cannot be
        // Code C data pairs (those stop at 99), so those folds are safe.
        if encoded.len() >= 2 {
            let folded = match encoded[1] {
                TO_A_FROM_B_OR_C => Some(START_A),
                TO_B_FROM_A_OR_C => Some(START_B),
                _ => None,
            };
            if let Some(start) = folded {
                encoded.splice(0..2, [start]);
            }
        }
        encoded
    }
}

/// The full bar pattern ('1' = bar module, '0' = space module) for
/// `data`, checksum and stop pattern included — the equivalent of
/// python-barcode's `build()[0]`.
pub fn pattern(data: &str) -> Result<String, RenderError> {
    let invalid: Vec<char> = data
        .chars()
        .filter(|&c| value_a(c).is_none() && value_b(c).is_none())
        .collect();
    if !invalid.is_empty() {
        return Err(RenderError(format!(
            "not encodable in Code 128: {}",
            invalid
                .iter()
                .map(|c| format!("{c:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let mut encoder = Encoder {
        code: data.chars().collect(),
        charset: Charset::C,
        buffer: None,
    };
    let mut encoded = encoder.build();
    let checksum = (u32::from(encoded[0])
        + encoded[1..]
            .iter()
            .enumerate()
            .map(|(i, &v)| (i as u32 + 1) * u32::from(v))
            .sum::<u32>())
        % 103;
    encoded.push(checksum as u8);
    let mut out: String = encoded
        .iter()
        .map(|&v| CODES[usize::from(v)])
        .collect::<Vec<_>>()
        .join("");
    out.push_str(STOP);
    out.push_str("11");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::pattern;

    /// Golden outputs from python-barcode (the oracle labelcore.js was
    /// extracted from), generated with
    /// `barcode.get("code128", data).build()[0]`.
    #[test]
    fn matches_python_barcode() {
        let cases: &[(&str, &str)] = &[
            ("A", "1101001000010100011000100010110001100011101011"),
            (
                "HELLO",
                "110100100001100010100010001101000100011011101000110111010001110110110001010001100011101011",
            ),
            (
                "12V DC",
                "11010011100101100111001011110111011101011000110110011001011000100010001000110101110110001100011101011",
            ),
            (
                "1234567890",
                "110100111001011001110010001011000111000101101100001010011011110110100111100101100011101011",
            ),
            (
                "https://example.com/x?y=1",
                "1101001000010011000010100111101001001111010010100111100101111001001110010011010111001100101110011001011001000011110010010100101100001111011101010100111100110010100001011001000010011001110100001011001000111101011110111010101110011001111001001011011000110110110111101110011001010011100110100110010001100011101011",
            ),
            (
                "Hi there 42",
                "110100100001100010100010000110100110110011001001111010010011000010101100100001001001111010110010000110110011001100100111011001110010110001010001100011101011",
            ),
            (
                "123",
                "11010011100101100111001011110111011001011100100101100001100011101011",
            ),
            (
                "AB12345CD",
                "11010010000101000110001000101100010111011110101100111001000101100010111101110110111001001000100011010110001000110001001001100011101011",
            ),
            (
                "a1b2",
                "1101001000010010110000100111001101001000011011001110010110010000101100011101011",
            ),
            (
                "{weird}~stuff",
                "1101001000011110110110111100101001011001000010000110100100100111101000010011010100011110100010111101011110010010011110100100111100101011000010010110000100111011101101100011101011",
            ),
            (
                "12345678",
                "1101001110010110011100100010110001110001011011000010100100011101101100011101011",
            ),
            (
                "X99",
                "11010010000111000101101110010110011100101100100011110101100011101011",
            ),
        ];
        for (data, expected) in cases {
            assert_eq!(&pattern(data).unwrap(), expected, "data {data:?}");
        }
    }

    /// Data whose first digit pair is "99" hits a python-barcode bug
    /// (its start-fold optimization mistakes the 99 pair for a TO_C
    /// switch and deletes it). We deliberately diverge and keep the
    /// pair; assert the correct code-value sequences.
    #[test]
    fn leading_99_pair_survives() {
        let expect = |values: &[u8]| {
            let mut s: String = values
                .iter()
                .map(|&v| super::CODES[usize::from(v)])
                .collect();
            s.push_str("1100011101011");
            s
        };
        // [START_C, 99, 12] + checksum 22
        assert_eq!(pattern("9912").unwrap(), expect(&[105, 99, 12, 22]));
        // [START_C, 99] + checksum 101
        assert_eq!(pattern("99").unwrap(), expect(&[105, 99, 101]));
    }

    #[test]
    fn rejects_unencodable() {
        assert!(pattern("héllo").is_err());
        assert!(pattern("日本").is_err());
    }
}
