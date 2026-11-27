//! The YouTube Music status commands and their event.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::account::{self, AccountHandle};
use super::{IpcError, MAIN_WINDOW};
use crate::secrets::WindowsStore;
use crate::sources::youtube::{
    Account, CREDENTIAL_SERVICE, HttpTransport, ServiceStatus, SystemClock, YouTube,
};

/// Sent when the gate or the session check changes. Mirrored in `src/lib/ipc.ts`.
pub const YOUTUBE_STATUS_CHANGED: &str = "youtube-status-changed";

/// Where the gate's state is kept, in the app's local data folder.
const STATE_FILE: &str = "youtube-gate.json";

/// YouTube Music as the app runs it.
pub type Service = YouTube<WindowsStore, HttpTransport, SystemClock>;

/// Takes up the saved gate state and starts passing status changes to the UI.
/// Sends nothing to YouTube.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let state_file = app.path().app_local_data_dir()?.join(STATE_FILE);
    let announcer = app.clone();
    let service = Arc::new(Service::new(
        Account::new(WindowsStore::new(CREDENTIAL_SERVICE)),
        HttpTransport::default(),
        SystemClock::default(),
        state_file,
        move |status| account::announce(&announcer, status),
    ));
    app.manage(AccountHandle::new(Arc::clone(&service)));
    let sink = app.clone();
    tauri::async_runtime::spawn(async move {
        service
            .forward(move |status| {
                // A failure means the window is gone: nobody is left to tell.
                let _ = sink.emit_to(MAIN_WINDOW, YOUTUBE_STATUS_CHANGED, status);
            })
            .await;
    });
    Ok(())
}

#[tauri::command]
pub async fn get_youtube_status(
    handle: State<'_, AccountHandle>,
) -> Result<ServiceStatus, IpcError> {
    Ok(handle.youtube().status())
}

/// Ends a pause. Sends nothing to YouTube.
#[tauri::command]
pub async fn resume_youtube(handle: State<'_, AccountHandle>) -> Result<(), IpcError> {
    handle.youtube().resume();
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::sources::youtube::{Check, GateView, PauseReason, ServiceStatus};

    /// Mirrored in `src/lib/ipc.ts`.
    #[test]
    fn the_status_has_the_shape_the_ui_expects() {
        let cooling = GateView::CoolingDown { until_ms: 5 };
        let paused = GateView::Paused {
            reason: PauseReason::RepeatedFailures,
        };
        for (gate, check, gate_shape, check_name) in [
            (
                GateView::Ready,
                Check::Unchecked,
                json!({ "state": "ready" }),
                "unchecked",
            ),
            (
                cooling,
                Check::Checking,
                json!({ "state": "coolingDown", "untilMs": 5 }),
                "checking",
            ),
            (
                paused,
                Check::Verified,
                json!({ "state": "paused", "reason": "repeatedFailures" }),
                "verified",
            ),
            (
                GateView::Ready,
                Check::Offline,
                json!({ "state": "ready" }),
                "offline",
            ),
            (
                GateView::Ready,
                Check::Unexpected,
                json!({ "state": "ready" }),
                "unexpected",
            ),
        ] {
            let status = ServiceStatus {
                revision: 3,
                gate,
                check,
            };

            assert_eq!(
                serde_json::to_value(status).unwrap(),
                json!({ "revision": 3, "gate": gate_shape, "check": check_name })
            );
        }
    }
}
