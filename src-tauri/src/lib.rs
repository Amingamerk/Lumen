//! Lumen's Rust core. Application state lives here; the webview only renders it.

mod audio;
mod catalog;
mod ipc;
mod queue;
mod recs;
mod secrets;
mod sources;
mod state;

use tauri::{Manager, RunEvent, WindowEvent};

/// Builds the app and runs its event loop until the last window closes.
///
/// Only one instance ever runs, so there is one session and one rate limiter. A
/// second launch focuses the running window and exits.
pub fn run() -> Result<(), tauri::Error> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    let play_file = audio::play_file_arg(std::env::args_os().skip(1));
    tauri::Builder::default()
        // First, so a second launch exits before it sets anything up.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            ipc::focus_main_window(app);
        }))
        .on_window_event(|window, event| {
            // Without the main window the login window would keep Lumen alive.
            if window.label() == ipc::MAIN_WINDOW && matches!(event, WindowEvent::Destroyed) {
                // If this fails, the user can still close it by hand.
                let _ = ipc::account::close_login(window.app_handle());
            }
        })
        .manage(ipc::account::AccountHandle::default())
        .manage(ipc::SharedState::new(state::AppState::from_args(args)))
        .setup(move |app| Ok(ipc::player::start(app.handle(), play_file)?))
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
            ipc::account::get_account_status,
            ipc::account::sign_in,
            ipc::account::disconnect,
        ])
        .build(tauri::generate_context!())?
        .run(|app, event| {
            if matches!(event, RunEvent::Exit) {
                ipc::player::stop(app);
            }
        });
    Ok(())
}
