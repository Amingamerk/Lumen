//! Lumen's Rust core. Application state lives here; the webview only renders it.

mod catalog;
mod ipc;
mod state;

/// Builds the app and runs its event loop until the last window closes.
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        .manage(ipc::SharedState::default())
        .invoke_handler(tauri::generate_handler![
            ipc::get_app_state,
            ipc::navigate,
            ipc::get_home,
            ipc::get_library,
            ipc::get_collection,
            ipc::get_track_slice,
            ipc::get_search,
            ipc::search_catalog,
        ])
        .run(tauri::generate_context!())
}
