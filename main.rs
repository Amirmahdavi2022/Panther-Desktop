// Thin Tauri layer. All the real behaviour lives in `engines`, which is plain
// std Rust so it can be compiled and tested without a windowing stack.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engines;

use engines::{Engine, Supervisor};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, RunEvent, State};

struct AppState {
    supervisor: Mutex<Supervisor>,
}

#[derive(Serialize)]
struct Status {
    running: bool,
    engine: Option<String>,
    socks: Option<String>,
    region: String,
}

fn snapshot(sup: &mut Supervisor) -> Status {
    let running = sup.is_running();
    Status {
        running,
        engine: sup.current().map(|e| e.label().to_string()),
        socks: sup.socks_port().map(|p| format!("127.0.0.1:{p}")),
        region: sup.region().to_string(),
    }
}

// Async so Tauri runs it on a worker thread. A plain `fn` command runs on the
// main thread, and starting an engine waits up to 45 s per step for its port,
// which froze the whole window until it finished.
#[tauri::command]
async fn start_engine(
    engine: String,
    region: Option<String>,
    state: State<'_, AppState>,
) -> Result<Status, String> {
    let picked = Engine::parse(&engine).ok_or_else(|| format!("unknown engine: {engine}"))?;
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    sup.start_in(picked, region.as_deref().unwrap_or(""))
        .map_err(|e| e.to_string())?;
    Ok(snapshot(&mut sup))
}

/// The exit countries to offer before the engine has listed its own.
#[tauri::command]
fn regions() -> Vec<String> {
    engines::REGIONS.iter().map(|s| s.to_string()).collect()
}

#[tauri::command]
async fn stop_engine(state: State<'_, AppState>) -> Result<(), String> {
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    sup.stop();
    Ok(())
}

#[tauri::command]
async fn engine_status(state: State<'_, AppState>) -> Result<Status, String> {
    let mut sup = state.supervisor.lock().map_err(|e| e.to_string())?;
    Ok(snapshot(&mut sup))
}

#[derive(Serialize)]
struct Exit {
    ip: String,
    country: String,
}

/// The address and country the internet actually sees, asked through the
/// running engine. Runs on a blocking thread: it is a network round trip.
#[tauri::command]
async fn exit_info(state: State<'_, AppState>) -> Result<Exit, String> {
    let port = {
        let sup = state.supervisor.lock().map_err(|e| e.to_string())?;
        sup.socks_port().ok_or_else(|| "not connected".to_string())?
    };
    let (ip, country) = tauri::async_runtime::spawn_blocking(move || {
        // One retry: the very first request through a fresh tunnel is the one
        // most likely to be slow.
        engines::exit_check(port).or_else(|_| engines::exit_check(port))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(Exit { ip, country })
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Cores ship as resources next to the executable; engine state goes
            // in the per-user data directory so nothing is written into
            // Program Files.
            let resources = app
                .path()
                .resolve("resources", tauri::path::BaseDirectory::Resource)
                .unwrap_or_else(|_| std::path::PathBuf::from("resources"));
            let data = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("panther"));
            // Whole-system VPN: every app goes through the tunnel, the way the
            // Android build works. Needs admin rights, which the manifest in
            // build.rs asks for.
            let mut supervisor = Supervisor::new(resources, data);
            supervisor.set_tunnel(true);
            app.manage(AppState {
                supervisor: Mutex::new(supervisor),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_engine,
            stop_engine,
            engine_status,
            regions,
            exit_info,
            app_version
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Panther")
        .run(|app, event| {
            // However the app is closed, take the tunnel and the cores down
            // with it. A tunnel left behind with no engine under it would cut
            // the machine off the internet.
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut sup) = state.supervisor.lock() {
                        sup.stop();
                    }
                }
            }
        });
}
