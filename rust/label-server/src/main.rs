// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! `label-web` — serve the label editor UI (Rust port).

use clap::Parser;

#[derive(Parser)]
#[command(name = "label-web")]
struct Args {
    #[arg(long, default_value_t = 8763)]
    port: u16,

    #[arg(long)]
    no_browser: bool,

    /// Export the web app + all fonts + licenses into DIR, ready for
    /// any static HTTPS host (WebUSB printing, no server), then exit.
    #[arg(long, value_name = "DIR")]
    export_static: Option<std::path::PathBuf>,
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    if let Some(outdir) = args.export_static {
        return match label_server::export_static(&outdir) {
            Ok((shipped, failures)) => {
                println!("{}: 4 page files, {shipped} fonts", outdir.display());
                if failures.is_empty() {
                    std::process::ExitCode::SUCCESS
                } else {
                    eprintln!("export incomplete:\n  {}", failures.join("\n  "));
                    std::process::ExitCode::FAILURE
                }
            }
            Err(e) => {
                eprintln!("export failed: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    if !args.no_browser {
        let url = format!("http://127.0.0.1:{}/", args.port);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            let _ = webbrowser::open(&url);
        });
    }
    match label_server::serve(args.port) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::ExitCode::FAILURE
        }
    }
}
