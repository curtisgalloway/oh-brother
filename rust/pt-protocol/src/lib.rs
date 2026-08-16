// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Brother P-touch raster protocol over USB and Bluetooth.
//!
//! A Rust port of oh-brother's `protocol.py`, which is the
//! hardware-verified reference: the byte streams produced here are
//! golden-tested to match it exactly. Two supported printers, one
//! command language:
//!
//! - PT-H500 over USB (rusb with vendored libusb). Per Brother's
//!   "PT-H500/P700/E500 Raster Command Reference".
//! - PT-P300BT (P-touch Cube) over Bluetooth SPP. On macOS the
//!   transport is a Swift IOBluetooth shim (swift/ptbt.swift) — the
//!   /dev/cu.* serial devices no longer dial the link on current
//!   macOS. Only usable from the process main thread.
//!
//! Both heads take 16-byte (128-dot) raster lines at 180 dpi; the Cube
//! can only darken the middle 64 dots.

use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod bt_macos;
mod usb;

#[cfg(target_os = "macos")]
pub use bt_macos::BtTransport;
pub use usb::UsbTransport;

pub const HEAD_PX: u32 = 128; // bits per raster line, both models
pub const DPI: u32 = 180;

// Trailing feed on the Cube in tape-saver mode, in dots: just enough
// (~2 mm) that the built-in cutter clears the print instead of
// cutting flush against it; the user trims the mechanical lead with
// scissors. The default mode instead feeds the model's lead_margin_mm
// so both margins come out equal (see write_print_info).
pub const SAVE_TAPE_MARGIN_PX: u16 = 14;

/// Any printer-side or transport failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A byte pipe to a printer. `read` is one bounded attempt returning
/// whatever arrived (possibly nothing); `drain` drops buffered input.
pub trait Transport {
    fn write(&mut self, data: &[u8]) -> Result<()>;
    fn read(&mut self, max: usize) -> Result<Vec<u8>>;
    fn drain(&mut self) -> Result<()>;
}

/// What differs between P-touch models that share the raster language.
pub struct ModelSpec {
    pub name: &'static str,
    // Printable pixels per tape width (mm, as reported in the status
    // reply). 3.5 mm tape reports itself as 4 mm.
    tape_px: &'static [(u8, u16)],
    // Blank tape mechanically fed before the first printed column of a
    // non-chained job: the head-to-cutter distance. Not reducible in
    // software — surfaced in the UI so users know it's the hardware.
    // PT-H500: 24.5 mm per the raster reference ("minimum length of
    // tape that can be fed out"); PT-P300BT: ~25 mm, measured.
    pub lead_margin_mm: f32,
    // The Cube needs ESC i a (command-set select) + ESC i z (print
    // information, with the raster-line count) before raster data; the
    // PT-H500 instead takes the bare ESC i R raster-mode switch.
    needs_print_info: bool,
    // The Cube aborts the job with an error blink if the host drops
    // the link before printing finishes (verified on hardware), so
    // printing must block until the "printing completed" status.
    confirm_print: bool,
    // Dots from the head's 128-dot center to the loaded tape's center,
    // positive = tape sits toward higher dot numbers. 0 on the H500
    // and Cube; the PT-18R's tape path runs ~4 dots high (measured on
    // hardware: a head-centered window printed with a ~1.5 mm top gap
    // and near-zero bottom gap; +4 evened the gaps out).
    tape_center_offset: i32,
}

impl ModelSpec {
    pub fn tape_px_for(&self, mm: u8) -> u16 {
        self.tape_px
            .iter()
            .find(|(w, _)| *w == mm)
            .map(|(_, px)| *px)
            .unwrap_or(0)
    }

    pub fn max_px(&self) -> u16 {
        self.tape_px.iter().map(|(_, px)| *px).max().unwrap()
    }

    /// (tape width mm, printable px) pairs, ascending.
    pub fn tape_table(&self) -> &'static [(u8, u16)] {
        self.tape_px
    }
}

pub static PTH500: ModelSpec = ModelSpec {
    name: "PT-H500",
    tape_px: &[(4, 24), (6, 32), (9, 52), (12, 76), (18, 120), (24, 128)],
    lead_margin_mm: 24.5,
    needs_print_info: false,
    confirm_print: false,
    tape_center_offset: 0,
};

pub static PTP300BT: ModelSpec = ModelSpec {
    name: "PT-P300BT",
    tape_px: &[(4, 24), (6, 32), (9, 52), (12, 64)],
    lead_margin_mm: 25.0,
    needs_print_info: true,
    confirm_print: true,
    tape_center_offset: 0,
};

// PT-18R (04f9:201a), brought up empirically 2026-08-15 — no raster
// reference exists for it, but it speaks the PT-H500 minimal dialect
// verbatim (ESC i S status, ESC i d end margin, M 02 + ESC i R +
// G-framed packbits, 0C/1A; chain suppresses its auto-cutter). 18 mm
// gets 112 px = the official spec sheet's 15.8 mm max print height;
// narrower widths reuse the family's tape-safe windows (the firmware
// does not clip — a full-head print runs past the tape edges).
pub static PT18R: ModelSpec = ModelSpec {
    name: "PT-18R",
    tape_px: &[(6, 32), (9, 52), (12, 76), (18, 112)],
    lead_margin_mm: 24.0, // measured ≈24 mm on hardware
    needs_print_info: false,
    confirm_print: false,
    tape_center_offset: 4,
};

const MEDIA_TYPES: &[(u8, &str)] = &[
    (0x00, "no tape"),
    (0x01, "laminated (TZe)"),
    (0x03, "non-laminated"),
    (0x11, "heat-shrink tube"),
    (0xFF, "incompatible tape"),
];

const ERRORS_BYTE8: &[(u8, &str)] = &[
    (0x01, "no tape"),
    (0x02, "end of tape"),
    (0x04, "cutter jam"),
    (0x08, "weak batteries"),
];

const ERRORS_BYTE9: &[(u8, &str)] = &[
    (0x01, "replace the tape"),
    (0x04, "communication error"),
    (0x10, "cover open"),
];

fn decode_errors(b8: u8, b9: u8) -> Vec<String> {
    let mut out = Vec::new();
    for (bit, name) in ERRORS_BYTE8 {
        if b8 & bit != 0 {
            out.push((*name).to_owned());
        }
    }
    for (bit, name) in ERRORS_BYTE9 {
        if b9 & bit != 0 {
            out.push((*name).to_owned());
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct TapeStatus {
    pub media_width_mm: u8,
    pub media_type: String,
    pub errors: Vec<String>,
    pub raw: [u8; 32],
    pub tape_px: u16,
}

/// A 1-bit label bitmap; rows map across the tape, columns along it.
pub struct Label {
    width: u32,
    height: u32,
    black: Vec<bool>, // row-major, y * width + x
}

impl Label {
    pub fn new(width: u32, height: u32, black: Vec<bool>) -> Label {
        assert_eq!(black.len(), (width * height) as usize);
        Label {
            width,
            height,
            black,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn is_black(&self, x: u32, y: u32) -> bool {
        self.black[(y * self.width + x) as usize]
    }
}

/// A P-touch printer: a transport plus the model quirks table.
pub struct Printer {
    transport: Box<dyn Transport>,
    pub spec: &'static ModelSpec,
    /// The id from `available_printers` this was opened as.
    pub printer_id: String,
    media_width_mm: Option<u8>,
}

impl Printer {
    pub fn new(
        transport: Box<dyn Transport>,
        spec: &'static ModelSpec,
        printer_id: &str,
    ) -> Printer {
        Printer {
            transport,
            spec,
            printer_id: printer_id.to_owned(),
            media_width_mm: None,
        }
    }

    pub fn model_name(&self) -> &'static str {
        self.spec.name
    }

    fn read_status_reply(&mut self, timeout: Duration) -> Result<[u8; 32]> {
        let deadline = Instant::now() + timeout;
        let mut buf = Vec::new();
        while Instant::now() < deadline {
            buf.extend(self.transport.read(32)?);
            if buf.len() >= 32 {
                return Ok(buf[..32].try_into().unwrap());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(Error("timed out waiting for a status reply".into()))
    }

    /// Initialize the printer and request tape status.
    pub fn status(&mut self) -> Result<TapeStatus> {
        self.transport.drain()?; // drop stale phase blocks from a previous job
        let mut init = vec![0u8; 100];
        init.extend(b"\x1b\x40"); // invalidate + ESC @ init
        self.transport.write(&init)?;
        if self.spec.needs_print_info {
            self.transport.write(b"\x1b\x69\x61\x01")?; // ESC i a: raster command set
        }
        self.transport.write(b"\x1b\x69\x53")?; // ESC i S: status request
        let buf = self.read_status_reply(Duration::from_secs(5))?;
        if buf[0] != 0x80 || buf[1] != 0x20 {
            return Err(Error(format!("unexpected status reply: {:02x?}", buf)));
        }
        self.media_width_mm = Some(buf[10]);
        let media_type = MEDIA_TYPES
            .iter()
            .find(|(b, _)| *b == buf[11])
            .map(|(_, name)| (*name).to_owned())
            .unwrap_or_else(|| format!("unknown (0x{:02x})", buf[11]));
        Ok(TapeStatus {
            media_width_mm: buf[10],
            media_type,
            errors: decode_errors(buf[8], buf[9]),
            raw: buf,
            tape_px: self.spec.tape_px_for(buf[10]),
        })
    }

    /// The Cube's per-job preamble: ESC i z + page modes + margin.
    fn write_print_info(&mut self, raster_lines: u32, chain: bool, save_tape: bool) -> Result<()> {
        let width_mm = match self.media_width_mm {
            Some(mm) => mm,
            None => {
                self.status()?;
                self.media_width_mm.unwrap()
            }
        };
        // ESC i z: valid-field flags (width | quality | recovery),
        // media type, width mm, length mm (0 = unspecified),
        // raster-line count, first-page flag, reserved.
        let mut params = vec![0x04 | 0x40 | 0x80, 0x01, width_mm, 0];
        params.extend(raster_lines.to_le_bytes());
        params.extend([0, 0]);
        let mut cmd = b"\x1b\x69\x7a".to_vec();
        cmd.extend(&params);
        self.transport.write(&cmd)?;
        // ESC i K advanced mode: bit 3 = no page chaining (feed after
        // this page). No cutter on this model, so ESC i M stays 0.
        let advanced: &[u8] = if chain { b"\x00" } else { b"\x08" };
        self.transport
            .write(&[b"\x1b\x69\x4b" as &[u8], advanced].concat())?;
        self.transport.write(b"\x1b\x69\x4d\x00")?; // ESC i M: various mode
        self.write_end_margin(chain, save_tape)?;
        Ok(())
    }

    /// ESC i d: the end margin — how far the tape feeds past the last
    /// printed column (u16 LE dots; docs/pt-raster-h500-spec.md §5.8,
    /// §6.1). With 0 the cutter lands flush on the print; the leading
    /// blank is mechanical (head-to-cutter distance) and not
    /// controllable from here. Default: feed the same amount so the
    /// margins come out equal; save_tape feeds just ~2 mm (the
    /// documented 14-dot minimum). Chained pages keep 0 so consecutive
    /// labels stay flush.
    fn write_end_margin(&mut self, chain: bool, save_tape: bool) -> Result<()> {
        let margin: u16 = if chain {
            0
        } else if save_tape {
            SAVE_TAPE_MARGIN_PX
        } else {
            (f64::from(self.spec.lead_margin_mm) / 25.4 * f64::from(DPI)).round() as u16
        };
        let mut cmd = b"\x1b\x69\x64".to_vec();
        cmd.extend(margin.to_le_bytes());
        self.transport.write(&cmd)?;
        Ok(())
    }

    /// Print a label; the image must already fit the loaded tape.
    /// save_tape trades the default equal trailing margin for a ~2 mm
    /// one (trim the mechanical lead with scissors).
    pub fn print(&mut self, label: &Label, chain: bool, save_tape: bool) -> Result<()> {
        let height = label.height();
        let max_px = self.spec.max_px() as u32;
        if height > max_px {
            return Err(Error(format!(
                "image height {} exceeds the {} head ({} px)",
                height, self.spec.name, max_px
            )));
        }

        if self.spec.needs_print_info {
            self.write_print_info(label.width(), chain, save_tape)?;
            self.transport.write(b"\x4d\x02")?; // M 0x02: packbits compression
        } else {
            // The PT-H500 honors the same ESC i d margin; per the
            // databook's sequence, control codes precede the
            // compression select (docs/pt-raster-h500-spec.md §7).
            self.write_end_margin(chain, save_tape)?;
            self.transport.write(b"\x4d\x02")?;
            self.transport.write(b"\x1b\x69\x52\x01")?; // ESC i R: raster transfer mode
        }

        // The printable window sits centered in the 128-bit line
        // (the Cube's head covers only the middle 64 dots), shifted by
        // the model's measured tape-center offset.
        let centered = ((HEAD_PX - height) / 2) as i32 + self.spec.tape_center_offset;
        let offset = centered.clamp(0, (HEAD_PX - height) as i32) as u32;
        let line_bytes = (HEAD_PX / 8) as usize;
        for x in 0..label.width() {
            let mut raster = vec![0u8; line_bytes];
            for i in 0..height {
                if label.is_black(x, height - 1 - i) {
                    let p = (offset + i) as usize;
                    raster[(line_bytes - 1) - (p / 8)] |= 1 << (p % 8);
                }
            }
            if self.spec.needs_print_info && raster.iter().all(|b| *b == 0) {
                // Z (zero raster line) is verified on the Cube; over
                // its slow RFCOMM link the shortcut is worth having.
                // The PT-H500 keeps getting explicit lines as always.
                self.transport.write(b"\x5a")?;
                continue;
            }
            // G <len16 LE> then a single uncompressed packbits run
            let mut frame = vec![0x47u8];
            let payload_len = (line_bytes + 1) as u16;
            frame.extend(payload_len.to_le_bytes());
            frame.push((line_bytes - 1) as u8);
            frame.extend(&raster);
            self.transport.write(&frame)?;
        }

        // 0x0C prints and leaves the tape (chaining); 0x1A prints and feeds
        self.transport
            .write(if chain { b"\x0c" } else { b"\x1a" })?;
        if self.spec.confirm_print {
            self.wait_print_done(Duration::from_secs(30))?;
        }
        Ok(())
    }

    /// Block until the printer reports the page printed. Closing the
    /// connection before "printing completed" makes the Cube abort the
    /// job into an error state.
    fn wait_print_done(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error("the printer never confirmed the print".into()));
            }
            let buf = self.read_status_reply(remaining).map_err(|_| {
                Error("the printer went silent instead of confirming the print".into())
            })?;
            let errors = decode_errors(buf[8], buf[9]);
            if !errors.is_empty() {
                return Err(Error(format!("printer reports: {}", errors.join(", "))));
            }
            if buf[18] == 0x01 {
                return Ok(()); // status type: printing completed
            }
        }
    }
}

/// One entry per reachable/paired printer; `id` feeds `find_printer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterInfo {
    pub id: String,
    pub label: String,
}

/// Enumerate reachable/paired printers without opening connections.
pub fn available_printers() -> Vec<PrinterInfo> {
    let mut out = Vec::new();
    for spec in usb::present_models() {
        out.push(PrinterInfo {
            id: spec.name.into(),
            label: format!("{} (USB)", spec.name),
        });
    }
    #[cfg(target_os = "macos")]
    for name in bt_macos::paired_cubes() {
        out.push(PrinterInfo {
            label: format!("{name} (Bluetooth)"),
            id: name,
        });
    }
    out
}

/// Open a printer. `printer_id` (an id from `available_printers`)
/// picks one; with `None` the first supported USB printer wins, then
/// the first paired Cube.
pub fn find_printer(printer_id: Option<&str>) -> Result<Printer> {
    if printer_id.is_none() || printer_id.is_some_and(usb::is_usb_model) {
        if let Some((t, spec)) = usb::UsbTransport::find(printer_id)? {
            return Ok(Printer::new(Box::new(t), spec, spec.name));
        }
        if let Some(id) = printer_id {
            return Err(Error(format!(
                "no {id} found on USB; is it plugged in and turned on?"
            )));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let t = bt_macos::BtTransport::open(printer_id)?;
        let name = t.device_name.clone();
        Ok(Printer::new(Box::new(t), &PTP300BT, &name))
    }
    #[cfg(not(target_os = "macos"))]
    Err(Error(
        "no printer found: no supported USB printer (PT-H500, PT-18R), and \
         Bluetooth support for this platform is not ported yet"
            .into(),
    ))
}

#[cfg(test)]
mod tests;
