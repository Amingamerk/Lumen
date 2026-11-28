//! What a live search finds: songs, and the albums, artists and playlists
//! beside them, plus the chips that narrow a search to one type.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Track;

/// The type chip chosen on the Search view. Each one is a request of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    #[default]
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchKind {
    /// A fixed name for the cache to key by.
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Songs => "songs",
            Self::Albums => "albums",
            Self::Artists => "artists",
            Self::Playlists => "playlists",
        }
    }
}

/// What a search result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HitKind {
    Song,
    Album,
    Artist,
    Playlist,
}

/// One result that is not played from the list: an album, an artist or a
/// playlist. A top result that is a song is a Hit too, with its Track's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Names its picture for `lumen://art`, like a Track's id does.
    pub id: Uuid,
    pub kind: HitKind,
    pub title: String,
    /// The artists of an album or song, or the author of a playlist.
    pub by: Vec<String>,
    /// What else the service said, such as an album's year or a playlist's size.
    pub detail: Option<String>,
    pub artwork_url: Option<String>,
}

/// The best match, with the songs that can be played from its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top {
    pub best: Hit,
    /// At most [`QUICK_SONGS`].
    pub songs: Vec<Track>,
}

/// How many songs the Top Result card offers.
pub const QUICK_SONGS: usize = 3;

/// Everything one search found. A chip leaves all but its own type empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Results {
    pub top: Option<Top>,
    pub songs: Vec<Track>,
    pub albums: Vec<Hit>,
    pub artists: Vec<Hit>,
    pub playlists: Vec<Hit>,
}

impl Results {
    #[cfg(test)]
    pub fn of_songs(songs: Vec<Track>) -> Self {
        Self {
            songs,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.top.is_none()
            && self.songs.is_empty()
            && self.albums.is_empty()
            && self.artists.is_empty()
            && self.playlists.is_empty()
    }

    /// At most `limit` of each list.
    pub fn truncate(&mut self, limit: usize) {
        self.songs.truncate(limit);
        self.albums.truncate(limit);
        self.artists.truncate(limit);
        self.playlists.truncate(limit);
    }

    /// The song with this id, wherever the results show it.
    pub fn track(&self, id: Uuid) -> Option<&Track> {
        let top = self.top.iter().flat_map(|top| &top.songs);
        self.songs.iter().chain(top).find(|track| track.id == id)
    }

    /// Where the picture of this song or hit lives.
    pub fn artwork(&self, id: Uuid) -> Option<&str> {
        if let Some(track) = self.track(id) {
            return track.artwork_url.as_deref();
        }
        let top = self.top.iter().map(|top| &top.best);
        let hits = self
            .albums
            .iter()
            .chain(&self.artists)
            .chain(&self.playlists);
        let hit = top.chain(hits).find(|hit| hit.id == id)?;
        hit.artwork_url.as_deref()
    }
}

/// A Hit as a row or a card shows it. The picture's address stays in Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitRow {
    pub id: Uuid,
    pub kind: HitKind,
    pub title: String,
    pub by: Vec<String>,
    pub detail: Option<String>,
    pub has_art: bool,
}

impl From<&Hit> for HitRow {
    fn from(hit: &Hit) -> Self {
        Self {
            id: hit.id,
            kind: hit.kind,
            title: hit.title.clone(),
            by: hit.by.clone(),
            detail: hit.detail.clone(),
            has_art: hit.artwork_url.is_some(),
        }
    }
}

/// The Top Result card: the best match and its quick-play songs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopView {
    pub best: HitRow,
    pub songs: Vec<super::TrackRow>,
}

impl From<&Top> for TopView {
    fn from(top: &Top) -> Self {
        Self {
            best: HitRow::from(&top.best),
            songs: top.songs.iter().map(super::TrackRow::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn chips_use_lowercase_names_on_the_wire() {
        for (kind, wire) in [
            (SearchKind::All, "all"),
            (SearchKind::Songs, "songs"),
            (SearchKind::Albums, "albums"),
            (SearchKind::Artists, "artists"),
            (SearchKind::Playlists, "playlists"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<SearchKind>(json!(wire)).unwrap(),
                kind
            );
            assert_eq!(kind.name(), wire);
        }
        assert!(serde_json::from_value::<SearchKind>(json!("videos")).is_err());
    }

    #[test]
    fn nothing_found_is_empty() {
        assert!(Results::default().is_empty());
    }
}
