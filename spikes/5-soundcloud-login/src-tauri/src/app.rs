//! Tauri shell: the login window and the commands the webview calls.

use std::{
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::{
    sync::{Mutex, oneshot},
    time::{Instant, sleep, timeout},
};
use url::Url;

use crate::{
    http::Identity,
    likes::{self, Likes},
    login::{self, COOKIE_URL, IDENTITY_SCRIPT, LOGIN_URL, TOKEN_COOKIE},
    secret::Secret,
    session::{self, KeyringStore, SessionStatus, SessionStore},
    throttle::{Limits, Throttle},
    transport::ReqwestHttp,
};

const SERVICE: &str = "dev.lumen.spike.soundcloud";
const MAIN: &str = "main";
const LOGIN: &str = "soundcloud-login";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

struct Spike {
    http: Throttle<ReqwestHttp>,
    store: KeyringStore,
    /// One session operation at a time: no parallel fetches, no save racing a clear.
    session_lock: Mutex<()>,
    /// Bumped to abandon a running login watcher.
    login_generation: AtomicU64,
}

/// Command errors reach the webview as text. Every message is free of secrets.
fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn current_status(spike: &Spike) -> Result<SessionStatus, String> {
    let stored = spike.store.load().map_err(text)?;
    Ok(session::status(stored.as_ref()))
}

fn announce(app: &AppHandle, spike: &Spike) {
    if let Ok(status) = current_status(spike) {
        let _ = app.emit_to(MAIN, "session-changed", status);
    }
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(MAIN)
        .ok_or_else(|| "the main window is gone".to_string())
}

/// The `oauth_token` cookie, if the webview profile has one. Cookie reads deadlock on the
/// UI thread, so this runs on a blocking thread.
async fn read_token(window: WebviewWindow) -> Option<Secret> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = Url::parse(COOKIE_URL).ok()?;
        let cookies = window.cookies_for_url(url).ok()?;
        login::token_from_cookies(cookies.iter().map(|c| (c.name(), c.value())))
    })
    .await
    .ok()
    .flatten()
}

async fn delete_token_cookie(window: WebviewWindow) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = Url::parse(COOKIE_URL).map_err(text)?;
        for cookie in window.cookies_for_url(url).map_err(text)? {
            if cookie.name() == TOKEN_COOKIE {
                window.delete_cookie(cookie).map_err(text)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(text)?
}

/// User agent and language of a webview, as that webview reports them.
async fn read_identity(window: &WebviewWindow) -> Option<Identity> {
    let (sender, receiver) = oneshot::channel();
    let sender = StdMutex::new(Some(sender));
    window
        .eval_with_callback(IDENTITY_SCRIPT, move |json| {
            if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
                let _ = sender.send(json);
            }
        })
        .ok()?;
    let json = timeout(Duration::from_secs(3), receiver).await.ok()?.ok()?;
    login::identity_from_eval(&json)
}

/// Waits for the login window to yield a token, stores it and closes the window.
async fn watch_login(app: AppHandle, window: WebviewWindow, generation: u64) {
    let spike = app.state::<Spike>();
    let report = |message: &str| {
        let _ = app.emit_to(MAIN, "login-status", message);
    };
    let superseded = || spike.login_generation.load(Ordering::SeqCst) != generation;
    let deadline = Instant::now() + LOGIN_TIMEOUT;
    loop {
        sleep(Duration::from_secs(1)).await;
        if superseded() {
            return;
        }
        if app.get_webview_window(LOGIN).is_none() {
            return report("Login window closed without a session.");
        }
        if Instant::now() > deadline {
            let _ = window.destroy();
            return report("Login timed out.");
        }
        let Some(token) = read_token(window.clone()).await else {
            continue;
        };
        let Some(identity) = read_identity(&window).await else {
            continue;
        };
        let saved = {
            let _guard = spike.session_lock.lock().await;
            if superseded() {
                return;
            }
            session::save_login(&spike.store, token, identity)
        };
        let _ = window.destroy();
        announce(&app, &spike);
        return match saved {
            Ok(()) => report("Signed in. The session is in Credential Manager."),
            Err(error) => report(&error.to_string()),
        };
    }
}

#[tauri::command]
fn session_status(spike: State<'_, Spike>) -> Result<SessionStatus, String> {
    current_status(&spike)
}

// Async because creating a window or reading cookies in a sync command deadlocks WebView2.
#[tauri::command]
async fn open_login(app: AppHandle, spike: State<'_, Spike>) -> Result<(), String> {
    let _guard = spike.session_lock.lock().await;
    if let Some(window) = app.get_webview_window(LOGIN) {
        window.set_focus().map_err(text)?;
        return Err("the login window is already open".to_string());
    }
    // A dead token left in the webview profile would be captured again at once.
    if spike.store.load().map_err(text)?.is_some_and(|s| s.expired) {
        delete_token_cookie(main_window(&app)?).await?;
    }
    let url = Url::parse(LOGIN_URL).map_err(text)?;
    // No capability names this window, so the remote page cannot call into Rust.
    let window = WebviewWindowBuilder::new(&app, LOGIN, WebviewUrl::External(url))
        .title("Sign in to SoundCloud")
        // soundcloud.com scrolls sideways below about 1000 px.
        .inner_size(1040.0, 760.0)
        .build()
        .map_err(text)?;
    let generation = spike.login_generation.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(watch_login(app.clone(), window, generation));
    Ok(())
}

/// Fallback for debugging: stores a token pasted by hand.
#[tauri::command]
async fn import_token(
    app: AppHandle,
    spike: State<'_, Spike>,
    token: String,
) -> Result<SessionStatus, String> {
    let token = session::parse_token(&token).map_err(text)?;
    let identity = read_identity(&main_window(&app)?)
        .await
        .ok_or("could not read the WebView2 user agent")?;
    let _guard = spike.session_lock.lock().await;
    session::save_login(&spike.store, token, identity).map_err(text)?;
    current_status(&spike)
}

#[tauri::command]
async fn fetch_likes(app: AppHandle, spike: State<'_, Spike>) -> Result<Likes, String> {
    let _guard = spike.session_lock.lock().await;
    let result = likes::fetch_likes(&spike.http, &spike.store).await;
    // The fetch may have cached a client_id or marked the session expired.
    announce(&app, &spike);
    result.map_err(text)
}

/// Forgets the stored session and signs the webview profile out.
#[tauri::command]
async fn clear_session(app: AppHandle, spike: State<'_, Spike>) -> Result<SessionStatus, String> {
    let _guard = spike.session_lock.lock().await;
    spike.login_generation.fetch_add(1, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(LOGIN) {
        window.destroy().map_err(text)?;
    }
    spike.store.clear().map_err(text)?;
    main_window(&app)?.clear_all_browsing_data().map_err(text)?;
    current_status(&spike)
}

pub fn run() -> anyhow::Result<()> {
    let spike = Spike {
        http: Throttle::new(ReqwestHttp::new()?, Limits::default()),
        store: KeyringStore::new(SERVICE),
        session_lock: Mutex::new(()),
        login_generation: AtomicU64::new(0),
    };
    tauri::Builder::default()
        .manage(spike)
        .invoke_handler(tauri::generate_handler![
            session_status,
            open_login,
            import_token,
            fetch_likes,
            clear_session
        ])
        .run(tauri::generate_context!())?;
    Ok(())
}
