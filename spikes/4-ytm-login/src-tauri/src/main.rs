// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod webview;

use anyhow::Context;
use tauri::{Manager, RunEvent, WindowEvent};
use ytm_login_spike::{
    account::Account, clock::SystemClock, http::HttpTransport, jar, resolver::YtDlpProcess,
    session::KeyringStore,
};

/// Credential Manager name of the stored session.
const CREDENTIAL_SERVICE: &str = "lumen-spike-ytm";
/// Event carrying the new session status (or null) to the main window.
const SESSION_CHANGED: &str = "session-changed";

type App = Account<KeyringStore, HttpTransport, YtDlpProcess, SystemClock>;

fn main() -> anyhow::Result<()> {
    let jar_dir = jar::default_dir();
    // A crash or kill skips destructors, so clear leftovers before doing anything.
    jar::sweep(&jar_dir).context("could not remove old temporary cookie files")?;
    let account: App = Account::new(
        KeyringStore::new(CREDENTIAL_SERVICE),
        HttpTransport::new().context("could not set up HTTP")?,
        YtDlpProcess,
        SystemClock,
        jar_dir.clone(),
    );

    tauri::Builder::default()
        .manage(account)
        .invoke_handler(tauri::generate_handler![
            commands::session_status,
            commands::open_login,
            commands::capture_now,
            commands::import_session,
            commands::clear_session,
            commands::run_check,
            commands::play_last_stream,
        ])
        .build(tauri::generate_context!())
        .context("failed to start the Tauri app")?
        .run(move |app, event| match event {
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::Destroyed,
                ..
            } if label == "main" => {
                if let Some(login) = app.get_webview_window(webview::LOGIN_LABEL) {
                    // Failing to close it only means the user closes it by hand.
                    let _ = login.close();
                }
            }
            RunEvent::Exit => {
                // Let a running yt-dlp finish first: it rewrites the cookie file when it exits.
                tauri::async_runtime::block_on(app.state::<App>().wait_until_idle());
                if let Err(error) = jar::sweep(&jar_dir) {
                    eprintln!("could not remove temporary cookie files: {error}");
                }
            }
            _ => {}
        });
    Ok(())
}
