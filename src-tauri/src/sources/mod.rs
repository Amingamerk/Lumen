//! The music services. Each one stays in its own module, so a break in one does
//! not take down the others.

use std::future::Future;

use crate::catalog::{SourceRef, Track};

pub mod youtube;

/// Why a source gave no tracks. Fixed texts only.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum SourceError {
    /// This source cannot do that yet. Nothing was sent.
    // Nothing asks for radio seeds before Milestone 6.
    #[cfg_attr(not(test), allow(dead_code))]
    #[error("this service cannot do that yet")]
    Unsupported,
    #[error(transparent)]
    Service(#[from] youtube::ServiceError),
}

/// The metadata role of a service: finding Tracks. It never produces audio; that
/// is the resolver's job.
pub trait Source: Send + Sync {
    /// At most `limit` songs matching `query`. Sends nothing for a blank query,
    /// or if `wanted` says no once it is this search's turn.
    fn search(
        &self,
        query: &str,
        limit: usize,
        wanted: impl Fn() -> bool + Send + Sync,
    ) -> impl Future<Output = Result<Vec<Track>, SourceError>> + Send;

    /// Songs like `reference`, as radio seeds.
    // Nothing asks for them before Milestone 6.
    #[cfg_attr(not(test), allow(dead_code))]
    fn related(
        &self,
        reference: &SourceRef,
    ) -> impl Future<Output = Result<Vec<Track>, SourceError>> + Send;
}

/// A stand-in for a service, for the tests of the modules that use sources.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use super::*;

    /// Answers every search the same way and remembers what was asked.
    pub struct FakeSource {
        answer: Result<Vec<Track>, SourceError>,
        searched: Mutex<Vec<String>>,
    }

    impl FakeSource {
        pub fn answering(answer: Result<Vec<Track>, SourceError>) -> Self {
            Self {
                answer,
                searched: Mutex::default(),
            }
        }

        /// The queries it was asked, in order.
        pub fn searched(&self) -> Vec<String> {
            self.searched.lock().unwrap().clone()
        }
    }

    impl Source for FakeSource {
        async fn search(
            &self,
            query: &str,
            limit: usize,
            wanted: impl Fn() -> bool + Send + Sync,
        ) -> Result<Vec<Track>, SourceError> {
            self.searched.lock().unwrap().push(query.to_owned());
            if !wanted() {
                return Err(youtube::ServiceError::Gate(youtube::GateError::Superseded).into());
            }
            let mut tracks = self.answer.clone()?;
            tracks.truncate(limit);
            Ok(tracks)
        }

        async fn related(&self, _reference: &SourceRef) -> Result<Vec<Track>, SourceError> {
            Err(SourceError::Unsupported)
        }
    }
}
