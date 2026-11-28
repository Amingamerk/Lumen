//! Lumen's Rust core. Application state lives here; the webview only renders it.

mod art;
mod audio;
mod catalog;
mod ipc;
mod library;
mod queue;
mod recs;
mod resolver;
mod secrets;
mod sources;
mod state;

use tauri::{Manager, RunEvent, WindowEvent};

/// Builds the app and runs its event loop until the last window closes.
///
/// Only one instance ever runs, so there is one session and one rate limiter. A
/// second launch focuses the running window and exits.
pub fn run() -> Result<(), tauri::Error> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let list = state::wants_list(&args);
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
        .register_asynchronous_uri_scheme_protocol(ipc::art::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let webview = ctx.webview_label().to_owned();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn(async move {
                responder.respond(ipc::art::serve(&app, &webview, &path).await);
            });
        })
        .manage(ipc::SharedState::new(state::AppState::without_samples(
            args,
        )))
        .setup(move |app| {
            ipc::art::start(app.handle())?;
            ipc::youtube::start(app.handle())?;
            ipc::library::start(app.handle(), list)?;
            ipc::playback::start(app.handle())?;
            Ok(ipc::player::start(app.handle(), play_file)?)
        })
        .invoke_handler(tauri::generate_handler![
            ipc::get_app_state,
            ipc::navigate,
            ipc::get_home,
            ipc::get_library,
            ipc::get_collection,
            ipc::get_track_slice,
            ipc::get_search,
            ipc::search::submit_search,
            ipc::search::set_search_filter,
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
            ipc::library::get_library_summary,
            ipc::library::like_track,
            ipc::library::unlike_track,
            ipc::library::get_liked_ids,
            ipc::library::get_liked_slice,
            ipc::library::get_recent_slice,
            ipc::library::play_library_track,
            ipc::playlists::get_playlists,
            ipc::playlists::get_playlist,
            ipc::playlists::get_playlist_slice,
            ipc::playlists::create_playlist,
            ipc::playlists::rename_playlist,
            ipc::playlists::delete_playlist,
            ipc::playlists::add_to_playlist,
            ipc::playlists::remove_from_playlist,
            ipc::account::get_account_status,
            ipc::account::sign_in,
            ipc::account::disconnect,
            ipc::youtube::get_youtube_status,
            ipc::youtube::resume_youtube,
        ])
        .build(tauri::generate_context!())?
        .run(|app, event| {
            if matches!(event, RunEvent::Exit) {
                ipc::playback::stop(app);
                ipc::player::stop(app);
                ipc::library::stop(app);
            }
        });
    Ok(())
}
