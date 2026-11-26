//! Spike UI: log in to Spotify, run one liked songs check, clear the session.
//! All Spotify traffic and every secret stays in Rust.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::{Duration, Instant};

use serde::Serialize;
use spotify_login_spike::spotify::{
    client::{self, Report, Route, SpotifyClient, system_clock},
    http::ReqwestTransport,
    login::{COOKIE_URL, LOGIN_URL, allows_navigation, is_session_cookie},
    secret::Secret,
    session::{Browser, KeyringStore, SessionStore, save_login},
};
use tauri::{AppHandle, Emitter, Manager, State, Url, WebviewUrl, WebviewWindowBuilder};

const LOGIN_LABEL: &str = "spotify-login";
/// Event telling the UI how the login window ended.
const LOGIN_EVENT: &str = "spotify-login";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the cookie jar of the login window is read. Local, no network.
const COOKIE_CHECK: Duration = Duration::from_secs(1);
const STORE: KeyringStore = KeyringStore::new("dev.lumen.spike.spotify-login", "spotify-session");

struct Spotify(SpotifyClient<ReqwestTransport>);

#[derive(Serialize)]
struct SessionStatus {
    saved: bool,
    expired: bool,
}

#[derive(Clone, Serialize)]
struct LoginResult {
    saved: bool,
    message: String,
}

#[tauri::command]
async fn session_status() -> Result<SessionStatus, String> {
    let session = STORE.load().map_err(|e| e.to_string())?;
    Ok(SessionStatus {
        saved: session.is_some(),
        expired: session.is_some_and(|s| s.expired),
    })
}

/// Async on purpose: creating a window or reading cookies in a synchronous
/// command deadlocks WebView2.
#[tauri::command]
async fn open_login(
    app: AppHandle,
    user_agent: String,
    languages: Vec<String>,
) -> Result<(), String> {
    let browser = Browser::new(&user_agent, &languages).map_err(|e| e.to_string())?;
    if let Some(window) = app.get_webview_window(LOGIN_LABEL) {
        return window.set_focus().map_err(|e| e.to_string());
    }
    let url = Url::parse(LOGIN_URL).map_err(|e| e.to_string())?;
    // Incognito: the login cookies are never written to a WebView2 profile, so
    // Credential Manager holds the only saved copy. The window is remote content
    // and is not in any capability, so it cannot call these commands.
    WebviewWindowBuilder::new(&app, LOGIN_LABEL, WebviewUrl::External(url))
        .title("Log in to Spotify")
        .inner_size(520.0, 760.0)
        .incognito(true)
        .on_navigation(allows_navigation)
        .build()
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(watch_login(app, browser));
    Ok(())
}

/// Waits for `sp_dc` to appear, saves it and closes the login window.
async fn watch_login(app: AppHandle, browser: Browser) {
    let started = Instant::now();
    let result = loop {
        tokio::time::sleep(COOKIE_CHECK).await;
        let Some(window) = app.get_webview_window(LOGIN_LABEL) else {
            break Err("login window closed before a session appeared".to_owned());
        };
        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            break Err("login timed out".to_owned());
        }
        match read_sp_dc(&window) {
            Ok(Some(sp_dc)) => {
                let saved = save_session(&app, sp_dc.expose(), browser).await;
                let _ = window.close();
                break saved;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = window.close();
                break Err(format!("could not read the login cookies: {error}"));
            }
        }
    };
    let payload = match result {
        Ok(()) => LoginResult {
            saved: true,
            message: "logged in, session saved to Credential Manager".to_owned(),
        },
        Err(message) => LoginResult {
            saved: false,
            message,
        },
    };
    let _ = app.emit(LOGIN_EVENT, payload);
}

fn read_sp_dc(window: &tauri::WebviewWindow) -> tauri::Result<Option<Secret>> {
    let url = Url::parse(COOKIE_URL).expect("COOKIE_URL is valid");
    Ok(window
        .cookies_for_url(url)?
        .iter()
        .find(|c| is_session_cookie(c.name(), c.domain(), c.value()))
        .map(|c| Secret::new(c.value())))
}

async fn save_session(app: &AppHandle, sp_dc: &str, browser: Browser) -> Result<(), String> {
    save_login(&STORE, sp_dc, browser).map_err(|e| e.to_string())?;
    app.state::<Spotify>().0.forget().await;
    Ok(())
}

/// Fallback when the login window cannot be used.
#[tauri::command]
async fn save_manual_session(
    app: AppHandle,
    sp_dc: String,
    user_agent: String,
    languages: Vec<String>,
) -> Result<(), String> {
    let browser = Browser::new(&user_agent, &languages).map_err(|e| e.to_string())?;
    save_session(&app, &sp_dc, browser).await
}

#[tauri::command]
async fn run_check(spotify: State<'_, Spotify>, route: Route) -> Result<Report, String> {
    client::run_check(&spotify.0, &STORE, route)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn clear_session(app: AppHandle, spotify: State<'_, Spotify>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LOGIN_LABEL) {
        window.close().map_err(|e| e.to_string())?;
    }
    STORE.clear().map_err(|e| e.to_string())?;
    spotify.0.forget().await;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let spotify = Spotify(SpotifyClient::new(ReqwestTransport::new()?, system_clock));
    tauri::Builder::default()
        .manage(spotify)
        .invoke_handler(tauri::generate_handler![
            session_status,
            open_login,
            save_manual_session,
            run_check,
            clear_session
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
