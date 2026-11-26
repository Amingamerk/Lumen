//! Tauri commands. Cookies only ever travel inwards; the UI gets [`SessionStatus`] and reports.
//! All commands are `async`: window and cookie calls deadlock in synchronous ones on Windows.

use tauri::{AppHandle, Emitter, Manager, State};
use ytm_login_spike::{
    account::CheckReport,
    cookies::parse_import,
    resolver::Clients,
    session::{CaptureMethod, SessionStatus},
};

use crate::{App, SESSION_CHANGED, webview};

type CommandResult<T> = Result<T, String>;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn announce(app: &AppHandle) -> CommandResult<()> {
    let status = app.state::<App>().status().await.map_err(text)?;
    app.emit(SESSION_CHANGED, status).map_err(text)
}

#[tauri::command]
pub async fn session_status(account: State<'_, App>) -> CommandResult<Option<SessionStatus>> {
    account.status().await.map_err(text)
}

#[tauri::command]
pub async fn open_login(app: AppHandle) -> CommandResult<()> {
    webview::open_login(&app).map_err(text)
}

/// Manual fallback if the automatic capture does not fire.
#[tauri::command]
pub async fn capture_now(app: AppHandle) -> CommandResult<SessionStatus> {
    webview::capture(&app).await
}

/// Imports a `cookies.txt` or `Cookie:` header. The User-Agent must be that of the browser the
/// cookies came from, so the session keeps one device identity (PROJECT.md 3c).
#[tauri::command]
pub async fn import_session(
    app: AppHandle,
    cookies: String,
    user_agent: String,
) -> CommandResult<SessionStatus> {
    let cookies = parse_import(&cookies).map_err(text)?;
    let status = app
        .state::<App>()
        .save(cookies, &user_agent, CaptureMethod::Import)
        .await
        .map_err(text)?;
    announce(&app).await?;
    Ok(status)
}

#[tauri::command]
pub async fn clear_session(app: AppHandle) -> CommandResult<()> {
    // Close first, so an open login window cannot save the session again.
    if let Some(login) = app.get_webview_window(webview::LOGIN_LABEL) {
        login.destroy().map_err(text)?;
    }
    app.state::<App>().clear().await.map_err(text)?;
    announce(&app).await
}

#[tauri::command]
pub async fn run_check(
    app: AppHandle,
    track_url: String,
    clients: Clients,
) -> CommandResult<CheckReport> {
    let report = app.state::<App>().check(&track_url, clients).await;
    // A 401 or 403 marks the session expired.
    announce(&app).await?;
    Ok(report)
}
