// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! Oh, Brother — native shell around the label-web UI (the Windows
//! counterpart of macos/main.swift, same thin-shell architecture):
//! spawn the bundled `label-web` binary next to this executable
//! (attach instead if a server is already running; only terminate
//! what we spawned), wait for readiness, show the UI in a webview.
//! The Tools menu installs a `label.cmd` PATH shim (Windows) so
//! `label` works from any shell — and `label --skill` teaches AI
//! agents to drive the printer.
//!
//! Builds and runs on macOS/Linux too for development (the Mac app
//! proper is macos/main.swift); the PATH installer is Windows-only.
//!
//! STATUS: not yet exercised on real Windows — see windows/TESTING.md.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use muda::{Menu, MenuEvent, MenuItem, Submenu};
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

const PORT: u16 = 8763;

const SPLASH: &str = r#"<!doctype html><html><head><meta charset="utf-8"><style>
  body { background:#141517; color:#8b8e96; font: 15px "Segoe UI", sans-serif;
         display:flex; align-items:center; justify-content:center; height:100vh; margin:0; }
  .msg { text-align:center; }
  .msg b { color:#ffcf24; letter-spacing:.1em; text-transform:uppercase; }
</style></head><body><div class="msg"><b>Oh, Brother</b><br><br>starting the label engine&hellip;</div></body></html>"#;

fn app_url() -> String {
    format!("http://127.0.0.1:{PORT}/")
}

fn server_alive() -> bool {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(1))
        .build()
        .get(&format!("http://127.0.0.1:{PORT}/api/meta"))
        .call()
        .is_ok()
}

fn alert(message: &str) {
    rfd::MessageDialog::new()
        .set_title("Oh, Brother")
        .set_description(message)
        .show();
}

/// The bundled sibling binary (label-web / label next to this exe);
/// in a dev `cargo run` that's the same target directory.
fn sibling(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let path = dir.join(file);
    path.exists().then_some(path)
}

fn spawn_server() -> Option<Child> {
    let Some(web) = sibling("label-web") else {
        alert(
            "Can't find label-web next to the app.\n\
             Rebuild with windows\\build.ps1 (or cargo build --release).",
        );
        return None;
    };
    let mut cmd = Command::new(web);
    cmd.args(["--no-browser", "--port", &PORT.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        // A windowed app must not flash a console for its child.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            alert(&format!("Couldn't start the label server:\n{e}"));
            None
        }
    }
}

/// Windows: write %LOCALAPPDATA%\oh-brother\bin\label.cmd running the
/// bundled label.exe, and put that dir on the per-user PATH.
#[cfg(windows)]
fn install_cli() -> Result<String, String> {
    let label = sibling("label").ok_or("This build has no bundled label.exe.")?;
    let local = std::env::var("LOCALAPPDATA").map_err(|_| "LOCALAPPDATA is not set")?;
    let bin_dir = PathBuf::from(local).join("oh-brother").join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let shim = bin_dir.join("label.cmd");
    std::fs::write(
        &shim,
        format!("@echo off\r\n\"{}\" %*\r\n", label.display()),
    )
    .map_err(|e| e.to_string())?;
    let changed = ensure_user_path(&bin_dir.display().to_string()).map_err(|e| e.to_string())?;
    let note = if changed {
        "PATH updated — open a new terminal."
    } else {
        "Already on PATH."
    };
    Ok(format!(
        "Installed {}\n{note}\nTry `label --help`; AI agents can run `label --skill`.",
        shim.display()
    ))
}

#[cfg(not(windows))]
fn install_cli() -> Result<String, String> {
    Err("The PATH installer is Windows-only; on macOS use Oh Brother.app.".into())
}

/// Append entry to the per-user PATH if missing; true if changed.
/// Mirrors the registry + WM_SETTINGCHANGE dance so new terminals see
/// the new PATH without a logout.
#[cfg(windows)]
fn ensure_user_path(entry: &str) -> std::io::Result<bool> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};

    let env = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;
    let current: String = env.get_value("Path").unwrap_or_default();
    let has = current
        .split(';')
        .any(|p| p.trim().eq_ignore_ascii_case(entry));
    if has {
        return Ok(false);
    }
    let mut parts: Vec<&str> = current.split(';').filter(|p| !p.is_empty()).collect();
    parts.push(entry);
    let joined = parts.join(";");
    // Preserve the expandable type so %VAR% entries keep working.
    let mut bytes: Vec<u8> = Vec::new();
    for unit in joined.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend(unit.to_le_bytes());
    }
    env.set_raw_value(
        "Path",
        &RegValue {
            vtype: REG_EXPAND_SZ,
            bytes,
        },
    )?;
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        let env_w: Vec<u16> = "Environment\0".encode_utf16().collect();
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            env_w.as_ptr() as _,
            SMTO_ABORTIFHUNG,
            5000,
            std::ptr::null_mut(),
        );
    }
    Ok(true)
}

#[derive(Debug)]
enum UserEvent {
    ServerReady,
    ServerTimeout,
}

fn main() {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let menu = Menu::new();
    let tools = Submenu::new("Tools", true);
    let install_item = MenuItem::new("Install 'label' Command in PATH", true, None);
    tools.append(&install_item).expect("menu append");
    menu.append(&tools).expect("menu append");

    let window = WindowBuilder::new()
        .with_title("Oh, Brother")
        .with_inner_size(tao::dpi::LogicalSize::new(980.0, 780.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0))
        .build(&event_loop)
        .expect("window");

    #[cfg(windows)]
    unsafe {
        use tao::platform::windows::WindowExtWindows;
        menu.init_for_hwnd(window.hwnd() as _).expect("menu attach");
    }
    #[cfg(target_os = "macos")]
    menu.init_for_nsapp();

    let webview = WebViewBuilder::new()
        .with_html(SPLASH)
        .build(&window)
        .expect("webview");

    let mut spawned: Option<Child> = None;
    if server_alive() {
        webview.load_url(&app_url()).ok();
    } else {
        spawned = spawn_server();
        let proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if server_alive() {
                    let _ = proxy.send_event(UserEvent::ServerReady);
                    return;
                }
                if Instant::now() >= deadline {
                    let _ = proxy.send_event(UserEvent::ServerTimeout);
                    return;
                }
                std::thread::sleep(Duration::from_millis(400));
            }
        });
    }

    let menu_rx = MenuEvent::receiver();
    let install_id = install_item.id().clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Ok(menu_event) = menu_rx.try_recv() {
            if menu_event.id == install_id {
                match install_cli() {
                    Ok(msg) => alert(&msg),
                    Err(e) => alert(&format!("Couldn't install the CLI shim:\n{e}")),
                }
            }
        }

        match event {
            Event::UserEvent(UserEvent::ServerReady) => {
                webview.load_url(&app_url()).ok();
            }
            Event::UserEvent(UserEvent::ServerTimeout) => {
                alert(
                    "The label server didn't come up within 20 seconds.\n\
                     Try running label-web by hand to see why.",
                );
            }
            Event::WindowEvent {
                event: tao::event::WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(child) = spawned.as_mut() {
                    let _ = child.kill();
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
