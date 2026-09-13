#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod lobby;
mod network;

use std::path::PathBuf;
use std::sync::Arc;

use network::engine::Engine;
use tauri::{Manager, State};

struct AppState {
    engine: Arc<Engine>,
}

#[tauri::command]
fn create_room(state: State<'_, AppState>, name: String) -> Result<serde_json::Value, String> {
    state.engine.create_room(name)
}

#[tauri::command]
fn join_room(state: State<'_, AppState>, target: String) -> Result<serde_json::Value, String> {
    state.engine.join_room(target)
}

#[tauri::command]
fn leave_room(state: State<'_, AppState>) {
    state.engine.leave_room();
}

#[tauri::command]
fn get_session(state: State<'_, AppState>) -> serde_json::Value {
    state.engine.session_json()
}

#[tauri::command]
fn get_peers(state: State<'_, AppState>) -> serde_json::Value {
    state.engine.peers_json()
}

#[tauri::command]
fn set_name(state: State<'_, AppState>, name: String) {
    state.engine.set_name(&name);
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to start tokio runtime");

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            create_room,
            join_room,
            leave_room,
            get_session,
            get_peers,
            set_name
        ])
        .setup(|app| {
            let mut candidates = Vec::<PathBuf>::new();
            if let Ok(rd) = app.path().resource_dir() {
                candidates.push(rd.join("assets").join("wintun.dll"));
            }
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    candidates.push(dir.join("wintun.dll"));
                }
            }
            candidates.push(PathBuf::from(r"C:\Windows\System32\wintun.dll"));

            let engine = Engine::new(rt, candidates);
            let default_name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Player".to_string());
            engine.set_name(&default_name);
            engine.attach(app.handle().clone());
            app.manage(AppState {
                engine: Arc::clone(&engine),
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running MNX LAN");
}