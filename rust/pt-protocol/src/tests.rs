// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Golden tests ported from the Python reference
//! (tests/test_protocol.py): the byte streams must match the
//! hardware-verified implementation exactly.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::*;

struct FakeTransport {
    written: Rc<RefCell<Vec<u8>>>,
    replies: Rc<RefCell<VecDeque<u8>>>,
    // Largest read the fake delivers at once — a fragmenting link.
    max_chunk: usize,
}

impl FakeTransport {
    fn new(replies: &[u8]) -> (FakeTransport, Rc<RefCell<Vec<u8>>>) {
        let written = Rc::new(RefCell::new(Vec::new()));
        let t = FakeTransport {
            written: written.clone(),
            replies: Rc::new(RefCell::new(replies.iter().copied().collect())),
            max_chunk: usize::MAX,
        };
        (t, written)
    }
}

impl Transport for FakeTransport {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.written.borrow_mut().extend_from_slice(data);
        Ok(())
    }

    fn read(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut replies = self.replies.borrow_mut();
        let n = max.min(self.max_chunk).min(replies.len());
        Ok(replies.drain(..n).collect())
    }

    fn drain(&mut self) -> Result<()> {
        Ok(()) // queued replies stand in for not-yet-sent responses
    }
}

fn status_reply(width_mm: u8, media: u8, err8: u8, err9: u8, status_type: u8) -> [u8; 32] {
    let mut buf = [0u8; 32];
    (buf[0], buf[1], buf[2], buf[3]) = (0x80, 0x20, 0x42, 0x30);
    (buf[8], buf[9], buf[10], buf[11]) = (err8, err9, width_mm, media);
    buf[18] = status_type;
    buf
}

/// What the Cube streams after the print command: phase change to
/// printing, then "printing completed" (status type 0x01).
fn print_done() -> Vec<u8> {
    let mut v = status_reply(12, 0x01, 0, 0, 0x06).to_vec();
    v.extend(status_reply(12, 0x01, 0, 0, 0x01));
    v
}

/// White label, one black pixel in the top-left corner.
fn one_dot_label(width: u32, height: u32) -> Label {
    let mut black = vec![false; (width * height) as usize];
    black[0] = true;
    Label::new(width, height, black)
}

/// The G-frame for column 0 of one_dot_label, centered in 128 dots.
fn expected_first_line(height: u32) -> Vec<u8> {
    let mut raster = [0u8; 16];
    let p = ((HEAD_PX - height) / 2 + (height - 1)) as usize;
    raster[15 - p / 8] |= 1 << (p % 8);
    let mut frame = vec![0x47, 0x11, 0x00, 15];
    frame.extend(raster);
    frame
}

fn cube_with_status(replies: &[u8]) -> (Printer, Rc<RefCell<Vec<u8>>>) {
    let (t, written) = FakeTransport::new(replies);
    let mut p = Printer::new(Box::new(t), &PTP300BT, "test");
    p.status().unwrap();
    written.borrow_mut().clear();
    (p, written)
}

#[test]
fn cube_status_and_tape_px() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PTP300BT, "test");
    let st = p.status().unwrap();
    assert_eq!(st.media_width_mm, 12);
    assert_eq!(st.tape_px, 64);
    assert_eq!(st.media_type, "laminated (TZe)");
    assert!(st.errors.is_empty());
    let mut expected = vec![0u8; 100];
    expected.extend(b"\x1b\x40\x1b\x69\x61\x01\x1b\x69\x53");
    assert_eq!(*written.borrow(), expected);
}

#[test]
fn h500_status_and_tape_px() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PTH500, "test");
    let st = p.status().unwrap();
    assert_eq!(st.tape_px, 76);
    let mut expected = vec![0u8; 100];
    expected.extend(b"\x1b\x40\x1b\x69\x53");
    assert_eq!(*written.borrow(), expected);
}

#[test]
fn status_errors_reported() {
    let (t, _) = FakeTransport::new(&status_reply(12, 0x01, 0x02, 0x10, 0));
    let mut p = Printer::new(Box::new(t), &PTP300BT, "test");
    let st = p.status().unwrap();
    assert_eq!(st.errors, vec!["end of tape", "cover open"]);
}

#[test]
fn cube_print_stream_is_exact() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend(print_done());
    let (mut p, written) = cube_with_status(&replies);
    p.print(&one_dot_label(3, 64), false, false).unwrap();

    let mut expected = b"\x1b\x69\x7a".to_vec();
    expected.extend([0xC4, 0x01, 12, 0]);
    expected.extend(3u32.to_le_bytes());
    expected.extend([0, 0]);
    expected.extend(b"\x1b\x69\x4b\x08"); // advanced mode: no page chaining
    expected.extend(b"\x1b\x69\x4d\x00"); // various mode: no auto-cut
    expected.extend(b"\x1b\x69\x64\xb1\x00"); // end margin: 177 dots (25 mm) = the lead
    expected.extend(b"\x4d\x02"); // packbits compression
    expected.extend(expected_first_line(64));
    expected.extend(b"\x5a\x5a"); // two blank columns as Z lines
    expected.extend(b"\x1a"); // print and feed
    assert_eq!(*written.borrow(), expected);
}

#[test]
fn cube_save_tape_uses_minimal_end_margin() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend(print_done());
    let (mut p, written) = cube_with_status(&replies);
    p.print(&one_dot_label(3, 64), false, true).unwrap();
    let written = written.borrow();
    // ~2 mm trailing feed instead of matching the mechanical lead
    assert!(written.windows(5).any(|w| w == b"\x1b\x69\x64\x0e\x00"));
    assert_eq!(written.last(), Some(&0x1a));
}

#[test]
fn cube_chain_keeps_page_open() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend(print_done());
    let (mut p, written) = cube_with_status(&replies);
    p.print(&one_dot_label(3, 64), true, false).unwrap();
    let written = written.borrow();
    assert!(written.windows(4).any(|w| w == b"\x1b\x69\x4b\x00"));
    // chained pages keep a 0 end margin so consecutive labels stay flush
    assert!(written.windows(5).any(|w| w == b"\x1b\x69\x64\x00\x00"));
    assert_eq!(written.last(), Some(&0x0c));
}

#[test]
fn cube_rejects_too_tall_images() {
    let (mut p, _) = cube_with_status(&status_reply(12, 0x01, 0, 0, 0));
    let err = p.print(&one_dot_label(3, 76), false, false).unwrap_err();
    assert!(err.0.contains("exceeds"), "{err}");
}

/// RFCOMM may hand a 32-byte block over in pieces. The reader must
/// take only what is missing from the current block, so the head of
/// the NEXT block (here "printing completed") is never swallowed and
/// discarded along with the tail of the previous one.
#[test]
fn cube_fragmented_status_blocks_stay_aligned() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend(print_done());
    let (mut t, _) = FakeTransport::new(&replies);
    t.max_chunk = 20; // 32-byte blocks arrive as 20 + 12
    let mut p = Printer::new(Box::new(t), &PTP300BT, "test");
    p.status().unwrap();
    p.print(&one_dot_label(3, 64), false, false).unwrap();
}

/// A block that does not carry the status header cannot be a status
/// reply; decoding its bytes as one would invent errors or a false
/// "printing completed".
#[test]
fn cube_garbage_while_waiting_is_an_error() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend([0u8; 32]);
    let (mut p, _) = cube_with_status(&replies);
    let err = p.print(&one_dot_label(3, 64), false, false).unwrap_err();
    assert!(err.0.contains("unexpected status reply"), "{err}");
}

#[test]
fn cube_error_during_print_fails() {
    let mut replies = status_reply(12, 0x01, 0, 0, 0).to_vec();
    replies.extend(status_reply(12, 0x01, 0x02, 0, 0x02));
    let (mut p, _) = cube_with_status(&replies);
    let err = p.print(&one_dot_label(3, 64), false, false).unwrap_err();
    assert!(err.0.contains("end of tape"), "{err}");
}

#[test]
fn h500_print_stream_is_exact() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PTH500, "test");
    p.status().unwrap();
    written.borrow_mut().clear();
    p.print(&one_dot_label(3, 64), false, false).unwrap();

    let mut blank = vec![0x47u8, 0x11, 0x00, 15];
    blank.extend([0u8; 16]);
    // end margin first: 174 dots = the 24.5 mm lead, for equal margins
    // (docs/pt-raster-h500-spec.md §6.1–6.2, §7)
    let mut expected = b"\x1b\x69\x64\xae\x00".to_vec();
    expected.extend(b"\x4d\x02");
    expected.extend(b"\x1b\x69\x52\x01"); // ESC i R raster mode; no ESC i z on H500
    expected.extend(expected_first_line(64));
    expected.extend(&blank); // H500 never gets the Z shortcut
    expected.extend(&blank);
    expected.extend(b"\x1a");
    assert_eq!(*written.borrow(), expected);
}

#[test]
fn pt18r_print_stream_is_exact() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PT18R, "test");
    let st = p.status().unwrap();
    assert_eq!(st.tape_px, 76);
    written.borrow_mut().clear();
    p.print(&one_dot_label(3, 76), false, false).unwrap();

    // the window is shifted +4 dots from head center (measured), so
    // the top-left dot of a 76-px label lands at pin 26+4+75 = 105
    let mut first = vec![0x47u8, 0x11, 0x00, 15];
    let mut raster = [0u8; 16];
    raster[15 - 105 / 8] |= 1 << (105 % 8);
    first.extend(raster);
    let mut blank = vec![0x47u8, 0x11, 0x00, 15];
    blank.extend([0u8; 16]);
    // end margin: 170 dots = the measured ~24 mm lead, for equal margins
    let mut expected = b"\x1b\x69\x64\xaa\x00".to_vec();
    expected.extend(b"\x4d\x02");
    expected.extend(b"\x1b\x69\x52\x01");
    expected.extend(&first);
    expected.extend(&blank);
    expected.extend(&blank);
    expected.extend(b"\x1a");
    assert_eq!(*written.borrow(), expected);
}

#[test]
fn h500_save_tape_uses_minimal_end_margin() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PTH500, "test");
    p.status().unwrap();
    written.borrow_mut().clear();
    p.print(&one_dot_label(3, 64), false, true).unwrap();
    let written = written.borrow();
    // the documented 14-dot (2 mm) minimum instead of the 24.5 mm lead
    assert!(written.windows(5).any(|w| w == b"\x1b\x69\x64\x0e\x00"));
    assert_eq!(written.last(), Some(&0x1a));
}

#[test]
fn h500_chain_keeps_zero_margin_and_page_open() {
    let (t, written) = FakeTransport::new(&status_reply(12, 0x01, 0, 0, 0));
    let mut p = Printer::new(Box::new(t), &PTH500, "test");
    p.status().unwrap();
    written.borrow_mut().clear();
    p.print(&one_dot_label(3, 64), true, false).unwrap();
    let written = written.borrow();
    assert!(written.windows(5).any(|w| w == b"\x1b\x69\x64\x00\x00"));
    assert_eq!(written.last(), Some(&0x0c));
}

#[test]
fn valid_tape_px_covers_both_models() {
    assert_eq!(PTP300BT.tape_px_for(12), 64);
    assert_eq!(PTH500.tape_px_for(12), 76);
    assert_eq!(PTH500.tape_px_for(24), 128);
    assert_eq!(PTP300BT.tape_px_for(24), 0);
}
