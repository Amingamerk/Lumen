//! The live search commands and their event. The rules live in `state`.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime, State};

use super::account::AccountHandle;
use super::{EventSink, IpcError, SharedState, lock};
use crate::catalog::SourceFilter;
use crate::state::Ticket;

/// Sent when the search view changes. Mirrored in `src/lib/ipc.ts`.
pub const SEARCH_CHANGED: &str = "search-changed";

/// Takes up an Enter on the Search view and announces it.
fn begin(
    state: &SharedState,
    sink: &impl EventSink,
    query: &str,
    filter: SourceFilter,
) -> Result<Option<Ticket>, IpcError> {
    // The lock is released before the event goes out. The revision keeps order.
    let submitted = lock(state).submit_search(query, filter);
    if let Some(view) = submitted.view {
        sink.search_changed(&view)?;
    }
    Ok(submitted.ticket)
}

/// Enter on the Search view: the only thing that sends a search. The answer to
/// the command is the events.
#[tauri::command]
pub async fn submit_search<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    account: State<'_, AccountHandle>,
    query: String,
    filter: SourceFilter,
) -> Result<(), IpcError> {
    let Some(ticket) = begin(&state, &app, &query, filter)? else {
        return Ok(());
    };
    let youtube = Arc::clone(account.youtube());
    let wanted = || lock(&state).is_newest_search(ticket.number);
    let outcome = youtube.search_songs(&ticket.query, wanted).await;
    let view = lock(&state).finish_search(ticket.number, outcome);
    match view {
        Some(view) => app.search_changed(&view),
        None => Ok(()),
    }
}

/// A chip change. Sends nothing to YouTube.
#[tauri::command]
pub async fn set_search_filter<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, SharedState>,
    account: State<'_, AccountHandle>,
    filter: SourceFilter,
) -> Result<(), IpcError> {
    let query = lock(&state).submitted_query();
    // Read without the app's lock: an account change takes that lock too.
    let cached = account.youtube().cached_songs(&query);
    let view = lock(&state).set_search_filter(filter, &query, cached);
    app.search_changed(&view)
}

/// Empties the live results after the account has changed.
pub fn account_changed<R: Runtime>(app: &AppHandle<R>) {
    let view = lock(app.state::<SharedState>().inner()).forget_live_search();
    if let Some(view) = view {
        // A failure means the window is gone: nobody is left to tell.
        let _ = app.search_changed(&view);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::RecordingSink;
    use super::*;
    use crate::catalog::SearchStatus;

    #[test]
    fn a_submit_announces_that_it_is_loading() {
        let state = SharedState::default();
        let sink = RecordingSink::default();

        let ticket = begin(&state, &sink, "paper", SourceFilter::All).unwrap();

        assert!(ticket.is_some());
        let announced = sink.searches.borrow();
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].status, SearchStatus::Loading);
    }

    #[test]
    fn a_repeat_of_the_pending_query_announces_nothing() {
        let state = SharedState::default();
        let sink = RecordingSink::default();
        begin(&state, &sink, "paper", SourceFilter::All).unwrap();

        let again = begin(&state, &sink, "paper", SourceFilter::All).unwrap();

        assert_eq!(again, None);
        assert_eq!(sink.searches.borrow().len(), 1);
    }
}
