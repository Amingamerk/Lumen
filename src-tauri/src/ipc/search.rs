//! The live search commands and their event. The rules live in `state`.

use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime, State};

use super::account::AccountHandle;
use super::{EventSink, IpcError, SharedState, lock};
use crate::catalog::{SearchView, SourceFilter, Track};
use crate::sources::youtube::{Parsed, ServiceError};
use crate::sources::{Source, SourceError};
use crate::state::Ticket;

/// Sent when the search view changes. Mirrored in `src/lib/ipc.ts`.
pub const SEARCH_CHANGED: &str = "search-changed";

/// How many songs one search shows.
const SEARCH_LIMIT: usize = 50;

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
    match search_with(&*youtube, &state, &ticket).await {
        Some(view) => app.search_changed(&view),
        None => Ok(()),
    }
}

/// Asks `source` for the ticket's query and takes up the answer. Nothing is
/// shown for a search that a newer one has replaced.
async fn search_with(
    source: &impl Source,
    state: &SharedState,
    ticket: &Ticket,
) -> Option<SearchView> {
    let wanted = || lock(state).is_newest_search(ticket.number);
    let outcome = source.search(&ticket.query, SEARCH_LIMIT, wanted).await;
    lock(state).finish_search(ticket.number, found(outcome))
}

/// What the search state takes: no songs is an answer, a source that cannot
/// search is an unexpected one.
fn found(outcome: Result<Vec<Track>, SourceError>) -> Result<Parsed, ServiceError> {
    match outcome {
        Ok(tracks) if tracks.is_empty() => Ok(Parsed::NoResults),
        Ok(tracks) => Ok(Parsed::Songs(tracks)),
        Err(SourceError::Service(error)) => Err(error),
        Err(SourceError::Unsupported) => Err(ServiceError::Unexpected),
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
    use crate::library::testing::song;
    use crate::sources::testing::FakeSource;

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

    #[tokio::test]
    async fn a_source_that_found_songs_shows_them() {
        let state = SharedState::default();
        let ticket = lock(&state)
            .submit_search("paper", SourceFilter::All)
            .ticket;
        let source = FakeSource::answering(Ok(vec![song(1), song(2)]));

        let view = search_with(&source, &state, &ticket.unwrap())
            .await
            .unwrap();

        assert_eq!(view.status, SearchStatus::Results);
        assert_eq!(view.tracks.len(), 2);
        assert_eq!(source.searched(), ["paper"]);
    }

    #[tokio::test]
    async fn a_source_with_no_songs_says_so() {
        let state = SharedState::default();
        let ticket = lock(&state).submit_search("zzz", SourceFilter::All).ticket;
        let source = FakeSource::answering(Ok(Vec::new()));

        let view = search_with(&source, &state, &ticket.unwrap())
            .await
            .unwrap();

        assert_eq!(view.status, SearchStatus::NoResults);
    }

    #[tokio::test]
    async fn a_refusal_blocks_the_search() {
        let state = SharedState::default();
        let ticket = lock(&state)
            .submit_search("paper", SourceFilter::All)
            .ticket;
        let source = FakeSource::answering(Err(ServiceError::RateLimited.into()));

        let view = search_with(&source, &state, &ticket.unwrap())
            .await
            .unwrap();

        assert_eq!(view.status, SearchStatus::Blocked);
    }
}
