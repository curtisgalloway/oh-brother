// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! `label` — the Rust CLI: printer discovery, tape status, text / QR /
//! Code 128 / image printing, and font cache management, mirroring
//! cli.py flag for flag.

use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use image::GrayImage;
use label_render::{hstack, load_image, render_code128, render_label, render_qr, TextOptions};
use pt_protocol::{available_printers, find_printer, Label, Printer, TapeStatus, DPI, PTH500};

#[derive(Parser)]
#[command(
    name = "label",
    about = "Print a label on a Brother P-touch (PT-H500 on USB, or a \
             paired PT-P300BT Cube over Bluetooth). Embedded \\n in TEXT \
             starts a new line on the label."
)]
struct Args {
    /// Text to print
    text: Option<String>,

    /// Add a QR code (left of any text)
    #[arg(long, value_name = "DATA")]
    qr: Option<String>,

    /// Add a Code 128 barcode (left of any text)
    #[arg(long, value_name = "DATA")]
    code: Option<String>,

    /// Print an image file instead
    #[arg(long, value_name = "FILE")]
    image: Option<String>,

    /// Font shortcut (helvetica, menlo, ...) or path
    #[arg(long)]
    font: Option<String>,

    /// Fixed font size in px (default: fit tape)
    #[arg(long)]
    size: Option<u32>,

    /// Side margins in px (default 8)
    #[arg(long, default_value_t = 8)]
    margin: u32,

    /// Horizontal stretch in percent, e.g. 150 (default 100)
    #[arg(long, value_name = "PCT", default_value_t = 100)]
    width: u32,

    /// Number of copies
    #[arg(long, default_value_t = 1)]
    copies: u32,

    /// Do not feed after printing; saves tape between consecutive labels
    #[arg(long)]
    chain: bool,

    /// Feed only ~2 mm after the label instead of matching the ~25 mm
    /// mechanical lead margin (trim the lead with scissors)
    #[arg(long)]
    save_tape: bool,

    /// Render to a PNG and open it instead of printing
    #[arg(long)]
    preview: bool,

    /// Assumed tape width for --preview when no printer is reachable
    #[arg(long, value_name = "MM", default_value_t = 12, value_parser = parse_tape_mm)]
    tape_mm: u8,

    /// Print to a specific printer (an id from --printers);
    /// default: PT-H500 on USB, else the first Bluetooth Cube
    #[arg(long, value_name = "ID")]
    printer: Option<String>,

    /// List connected/paired printers and exit
    #[arg(long)]
    printers: bool,

    /// Show printer/tape status
    #[arg(long)]
    status: bool,

    /// Print the usage guide for AI agents (workflow, examples,
    /// printer gotchas) and exit
    #[arg(long)]
    skill: bool,

    /// List available fonts and exit
    #[arg(long)]
    fonts: bool,

    /// Download all fonts into the local cache (for offline use)
    #[arg(long)]
    fetch_fonts: bool,
}

const SKILL_MD: &str = include_str!("../../../SKILL.md");

/// Validate --tape-mm at parse time like argparse's `choices` does
/// (bad values exit 2 in every mode, even --status).
fn parse_tape_mm(s: &str) -> Result<u8, String> {
    let widths = || {
        PTH500
            .tape_table()
            .iter()
            .map(|(mm, _)| mm.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mm: u8 = s
        .parse()
        .map_err(|_| format!("invalid choice: {s} (choose from {})", widths()))?;
    if PTH500.tape_px_for(mm) == 0 {
        return Err(format!("invalid choice: {mm} (choose from {})", widths()));
    }
    Ok(mm)
}

/// Mirror of cli._list_fonts.
fn list_fonts() {
    let cache = label_render::fontcache::global();
    for font in cache.visible_fonts() {
        let id = font["id"].as_str().unwrap_or_default();
        let category = font["category"].as_str().unwrap_or_default();
        let cached = if cache.path_for(id).is_some() {
            "cached"
        } else {
            "      "
        };
        let aliases: Vec<&str> = font
            .get("aliases")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let alias_col = if aliases.is_empty() {
            String::new()
        } else {
            format!("  ({})", aliases.join(","))
        };
        println!("{id:<20} {category:<12} {cached}{alias_col}");
    }
}

/// Open the printer and check the tape, mirroring cli._tape_status.
fn tape_status(printer_id: Option<&str>) -> Result<(Printer, TapeStatus), String> {
    let mut printer = find_printer(printer_id).map_err(|e| e.0)?;
    let st = printer.status().map_err(|e| e.0)?;
    if !st.errors.is_empty() {
        return Err(format!("printer reports: {}", st.errors.join(", ")));
    }
    if st.tape_px == 0 {
        return Err(format!("unsupported tape width {} mm", st.media_width_mm));
    }
    Ok((printer, st))
}

fn to_label(img: &GrayImage) -> Label {
    Label::new(
        img.width(),
        img.height(),
        img.pixels().map(|p| p.0[0] < 128).collect(),
    )
}

fn open_file(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    let _ = cmd.status();
}

fn run(args: &Args) -> Result<(), String> {
    if args.skill {
        print!("{SKILL_MD}");
        return Ok(());
    }
    if args.fonts {
        list_fonts();
        return Ok(());
    }
    if args.fetch_fonts {
        let failed = label_render::fontcache::global().ensure_all(|line| println!("{line}"));
        if !failed.is_empty() {
            return Err(format!("failed to fetch: {}", failed.join(", ")));
        }
        return Ok(());
    }

    if args.printers {
        let printers = available_printers();
        if printers.is_empty() {
            return Err("no printers found (USB unplugged, nothing paired)".into());
        }
        for p in printers {
            println!("{:<24} {}", p.id, p.label);
        }
        return Ok(());
    }

    let printer_id = args.printer.as_deref();

    if args.status {
        let (printer, st) = tape_status(printer_id)?;
        println!(
            "{}: {} mm {}, {} px printable, {} mm lead margin (hardware)",
            printer.model_name(),
            st.media_width_mm,
            st.media_type,
            st.tape_px,
            printer.spec.lead_margin_mm
        );
        return Ok(());
    }

    if args.text.is_none() && args.qr.is_none() && args.code.is_none() && args.image.is_none() {
        Args::command().print_help().map_err(|e| e.to_string())?;
        std::process::exit(2);
    }

    let mut printer = None;
    let height = if args.preview {
        match tape_status(printer_id) {
            Ok((_, st)) => st.tape_px,
            // parse_tape_mm validated the width, so this never misses.
            Err(_) => PTH500.tape_px_for(args.tape_mm),
        }
    } else {
        let (p, st) = tape_status(printer_id)?;
        printer = Some(p);
        st.tape_px
    };
    let height = u32::from(height);

    let mut parts: Vec<GrayImage> = Vec::new();
    if let Some(path) = &args.image {
        parts.push(load_image(path, height).map_err(|e| e.0)?);
    }
    if let Some(data) = &args.qr {
        parts.push(render_qr(data, height).map_err(|e| e.0)?);
    }
    if let Some(data) = &args.code {
        parts.push(render_code128(data, height, args.font.as_deref()).map_err(|e| e.0)?);
    }
    if let Some(text) = &args.text {
        parts.push(
            render_label(
                &text.replace("\\n", "\n"),
                height,
                &TextOptions {
                    font: args.font.clone(),
                    size: args.size,
                    margin_px: args.margin,
                    hscale: f64::from(args.width) / 100.0,
                    ..TextOptions::default()
                },
            )
            .map_err(|e| e.0)?,
        );
    }
    let img = if parts.len() == 1 {
        parts.pop().unwrap()
    } else {
        hstack(&parts, height, 8)
    };

    if args.preview {
        let scale = 4;
        let big = image::imageops::resize(
            &img,
            img.width() * scale,
            img.height() * scale,
            image::imageops::FilterType::Nearest,
        );
        let file = tempfile::Builder::new()
            .prefix("label-")
            .suffix(".png")
            .tempfile()
            .map_err(|e| format!("cannot create preview file: {e}"))?;
        let (_, path) = file
            .keep()
            .map_err(|e| format!("cannot keep preview file: {e}"))?;
        big.save(&path)
            .map_err(|e| format!("cannot save preview: {e}"))?;
        println!("{}x{} px -> {}", img.width(), img.height(), path.display());
        open_file(&path);
        return Ok(());
    }

    let mut printer = printer.expect("printer opened for the non-preview path");
    let label = to_label(&img);
    for copy in 0..args.copies {
        let last = copy == args.copies - 1;
        printer
            .print(&label, args.chain || !last, args.save_tape)
            .map_err(|e| e.0)?;
    }
    let mm = (f64::from(label.width()) / f64::from(DPI) * 25.4).round();
    let copies_note = if args.copies > 1 {
        format!(" x{}", args.copies)
    } else {
        String::new()
    };
    println!(
        "printed {}x{} px (~{mm} mm of tape){copies_note}",
        label.width(),
        label.height()
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}
