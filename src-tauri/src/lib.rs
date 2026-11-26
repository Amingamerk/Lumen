//! Lumen's Rust core. Application state lives here; the webview only renders it.

mod ipc;
mod state;

/// Builds the app and runs its event loop until the last window closes.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        .manage(ipc::SharedState::default())
        .invoke_handler(tauri::generate_handler![ipc::get_app_state, ipc::navigate])
        .run(tauri::generate_context!())
}
