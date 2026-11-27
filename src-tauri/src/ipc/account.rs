//! The YouTube Music login window and the account commands.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::webview::PageLoadEvent;
use tauri::{
    AppHandle, Emitter, Manager, Runtime, State, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use tokio::sync::oneshot;

use super::youtube::Service;
use super::{IpcError, MAIN_WINDOW, lock, search};
use crate::secrets::SecretError;
use crate::sources::youtube::{
    AccountStatus, Cookie, LoginAttempt, ORIGIN, PageIdentity, Session, is_signed_in,
};

/// Events sent to the UI. Mirrored in `src/lib/ipc.ts`.
pub const ACCOUNT_CHANGED: &str = "account-changed";
pub const SIGN_IN_FAILED: &str = "sign-in-failed";

/// The login window's label. No capability may name it, so the remote pages it
/// shows cannot call into Rust.
const LOGIN_WINDOW: &str = "youtube-login";
/// Evaluated in the login page. The origin comes back with the values, so they
/// are known to belong to it.
const IDENTITY_SCRIPT: &str = "[location.origin, navigator.userAgent, navigator.language]";
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
/// Waits before each look at a loaded music page. A cookie can land a moment
/// after the page does.
const CAPTURE_DELAYS: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

// Fixed texts: an error from the webview or the store is never passed on.
const READ_FAILED: &str =
    "Lumen could not read the sign-in. Close the sign-in window and try again.";
const SAVE_FAILED: &str = "Lumen could not save the sign-in to Windows Credential Manager.";

/// YouTube Music, and with it the account, behind the commands.
pub struct AccountHandle {
    youtube: Arc<Service>,
    /// Held while the login window is looked up and built, so there is one.
    opening: Mutex<()>,
}

impl AccountHandle {
    pub fn new(youtube: Arc<Service>) -> Self {
        Self {
            youtube,
            opening: Mutex::default(),
        }
    }

    pub fn youtube(&self) -> &Arc<Service> {
        &self.youtube
    }
}

// The commands are `async` because building a window or reading cookies in a
// synchronous command deadlocks on Windows (wry#583).

#[tauri::command]
pub async fn get_account_status(
    handle: State<'_, AccountHandle>,
) -> Result<AccountStatus, IpcError> {
    Ok(handle.youtube.account().status()?)
}

/// Opens the login window, or focuses the one already open. The stored session
/// stays until a new sign-in completes.
#[tauri::command]
pub async fn sign_in<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, AccountHandle>,
) -> Result<(), IpcError> {
    let _opening = lock(&handle.opening);
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW) {
        window.unminimize()?;
        return Ok(window.set_focus()?);
    }
    let attempt = handle.youtube.account().begin_login();
    open_login(&app, attempt)?;
    Ok(())
}

#[tauri::command]
pub async fn disconnect<R: Runtime>(
    app: AppHandle<R>,
    handle: State<'_, AccountHandle>,
) -> Result<(), IpcError> {
    // This also cancels the open window's attempt, so it cannot save again.
    let cleared = handle.youtube.disconnect(|status| announce(&app, status));
    // The window goes even if clearing failed: it can no longer sign in.
    close_login(&app)?;
    Ok(cleared?)
}

/// Closes the login window, if one is open.
pub fn close_login<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    match app.get_webview_window(LOGIN_WINDOW) {
        Some(window) => window.destroy(),
        None => Ok(()),
    }
}

pub fn announce<R: Runtime>(app: &AppHandle<R>, status: AccountStatus) {
    // A failure means the window is gone: nobody is left to tell.
    let _ = app.emit_to(MAIN_WINDOW, ACCOUNT_CHANGED, status);
    // Songs found with another session are no longer shown.
    search::account_changed(app);
}

/// YouTube Music in an InPrivate window of its own: nothing of the login stays
/// in the WebView2 profile, and no live page keeps rotating the captured cookies.
fn open_login<R: Runtime>(app: &AppHandle<R>, attempt: LoginAttempt) -> tauri::Result<()> {
    let url = Url::parse(ORIGIN).map_err(tauri::Error::InvalidUrl)?;
    WebviewWindowBuilder::new(app, LOGIN_WINDOW, WebviewUrl::External(url))
        .title("Sign in to YouTube Music")
        .inner_size(1000.0, 760.0)
        .incognito(true)
        .on_page_load(move |window, load| {
            if matches!(load.event(), PageLoadEvent::Finished) && is_music_page(load.url()) {
                let app = window.app_handle().clone();
                // Reading cookies on this (main) thread deadlocks on Windows.
                tauri::async_runtime::spawn(capture_after_load(app, attempt));
            }
        })
        .build()?;
    Ok(())
}

/// What one look at the login window found.
enum Look {
    NotSignedIn,
    Unreadable,
    SignedIn(Session),
}

/// Stores the session and destroys the window once the sign-in is complete.
async fn capture_after_load<R: Runtime>(app: AppHandle<R>, attempt: LoginAttempt) {
    let mut unreadable = false;
    for delay in CAPTURE_DELAYS {
        tokio::time::sleep(delay).await;
        let Some(window) = app.get_webview_window(LOGIN_WINDOW) else {
            return;
        };
        match look(window).await {
            Look::NotSignedIn => unreadable = false,
            Look::Unreadable => unreadable = true,
            Look::SignedIn(session) => return store(&app, attempt, &session),
        }
    }
    // A window the user has closed meanwhile is not a failure.
    if unreadable && app.get_webview_window(LOGIN_WINDOW).is_some() {
        let _ = app.emit_to(MAIN_WINDOW, SIGN_IN_FAILED, READ_FAILED);
    }
}

/// What follows an attempt to store a captured session.
#[derive(Debug, PartialEq)]
enum AfterLogin {
    /// Another look stored it, or the attempt was cancelled.
    Nothing,
    /// Close the window, then confirm the session once.
    Verify,
    /// Say that saving failed, then close the window.
    Report,
}

fn after_login(stored: &Result<bool, SecretError>) -> AfterLogin {
    match stored {
        Ok(true) => AfterLogin::Verify,
        Ok(false) => AfterLogin::Nothing,
        Err(_) => AfterLogin::Report,
    }
}

fn store<R: Runtime>(app: &AppHandle<R>, attempt: LoginAttempt, session: &Session) {
    let youtube = Arc::clone(app.state::<AccountHandle>().youtube());
    let stored = youtube.complete_login(attempt, session, |status| announce(app, status));
    let next = after_login(&stored);
    match next {
        AfterLogin::Nothing => return,
        AfterLogin::Verify => {}
        AfterLogin::Report => {
            let _ = app.emit_to(MAIN_WINDOW, SIGN_IN_FAILED, SAVE_FAILED);
        }
    }
    // Destroyed, not closed: the page gets no say, and the memory comes back.
    // If that fails, the user can still close the window by hand.
    let _ = close_login(app);
    if next == AfterLogin::Verify {
        // The outcome reaches the UI as a status event.
        tauri::async_runtime::spawn(async move { youtube.verify().await });
    }
}

async fn look<R: Runtime>(window: WebviewWindow<R>) -> Look {
    match window.url() {
        Ok(url) if is_music_page(&url) => {}
        Ok(_) => return Look::NotSignedIn,
        Err(_) => return Look::Unreadable,
    }
    let Some(cookies) = music_cookies(window.clone()).await else {
        return Look::Unreadable;
    };
    if !is_signed_in(&cookies) {
        return Look::NotSignedIn;
    }
    let Some(page) = page_identity(&window).await else {
        return Look::Unreadable;
    };
    match Session::capture(page, cookies) {
        Some(session) => Look::SignedIn(session),
        None => Look::NotSignedIn,
    }
}

/// The cookies a request to music.youtube.com carries, `HttpOnly` ones included.
async fn music_cookies<R: Runtime>(window: WebviewWindow<R>) -> Option<Vec<Cookie>> {
    let url = Url::parse(ORIGIN).ok()?;
    let cookies = tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(url))
        .await
        .ok()?
        .ok()?;
    Some(cookies.iter().map(Cookie::from).collect())
}

/// Asked of the page itself: WebView2 sends YouTube another User-Agent than its
/// settings report.
async fn page_identity<R: Runtime>(window: &WebviewWindow<R>) -> Option<PageIdentity> {
    let (sender, receiver) = oneshot::channel();
    let sender = Mutex::new(Some(sender));
    window
        .eval_with_callback(IDENTITY_SCRIPT, move |result| {
            if let Some(sender) = lock(&sender).take() {
                // Nobody listens once the wait below has timed out.
                let _ = sender.send(result);
            }
        })
        .ok()?;
    let result = tokio::time::timeout(SCRIPT_TIMEOUT, receiver)
        .await
        .ok()?
        .ok()?;
    page_identity_from(&result)
}

fn is_music_page(url: &Url) -> bool {
    Url::parse(ORIGIN).is_ok_and(|music| url.origin() == music.origin())
}

/// Decodes the JSON that evaluating [`IDENTITY_SCRIPT`] hands back.
fn page_identity_from(result: &str) -> Option<PageIdentity> {
    let (origin, user_agent, language) = serde_json::from_str(result).ok()?;
    Some(PageIdentity {
        origin,
        user_agent,
        language,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::Value;

    use super::*;

    #[test]
    fn only_a_newly_stored_login_is_verified() {
        assert_eq!(after_login(&Ok(true)), AfterLogin::Verify);
        // Stored by another look, or cancelled.
        assert_eq!(after_login(&Ok(false)), AfterLogin::Nothing);
        assert_eq!(after_login(&Err(SecretError::Store)), AfterLogin::Report);
    }

    #[test]
    fn only_youtube_music_over_https_counts_as_the_music_page() {
        let is_music = |url: &str| is_music_page(&Url::parse(url).unwrap());

        assert!(is_music("https://music.youtube.com/"));
        assert!(is_music("https://music.youtube.com/library?x=1"));
        assert!(!is_music("http://music.youtube.com/"));
        assert!(!is_music("https://music.youtube.com:8443/"));
        assert!(!is_music(
            "https://accounts.google.com/v3/signin/identifier"
        ));
        assert!(!is_music("https://consent.youtube.com/m"));
        assert!(!is_music("https://music.youtube.com.example.org/"));
    }

    #[test]
    fn reads_the_page_identity_from_a_script_result() {
        let result = r#"["https://music.youtube.com","Mozilla/5.0 Fake","da-DK"]"#;

        assert_eq!(
            page_identity_from(result),
            Some(PageIdentity {
                origin: "https://music.youtube.com".to_owned(),
                user_agent: "Mozilla/5.0 Fake".to_owned(),
                language: "da-DK".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_any_other_script_result() {
        for result in [
            "",
            "null",
            "{}",
            r#""text""#,
            r#"["https://music.youtube.com","agent"]"#,
            r#"["https://music.youtube.com",null,"da"]"#,
        ] {
            assert_eq!(page_identity_from(result), None, "{result:?}");
        }
    }

    /// A guard: remote pages must never get IPC permissions.
    #[test]
    fn no_capability_names_the_login_window() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let read = |path: &Path| -> Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let files: Vec<_> = std::fs::read_dir(manifest.join("capabilities"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(!files.is_empty());
        for file in files {
            let capability = read(&file);
            // Exact labels only: a pattern or a remote URL could take it in.
            assert_eq!(
                capability["windows"],
                serde_json::json!(["main"]),
                "{file:?}"
            );
            for key in ["webviews", "remote"] {
                assert_eq!(capability.get(key), None, "{key} in {file:?}");
            }
        }
        let config = read(&manifest.join("tauri.conf.json"));
        assert_eq!(config["app"]["security"].get("capabilities"), None);
        assert_ne!(LOGIN_WINDOW, "main");
    }
}
