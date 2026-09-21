// Thin Tauri layer. All the real behaviour lives in `engines`, which is plain
// std Rust so it can be compiled and tested without a windowing stack.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engines;

use engines::{Engine, Supervisor, SOCKS_PORT};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};

struct AppState {
    supervisor: Mutex<Supervisor>,
}

#[derive(Serialize)]
struct Status {
    running: bool,
    engine: Option<String>,
    socks: String,
}

#[tauri::command]
fn start_engine(engine: String, state: State<'_, AppState>) -> Result<Status, String> {
    let picked = Engine::parse(&engine).ok_or_else(|| format!("unknown engine: {engine}"))?;
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    sup.start(picked, SOCKS_PORT).map_err(|e| e.to_string())?;
    Ok(Status {
        running: true,
        engine: Some(picked.label().to_string()),
        socks: format!("127.0.0.1:{SOCKS_PORT}"),
    })
}

#[tauri::command]
fn stop_engine(state: State<'_, AppState>) -> Result<(), String> {
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    sup.stop();
    Ok(())
}

#[tauri::command]
fn engine_status(state: State<'_, AppState>) -> Result<Status, String> {
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    let running = sup.is_running();
    Ok(Status {
        running,
        engine: sup.current().map(|e| e.label().to_string()),
        socks: format!("127.0.0.1:{SOCKS_PORT}"),
    })
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Cores are bundled as resources next to the executable.
            let dir = app
                .path()
                .resolve("resources", tauri::path::BaseDirectory::Resource)
                .unwrap_or_else(|_| std::path::PathBuf::from("resources"));
            app.manage(AppState {
                supervisor: Mutex::new(Supervisor::new(dir)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_engine,
            stop_engine,
            engine_status,
            app_version
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Panther");
}
