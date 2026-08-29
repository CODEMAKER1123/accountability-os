//! Local HTTP bridge for the browser extension (spec §9). Binds loopback
//! only; the extension authenticates with a token shown in Settings. Only
//! the active tab of a focused browser window ever arrives here, and it
//! goes no further than in-memory state + the local database.

use std::io::Read;

use serde::Deserialize;
use tauri::Manager;

use crate::state::{AppState, BrowserReport};

#[derive(Deserialize)]
struct ActivityReport {
    domain: String,
    title: String,
    #[serde(default = "default_true")]
    window_focused: bool,
}

fn default_true() -> bool {
    true
}

pub fn spawn(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("extension-bridge".into())
        .spawn(move || {
            let (port, expected_token) = {
                let state = app.state::<AppState>();
                let engine = state.engine.lock();
                (engine.settings.extension_port, engine.settings.extension_token.clone())
            };
            let server = match tiny_http::Server::http(("127.0.0.1", port)) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(target: "bridge", "extension bridge failed to bind 127.0.0.1:{port}: {e}");
                    return;
                }
            };
            log::info!(target: "bridge", "extension bridge listening on 127.0.0.1:{port}");
            for mut request in server.incoming_requests() {
                let response = handle(&app, &expected_token, &mut request);
                let _ = request.respond(response);
            }
        })
        .expect("spawn extension bridge thread");
}

fn cors(mut resp: tiny_http::Response<std::io::Cursor<Vec<u8>>>) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let hdr = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("static header");
    resp.add_header(hdr("Access-Control-Allow-Origin", "*"));
    resp.add_header(hdr("Access-Control-Allow-Headers", "Content-Type, X-AOS-Token"));
    resp.add_header(hdr("Access-Control-Allow-Methods", "GET, POST, OPTIONS"));
    resp
}

fn json_response(status: u16, body: &str) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let resp = tiny_http::Response::from_string(body)
        .with_status_code(status)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                .expect("static header"),
        );
    cors(resp)
}

fn handle(
    app: &tauri::AppHandle,
    expected_token: &str,
    request: &mut tiny_http::Request,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let method = request.method().clone();
    let url = request.url().to_string();

    if method == tiny_http::Method::Options {
        return json_response(204, "");
    }
    match (method, url.as_str()) {
        (tiny_http::Method::Get, "/v1/ping") => {
            json_response(200, r#"{"ok":true,"app":"accountability-os"}"#)
        }
        (tiny_http::Method::Post, "/v1/activity") => {
            let token = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("X-AOS-Token"))
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_default();
            if expected_token.is_empty() || token != expected_token {
                return json_response(401, r#"{"error":"invalid token"}"#);
            }
            let mut body = String::new();
            // Cap the body read: this is a tiny metadata payload.
            let mut limited = request.as_reader().take(16 * 1024);
            if limited.read_to_string(&mut body).is_err() {
                return json_response(400, r#"{"error":"unreadable body"}"#);
            }
            let Ok(report) = serde_json::from_str::<ActivityReport>(&body) else {
                return json_response(400, r#"{"error":"invalid payload"}"#);
            };
            let state = app.state::<AppState>();
            let mut engine = state.engine.lock();
            if engine.settings.browser_monitoring_enabled {
                engine.last_extension_report = Some(BrowserReport {
                    domain: report.domain.chars().take(253).collect(),
                    title: report.title.chars().take(300).collect(),
                    at: crate::db::now(),
                    window_focused: report.window_focused,
                });
            }
            json_response(204, "")
        }
        _ => json_response(404, r#"{"error":"not found"}"#),
    }
}
