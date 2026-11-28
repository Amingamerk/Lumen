//! Streams kept in the library, so they outlive the app.

use std::sync::Arc;

use super::StreamInfo;
use crate::catalog::{SourceKind, SourceRef};
use crate::library::Library;

/// The library as the resolver uses it. Every failure ends as "nothing kept":
/// playback never waits on it, and no reason is passed on, because an error
/// could quote what the file holds.
#[derive(Clone)]
pub(super) struct Kept {
    library: Arc<Library>,
}

fn youtube(video: &str) -> SourceRef {
    SourceRef {
        kind: SourceKind::YouTube,
        id: video.to_owned(),
    }
}

impl Kept {
    pub fn new(library: Arc<Library>) -> Self {
        Self { library }
    }

    /// The stream of `video` kept for `login`, with `needed_ms` still ahead of it.
    pub fn load(&self, login: &str, video: &str, needed_ms: u64) -> Option<StreamInfo> {
        let stored = self
            .library
            .stream(login, &youtube(video), needed_ms)
            .ok()??;
        StreamInfo::from_stored(stored)
    }

    pub fn save(&self, login: &str, video: &str, stream: &StreamInfo) {
        if let Some(stored) = stream.to_stored() {
            // Nothing to do if it fails: the stream is resolved again later.
            let _ = self.library.keep_stream(login, &youtube(video), &stored);
        }
    }

    pub fn forget(&self, video: &str) {
        let _ = self.library.forget_stream(&youtube(video));
    }

    pub fn clear(&self) {
        let _ = self.library.forget_streams();
    }
}
