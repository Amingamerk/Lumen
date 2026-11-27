//! Lumen's Rust core. Application state lives here; the webview only renders it.

mod audio;
mod catalog;
mod ipc;
mod queue;
mod recs;
mod state;

/// Builds the app and runs its event loop until the last window closes.
pub fn run() -> Result<(), tauri::Error> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    tauri::Builder::default()
        .manage(ipc::SharedState::new(state::AppState::from_args(args)))
        .setup(|app| Ok(ipc::player::start(app.handle())?))
        .invoke_handler(tauri::generate_handler![
            ipc::get_app_state,
            ipc::navigate,
            ipc::get_home,
            ipc::get_library,
            ipc::get_collection,
            ipc::get_track_slice,
            ipc::get_search,
            ipc::search_catalog,
            ipc::toggle_queue,
            ipc::player::get_player_state,
            ipc::player::play_collection,
            ipc::player::play_search_result,
            ipc::player::toggle_play,
            ipc::player::next_track,
            ipc::player::previous_track,
            ipc::player::seek,
            ipc::player::seek_by,
            ipc::player::set_volume,
            ipc::player::change_volume,
            ipc::player::get_queue,
            ipc::player::move_queue_entry,
        ])
        .run(tauri::generate_context!())
}
