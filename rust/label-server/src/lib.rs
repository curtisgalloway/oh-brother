// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

//! `label-web`, in Rust: serves the (embedded) static label editor UI
//! and the printer API. Response shapes mirror web.py so the existing
//! browser code works unchanged.
//!
//! Threading mirrors web.py's constraint: the macOS Bluetooth
//! transport only works on the process main thread, so `serve` parks
//! the calling (main) thread in a printer-job loop and runs the tokio
//! runtime on a background thread. `/api/render` and `/api/print` use
//! the label-render crate (the render.py port); the web UI still
//! renders in the browser and prints via `/api/print-raw`.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use image::GrayImage;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use label_render::{fontcache, hostfonts, render_label, TextOptions};
use pt_protocol::{available_printers, find_printer, Label, DPI, PTH500, PTP300BT};

const INDEX_HTML: &[u8] = include_bytes!("../../../static/index.html");
const LABELCORE_JS: &[u8] = include_bytes!("../../../static/labelcore.js");
const QRCODE_JS: &[u8] = include_bytes!("../../../static/qrcode.js");
const FONTS_JSON: &[u8] = fontcache::MANIFEST_BYTES;

/// Build a self-contained static deploy: the embedded page files plus
/// every manifest font under fonts/<id>.ttf with its license text
/// under fonts/licenses/ (the OFL/Apache terms require the license to
/// accompany redistribution — a font whose license can't be fetched is
/// not shipped). The output serves from any static HTTPS host and
/// prints over WebUSB. Returns (fonts shipped, failure messages).
pub fn export_static(outdir: &std::path::Path) -> std::io::Result<(usize, Vec<String>)> {
    let fonts_dir = outdir.join("fonts");
    let lic_dir = fonts_dir.join("licenses");
    std::fs::create_dir_all(&lic_dir)?;
    for (name, bytes) in [
        ("index.html", INDEX_HTML),
        ("labelcore.js", LABELCORE_JS),
        ("qrcode.js", QRCODE_JS),
        ("fonts.json", FONTS_JSON),
    ] {
        std::fs::write(outdir.join(name), bytes)?;
    }
    let cache = fontcache::global();
    let mut shipped = 0;
    let mut failures = Vec::new();
    for entry in cache.fonts() {
        let id = entry["id"].as_str().unwrap_or_default();
        let font = match cache.ensure(id) {
            Ok(path) => path,
            Err(e) => {
                failures.push(format!("{id}: {e}"));
                continue;
            }
        };
        let lic = match cache.license_path_strict(id) {
            Ok(path) => path,
            Err(e) => {
                failures.push(format!("{id}: {e} — not shipping"));
                continue;
            }
        };
        std::fs::copy(&font, fonts_dir.join(format!("{id}.ttf")))?;
        std::fs::copy(&lic, lic_dir.join(format!("{id}.txt")))?;
        shipped += 1;
    }
    Ok((shipped, failures))
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Clone)]
struct AppState {
    jobs: mpsc::UnboundedSender<Job>,
}

/// Run a closure on the printer thread (the process main thread) and
/// await its result. The single queue also serializes printer access.
async fn run_printer_job<T, F>(state: &AppState, f: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    state
        .jobs
        .send(Box::new(move || {
            let _ = tx.send(f());
        }))
        .expect("printer job loop gone");
    rx.await.expect("printer job dropped")
}

fn static_response(content_type: &'static str, body: &'static [u8]) -> Response {
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}

async fn index() -> Response {
    static_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn labelcore_js() -> Response {
    static_response("text/javascript", LABELCORE_JS)
}

async fn qrcode_js() -> Response {
    static_response("text/javascript", QRCODE_JS)
}

async fn fonts_json() -> Response {
    static_response("application/json", FONTS_JSON)
}

async fn font_file(Path(file): Path<String>) -> Response {
    let Some(id) = file.strip_suffix(".ttf") else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such font"})),
        )
            .into_response();
    };
    let cache = fontcache::global();
    if cache.font_by_id(id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no such font"})),
        )
            .into_response();
    }
    // Fetch on demand (60 s worst case) — off the async workers.
    let id = id.to_owned();
    let ensured = tokio::task::spawn_blocking(move || fontcache::global().ensure(&id))
        .await
        .expect("font fetch task");
    match ensured {
        Ok(path) => match std::fs::read(&path) {
            Ok(bytes) => ([(header::CONTENT_TYPE, "font/ttf")], bytes).into_response(),
            Err(e) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": format!("could not read cached font: {e}")})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn api_meta() -> Json<Value> {
    let cache = fontcache::global();
    let fonts: Vec<Value> = cache
        .fonts()
        .iter()
        .filter(|f| f.get("hidden").and_then(Value::as_bool) != Some(true))
        .map(|f| {
            json!({
                "id": f["id"],
                "family": f["family"],
                "category": f["category"],
                "tags": f.get("tags").cloned().unwrap_or_else(|| json!([])),
                "aliases": f.get("aliases").cloned().unwrap_or_else(|| json!([])),
                "cached": cache.path_for(f["id"].as_str().unwrap_or_default()).is_some(),
            })
        })
        .collect();
    let tape_widths: Value = PTH500
        .tape_table()
        .iter()
        .map(|(mm, px)| (mm.to_string(), json!(px)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let host_fonts: Vec<Value> = hostfonts::list_families()
        .into_iter()
        .map(|f| json!({"family": f.family, "path": f.spec, "scripts": f.scripts}))
        .collect();
    Json(json!({
        "fonts": fonts,
        "host_fonts": host_fonts,
        "default_font": cache.manifest["default_font"],
        "samples": cache.manifest["samples"],
        "tape_widths": tape_widths,
        "dpi": DPI,
    }))
}

fn printers_json() -> Vec<Value> {
    available_printers()
        .into_iter()
        .map(|p| json!({"id": p.id, "label": p.label}))
        .collect()
}

async fn api_status(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let printer_id = params.get("printer").filter(|s| !s.is_empty()).cloned();
    let (result, printers) = run_printer_job(&state, move || {
        let printers = printers_json();
        let result = find_printer(printer_id.as_deref()).and_then(|mut printer| {
            let st = printer.status()?;
            Ok((
                st,
                printer.model_name(),
                printer.printer_id.clone(),
                printer.spec.lead_margin_mm,
            ))
        });
        (result, printers)
    })
    .await;
    match result {
        Ok((st, model, used_id, lead_mm)) => Json(json!({
            "connected": true,
            "model": model,
            "printer_id": used_id,
            "printers": printers,
            "tape_mm": st.media_width_mm,
            "tape_px": st.tape_px,
            "media": st.media_type,
            "errors": st.errors,
            "lead_mm": lead_mm,
        })),
        Err(e) => Json(json!({
            "connected": false,
            "error": e.0,
            "printers": printers,
        })),
    }
}

fn decode_png_label(data_url: &str) -> Result<image::GrayImage, String> {
    let b64 = data_url
        .split_once(',')
        .map(|(_, b)| b)
        .ok_or("bad png payload")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| "bad png payload")?;
    let img = image::load_from_memory(&bytes).map_err(|_| "bad png payload")?;
    Ok(img.to_luma8())
}

async fn api_print_raw(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let data = match parse_json(&body) {
        Ok(data) => data,
        Err(e) => return bad_request(e),
    };
    let copies = match data.get("copies") {
        Some(value) => match as_int(value) {
            Ok(copies) => copies.max(1) as u32,
            Err(e) => return bad_request(e),
        },
        None => 1,
    };
    let chain = truthy(data.get("chain"));
    let save_tape = truthy(data.get("save_tape"));
    let gray = match data
        .get("png")
        .and_then(Value::as_str)
        .ok_or_else(|| "bad png payload".to_owned())
        .and_then(decode_png_label)
    {
        Ok(g) => g,
        Err(e) => return bad_request(e),
    };
    let (width, height) = (gray.width(), gray.height());
    let label = Label::new(width, height, gray.pixels().map(|p| p.0[0] < 128).collect());
    let printer_id = printer_field(&data);

    // Errors are (status code, message); 400 for a client-fixable
    // mismatch, 503 for printer trouble — mirroring web.py.
    let result: Result<(), (u16, String)> = run_printer_job(&state, move || {
        let mut printer = find_printer(printer_id.as_deref()).map_err(|e| (503, e.0))?;
        let st = printer.status().map_err(|e| (503, e.0))?;
        if !st.errors.is_empty() {
            return Err((503, format!("printer reports: {}", st.errors.join(", "))));
        }
        if st.tape_px == 0 {
            return Err((
                503,
                format!("unsupported tape width {} mm", st.media_width_mm),
            ));
        }
        if height != u32::from(st.tape_px) {
            return Err((
                400,
                format!(
                    "label is {} px but tape is {} px — re-render",
                    height, st.tape_px
                ),
            ));
        }
        for copy in 0..copies {
            let last = copy == copies - 1;
            printer
                .print(&label, chain || !last, save_tape)
                .map_err(|e| (503, e.0))?;
        }
        Ok(())
    })
    .await;

    match result {
        Ok(()) => {
            let mm = (f64::from(width) / f64::from(DPI) * 25.4 * 10.0).round() / 10.0;
            Json(json!({"ok": true, "mm": mm, "copies": copies})).into_response()
        }
        Err((code, message)) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            Json(json!({"error": message})),
        )
            .into_response(),
    }
}

// ---- /api/render + /api/print: the Pillow-rendered HTTP API for ----
// ---- scripts, mirroring web.py's request coercions and shapes.  ----

/// Python `int(value)`: numbers truncate, strings parse, bools count.
fn as_int(value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| format!("bad number {n}")),
        Value::String(s) => s
            .trim()
            .parse()
            .map_err(|_| format!("invalid integer {s:?}")),
        Value::Bool(b) => Ok(i64::from(*b)),
        other => Err(format!("expected an integer, got {other}")),
    }
}

/// Python `float(value)`.
fn as_float(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("bad number {n}")),
        Value::String(s) => s
            .trim()
            .parse()
            .map_err(|_| format!("invalid number {s:?}")),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        other => Err(format!("expected a number, got {other}")),
    }
}

/// Parse a request body like Flask's get_json(force=True): the
/// Content-Type header plays no part, so bare `curl -d` posts work
/// exactly as they do against web.py.
fn parse_json(body: &axum::body::Bytes) -> std::result::Result<Value, String> {
    serde_json::from_slice(body).map_err(|e| format!("invalid JSON body: {e}"))
}

/// web.py's `data.get("printer") or None`: falsy values mean "default
/// printer"; a truthy non-string flows through as an id that matches
/// no printer, so the request fails instead of silently printing on a
/// printer the client did not select.
fn printer_field(data: &Value) -> Option<String> {
    let value = data.get("printer")?;
    if !truthy(Some(value)) {
        return None;
    }
    Some(match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

/// Python truthiness, for `if data.get(...)` guards.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// web.py's _render_from_request: pull the text/font/size/margin/
/// hscale fields out of the request and render. Errors map to 400.
fn render_from_request(data: &Value, tape_px: u32) -> Result<GrayImage, String> {
    let text = data
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim_end_matches('\n');
    // Python's str.strip() also treats U+001C..U+001F as whitespace.
    if text
        .trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
        .is_empty()
    {
        return Err("empty label".into());
    }
    let font = data
        .get("font")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let size = if truthy(data.get("size")) {
        let size = as_int(&data["size"])?;
        Some(u32::try_from(size).map_err(|_| format!("bad size {size}"))?)
    } else {
        None
    };
    let margin = match data.get("margin") {
        Some(value) => as_int(value)?,
        None => 8,
    };
    let margin = u32::try_from(margin).map_err(|_| format!("bad margin {margin}"))?;
    let hscale = match data.get("hscale") {
        Some(value) => as_float(value)?,
        None => 1.0,
    };
    render_label(
        text,
        tape_px,
        &TextOptions {
            font,
            size,
            margin_px: margin,
            hscale,
            ..TextOptions::default()
        },
    )
    .map_err(|e| e.0)
}

fn png_data_url(img: &GrayImage) -> String {
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("in-memory PNG encode");
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

fn mm_len(width: u32) -> f64 {
    (f64::from(width) / f64::from(DPI) * 25.4 * 10.0).round() / 10.0
}

/// Every printable height any supported model can report
/// (protocol.VALID_TAPE_PX).
fn valid_tape_px(px: i64) -> bool {
    PTH500
        .tape_table()
        .iter()
        .chain(PTP300BT.tape_table())
        .any(|&(_, valid)| i64::from(valid) == px)
}

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response()
}

async fn api_render(body: axum::body::Bytes) -> Response {
    let data = match parse_json(&body) {
        Ok(data) => data,
        Err(e) => return bad_request(e),
    };
    let tape_px = match data.get("tape_px") {
        Some(value) => match as_int(value) {
            Ok(px) => px,
            Err(e) => return bad_request(e),
        },
        None => 76,
    };
    if !valid_tape_px(tape_px) {
        return bad_request(format!("bad tape_px {tape_px}"));
    }
    match render_from_request(&data, tape_px as u32) {
        Ok(img) => Json(json!({
            "png": png_data_url(&img),
            "width": img.width(),
            "height": img.height(),
            "mm": mm_len(img.width()),
        }))
        .into_response(),
        Err(e) => bad_request(e),
    }
}

async fn api_print(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let data = match parse_json(&body) {
        Ok(data) => data,
        Err(e) => return bad_request(e),
    };
    let copies = match data.get("copies") {
        Some(value) => match as_int(value) {
            Ok(copies) => copies.max(1) as u32,
            Err(e) => return bad_request(e),
        },
        None => 1,
    };
    let chain = truthy(data.get("chain"));
    let save_tape = truthy(data.get("save_tape"));
    let printer_id = printer_field(&data);

    // (status code, message) errors: 400 for render/request problems,
    // 503 for printer trouble — mirroring web.py.
    let result: Result<GrayImage, (u16, String)> = run_printer_job(&state, move || {
        let mut printer = find_printer(printer_id.as_deref()).map_err(|e| (503, e.0))?;
        let st = printer.status().map_err(|e| (503, e.0))?;
        if !st.errors.is_empty() {
            return Err((503, format!("printer reports: {}", st.errors.join(", "))));
        }
        if st.tape_px == 0 {
            return Err((
                503,
                format!("unsupported tape width {} mm", st.media_width_mm),
            ));
        }
        let img = render_from_request(&data, u32::from(st.tape_px)).map_err(|e| (400, e))?;
        let label = Label::new(
            img.width(),
            img.height(),
            img.pixels().map(|p| p.0[0] < 128).collect(),
        );
        for copy in 0..copies {
            let last = copy == copies - 1;
            printer
                .print(&label, chain || !last, save_tape)
                .map_err(|e| (503, e.0))?;
        }
        Ok(img)
    })
    .await;

    match result {
        Ok(img) => Json(json!({
            "ok": true,
            "width": img.width(),
            "mm": mm_len(img.width()),
            "copies": copies,
        }))
        .into_response(),
        Err((code, message)) => (
            StatusCode::from_u16(code).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
            Json(json!({"error": message})),
        )
            .into_response(),
    }
}

pub fn router(jobs: mpsc::UnboundedSender<Job>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/labelcore.js", get(labelcore_js))
        .route("/qrcode.js", get(qrcode_js))
        .route("/fonts.json", get(fonts_json))
        .route("/fonts/{file}", get(font_file))
        .route("/api/meta", get(api_meta))
        .route("/api/status", get(api_status))
        .route("/api/print-raw", post(api_print_raw))
        .route("/api/render", post(api_render))
        .route("/api/print", post(api_print))
        .with_state(AppState { jobs })
}

/// Serve on 127.0.0.1:port. Parks the calling thread in the printer
/// job loop — call from the process main thread (the macOS Bluetooth
/// transport requires it) and expect this not to return on success.
pub fn serve(port: u16) -> Result<(), String> {
    // Bind synchronously so a taken port fails loudly here rather
    // than as a background-thread panic after startup.
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("cannot bind 127.0.0.1:{port}: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("listener setup failed: {e}"))?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
    let app = router(tx);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(std_listener).expect("tokio listener from std");
            eprintln!("serving http://127.0.0.1:{port}/");
            axum::serve(listener, app).await.expect("server error");
        });
    });
    while let Some(job) = rx.blocking_recv() {
        job();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request that renders without touching the font cache or the
    /// network: an explicit system-font path, like the Python tests.
    fn request(text: &str) -> Value {
        let font = fontcache::system_fallback().expect("a known system font exists");
        json!({"text": text, "font": font})
    }

    #[test]
    fn as_int_mirrors_python_int() {
        assert_eq!(as_int(&json!(12)), Ok(12));
        assert_eq!(as_int(&json!(12.9)), Ok(12)); // int() truncates floats
        assert_eq!(as_int(&json!(-3)), Ok(-3));
        assert_eq!(as_int(&json!("42")), Ok(42));
        assert_eq!(as_int(&json!(" 42 ")), Ok(42));
        assert_eq!(as_int(&json!(true)), Ok(1));
        assert!(as_int(&json!("12.5")).is_err()); // int("12.5") raises
        assert!(as_int(&json!("abc")).is_err());
        assert!(as_int(&json!(null)).is_err());
        assert!(as_int(&json!([1])).is_err());
    }

    #[test]
    fn as_float_mirrors_python_float() {
        assert_eq!(as_float(&json!(1.5)), Ok(1.5));
        assert_eq!(as_float(&json!(2)), Ok(2.0));
        assert_eq!(as_float(&json!("1.5")), Ok(1.5));
        assert_eq!(as_float(&json!(false)), Ok(0.0));
        assert!(as_float(&json!("wide")).is_err());
        assert!(as_float(&json!(null)).is_err());
    }

    #[test]
    fn truthy_mirrors_python() {
        assert!(!truthy(None));
        assert!(!truthy(Some(&json!(null))));
        assert!(!truthy(Some(&json!(false))));
        assert!(!truthy(Some(&json!(0))));
        assert!(!truthy(Some(&json!(0.0))));
        assert!(!truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!([]))));
        assert!(!truthy(Some(&json!({}))));
        assert!(truthy(Some(&json!(true))));
        assert!(truthy(Some(&json!(1))));
        assert!(truthy(Some(&json!("0")))); // nonempty string, like Python
        assert!(truthy(Some(&json!([0]))));
    }

    #[test]
    fn tape_px_validation_covers_both_models() {
        for px in [24, 32, 52, 76, 120, 128, 64] {
            assert!(valid_tape_px(px), "{px} should be valid");
        }
        for px in [0, 12, 77, 96, 180] {
            assert!(!valid_tape_px(px), "{px} should be invalid");
        }
    }

    #[test]
    fn render_request_text_and_size_coercions() {
        // Trailing newlines stripped, blank text refused.
        assert_eq!(
            render_from_request(&json!({"text": "  \n\n"}), 76).unwrap_err(),
            "empty label"
        );
        assert_eq!(
            render_from_request(&json!({"text": ""}), 76).unwrap_err(),
            "empty label"
        );

        let auto = render_from_request(&request("HI"), 76).unwrap();
        assert_eq!(auto.height(), 76);

        // Falsy sizes (0, "", null) mean auto-fit, exactly like
        // web.py's `int(data["size"]) if data.get("size") else None`.
        for falsy in [json!(0), json!(""), json!(null)] {
            let mut req = request("HI");
            req["size"] = falsy;
            let img = render_from_request(&req, 76).unwrap();
            assert_eq!(img.dimensions(), auto.dimensions());
        }

        // An explicit size changes the render; a string size parses.
        let mut req = request("HI");
        req["size"] = json!(20);
        let sized = render_from_request(&req, 76).unwrap();
        assert!(sized.width() < auto.width());
        let mut req = request("HI");
        req["size"] = json!("20");
        assert_eq!(
            render_from_request(&req, 76).unwrap().dimensions(),
            sized.dimensions()
        );

        // Unparseable or negative values map to a client error.
        let mut req = request("HI");
        req["size"] = json!("big");
        assert!(render_from_request(&req, 76).is_err());
        let mut req = request("HI");
        req["margin"] = json!(-1);
        assert!(render_from_request(&req, 76).is_err());
    }

    async fn response_json(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn body_of(value: &Value) -> axum::body::Bytes {
        axum::body::Bytes::from(serde_json::to_vec(value).unwrap())
    }

    #[tokio::test]
    async fn api_render_shapes_mirror_web_py() {
        // Happy path: png data URL + geometry, like web.py's response.
        let (status, body) = response_json(api_render(body_of(&request("HELLO"))).await).await;
        assert_eq!(status, StatusCode::OK);
        let png = body["png"].as_str().unwrap();
        assert!(png.starts_with("data:image/png;base64,"), "{png:.40}");
        assert_eq!(body["height"], 76); // tape_px defaults to 76
        assert!(body["width"].as_u64().unwrap() > 20);
        assert!(body["mm"].as_f64().unwrap() > 0.0);

        // Invalid tape and empty text are 400s with web.py's messages.
        let mut req = request("HELLO");
        req["tape_px"] = json!(77);
        let (status, body) = response_json(api_render(body_of(&req)).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "bad tape_px 77");

        let (status, body) = response_json(api_render(body_of(&json!({"text": " "}))).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "empty label");

        // Invalid JSON is a 400, like Flask's force=True BadRequest.
        let bad = axum::body::Bytes::from_static(b"{not json");
        let (status, body) = response_json(api_render(bad).await).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"].as_str().unwrap().contains("invalid JSON"),
            "{body}"
        );
    }

    /// web.py parses with get_json(force=True), so the Content-Type
    /// header must not matter — a bare `curl -d` post has to work.
    /// Exercised through the real router so extraction runs.
    #[tokio::test]
    async fn api_render_ignores_content_type() {
        use tower::ServiceExt;
        let (tx, _rx) = mpsc::unbounded_channel();
        let app = router(tx);
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/render")
            // no Content-Type header at all
            .body(axum::body::Body::from(
                serde_json::to_vec(&request("HELLO")).unwrap(),
            ))
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// web.py's `data.get("printer") or None`: falsy means default
    /// printer, but a truthy non-string must NOT silently fall back
    /// to the default printer — it flows through and fails to match.
    #[test]
    fn printer_field_mirrors_web_py() {
        assert_eq!(printer_field(&json!({})), None);
        assert_eq!(printer_field(&json!({"printer": null})), None);
        assert_eq!(printer_field(&json!({"printer": ""})), None);
        assert_eq!(printer_field(&json!({"printer": 0})), None);
        assert_eq!(printer_field(&json!({"printer": false})), None);
        assert_eq!(
            printer_field(&json!({"printer": "PT-H500"})),
            Some("PT-H500".to_owned())
        );
        assert_eq!(
            printer_field(&json!({"printer": 123})),
            Some("123".to_owned())
        );
        assert_eq!(
            printer_field(&json!({"printer": true})),
            Some("true".to_owned())
        );
    }
}
