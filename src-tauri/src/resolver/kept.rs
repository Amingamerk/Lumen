//! Streams kept in the library, so they outlive the app.

use std::sync::Arc;

use super::StreamInfo;
use crate::library::Library;

/// The library as the resolver uses it. Every failure ends as "nothing kept":
/// playback never waits on it.
#[derive(Clone)]
pub(super) struct Kept {
    _library: Arc<Library>,
}

impl Kept {
    pub fn new(library: Arc<Library>) -> Self {
        Self { _library: library }
    }

    pub fn load(&self, _login: &str, _video: &str, _needed_ms: u64) -> Option<StreamInfo> {
        None
    }

    pub fn save(&self, _login: &str, _video: &str, _stream: &StreamInfo) {}

    pub fn forget(&self, _video: &str) {}

    pub fn clear(&self) {}
}
