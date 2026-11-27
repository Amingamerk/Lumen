//! What plays when the Queue runs out. A static stand-in picks from the catalog
//! until the real engine arrives (Milestone 6).

use std::cmp::Reverse;
use std::sync::Arc;

use uuid::Uuid;

use crate::catalog::{Catalog, Track};

/// How many of the latest played Tracks a pick tries not to repeat.
pub const RECENT_HISTORY: usize = 20;

/// The seam the player asks for the next autoplay Track.
pub trait Recommender: Send {
    /// A Track to put after the Queue's last one: never `current` or one of
    /// `upcoming`. `history` is what played before, oldest first. `None` when
    /// there is nothing else to play.
    fn recommend(&self, history: &[Uuid], current: Uuid, upcoming: &[Uuid]) -> Option<Uuid>;
}

/// Picks from the catalog, the same way every time: a Track that did not play
/// lately, preferring the artist and then the source of the Track it follows.
/// When all have played lately, the one heard longest ago.
pub struct StaticRecommender {
    catalog: Arc<Catalog>,
}

impl StaticRecommender {
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self { catalog }
    }
}

impl Recommender for StaticRecommender {
    fn recommend(&self, history: &[Uuid], current: Uuid, upcoming: &[Uuid]) -> Option<Uuid> {
        let follows = self
            .catalog
            .track(upcoming.last().copied().unwrap_or(current))?;
        let recent = &history[history.len().saturating_sub(RECENT_HISTORY)..];
        self.catalog
            .recommendable()
            .iter()
            .filter(|track| track.id != current && !upcoming.contains(&track.id))
            // The first of equals wins, so ties go to catalog order.
            .min_by_key(|track| {
                // `None`, not played lately, sorts first; then the oldest play.
                let played = recent.iter().rposition(|id| *id == track.id);
                (played, Reverse(likeness(follows, track)))
            })
            .map(|track| track.id)
    }
}

/// Whether two Tracks share an artist, then whether they share a source. A
/// greater value is more alike.
fn likeness(a: &Track, b: &Track) -> (bool, bool) {
    let artist = a.artists.iter().any(|name| b.artists.contains(name));
    let source = a
        .sources
        .iter()
        .any(|x| b.sources.iter().any(|y| x.kind == y.kind));
    (artist, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{SourceKind, SourceRef, Track};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn ids(numbers: &[u128]) -> Vec<Uuid> {
        numbers.iter().copied().map(id).collect()
    }

    fn track(n: u128, artist: &str, kind: SourceKind) -> Track {
        Track {
            id: id(n),
            title: format!("Track {n}"),
            artists: vec![artist.to_owned()],
            album: None,
            duration_ms: 180_000,
            artwork_url: None,
            isrc: None,
            sources: vec![SourceRef {
                kind,
                id: format!("ref-{n}"),
            }],
        }
    }

    fn recommender() -> StaticRecommender {
        use SourceKind::{SoundCloud, YouTube};

        StaticRecommender::new(Arc::new(Catalog::of(vec![
            track(1, "Ada", YouTube),
            track(2, "Bo", SoundCloud),
            track(3, "Cleo", YouTube),
            track(4, "Ada", SoundCloud),
            track(5, "Bo", YouTube),
        ])))
    }

    fn pick(history: &[u128], current: u128, upcoming: &[u128]) -> Option<u128> {
        recommender()
            .recommend(&ids(history), id(current), &ids(upcoming))
            .map(|id| id.as_u128())
    }

    #[test]
    fn prefers_the_same_artist() {
        assert_eq!(pick(&[], 1, &[]), Some(4));
    }

    #[test]
    fn then_the_same_source_in_catalog_order() {
        assert_eq!(pick(&[4], 1, &[]), Some(3));
    }

    #[test]
    fn follows_the_last_upcoming_track() {
        assert_eq!(pick(&[], 1, &[2]), Some(5));
    }

    #[test]
    fn never_picks_a_queued_track() {
        assert_eq!(pick(&[], 1, &[4, 3, 5]), Some(2));
        assert_eq!(pick(&[], 1, &[2, 3, 4, 5]), None);
    }

    #[test]
    fn repeats_the_one_heard_longest_ago_when_all_played_lately() {
        assert_eq!(pick(&[3, 2, 4, 5], 1, &[]), Some(3));
        // Its latest play is what counts.
        assert_eq!(pick(&[3, 2, 4, 5, 3], 1, &[]), Some(2));
    }

    #[test]
    fn forgets_plays_older_than_the_window() {
        let mut history = ids(&[4]);
        history.extend([id(2); RECENT_HISTORY]);
        let recommender = recommender();

        assert_eq!(recommender.recommend(&history, id(1), &[]), Some(id(4)));
        history.pop();
        assert_eq!(recommender.recommend(&history, id(1), &[]), Some(id(3)));
    }

    #[test]
    fn has_nothing_for_an_empty_catalog() {
        let recommender = StaticRecommender::new(Arc::new(Catalog::of(Vec::new())));

        assert_eq!(recommender.recommend(&[], id(1), &[]), None);
    }

    #[test]
    fn has_nothing_after_the_only_track() {
        let only = Catalog::of(vec![track(1, "Ada", SourceKind::YouTube)]);
        let recommender = StaticRecommender::new(Arc::new(only));

        assert_eq!(recommender.recommend(&[], id(1), &[]), None);
        assert_eq!(recommender.recommend(&[id(1)], id(1), &[]), None);
    }

    #[test]
    fn a_long_run_stays_off_generated_tracks_and_recent_ones() {
        let recommender = StaticRecommender::new(Arc::new(Catalog::default()));
        // The first Track of the generated playlist.
        let mut current = id(10_000);
        let mut history: Vec<Uuid> = Vec::new();

        for _ in 0..200 {
            let next = recommender.recommend(&history, current, &[]).unwrap();

            assert!(next.as_u128() < 10_000, "picked a generated track");
            assert_ne!(next, current);
            assert!(
                !history
                    .iter()
                    .rev()
                    .take(RECENT_HISTORY)
                    .any(|h| *h == next)
            );
            history.push(current);
            current = next;
        }
    }
}
