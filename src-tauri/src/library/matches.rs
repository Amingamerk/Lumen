//! The match cache: which YouTube (or SoundCloud) item plays a Spotify song, and
//! how sure the match was. Spotify import (Milestone 5) fills it.

// Nothing reads the cache until Spotify import arrives.
#![cfg_attr(not(test), allow(dead_code))]

use rusqlite::{OptionalExtension, params};

use super::{Library, LibraryError, tracks};
use crate::catalog::SourceRef;

/// How well a candidate fit, each part from 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchScores {
    pub artist: f64,
    pub title: f64,
    pub duration: f64,
    /// What the weights made of the three.
    pub overall: f64,
}

/// A song of one service, and the playable item that stands in for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub from: SourceRef,
    pub to: SourceRef,
    pub scores: MatchScores,
}

impl Library {
    /// Keeps `found`, replacing what was kept for the same song.
    pub fn keep_match(&self, found: &Match) -> Result<(), LibraryError> {
        self.conn().execute(
            "INSERT OR REPLACE INTO matches
                (from_kind, from_id, to_kind, to_id,
                 artist_score, title_score, duration_score, overall_score)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                tracks::kind_name(found.from.kind),
                found.from.id,
                tracks::kind_name(found.to.kind),
                found.to.id,
                found.scores.artist,
                found.scores.title,
                found.scores.duration,
                found.scores.overall,
            ],
        )?;
        Ok(())
    }

    /// What is kept for `from`.
    pub fn match_for(&self, from: &SourceRef) -> Result<Option<Match>, LibraryError> {
        let row = self
            .conn()
            .query_row(
                "SELECT to_kind, to_id, artist_score, title_score, duration_score, overall_score
                 FROM matches WHERE from_kind = ?1 AND from_id = ?2",
                params![tracks::kind_name(from.kind), from.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        MatchScores {
                            artist: row.get(2)?,
                            title: row.get(3)?,
                            duration: row.get(4)?,
                            overall: row.get(5)?,
                        },
                    ))
                },
            )
            .optional()?;
        // A kind this Lumen does not know came from a newer one: no match.
        Ok(row.and_then(|(kind, id, scores)| {
            Some(Match {
                from: from.clone(),
                to: SourceRef {
                    kind: tracks::kind_of(&kind)?,
                    id,
                },
                scores,
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{memory, reopen};
    use super::*;
    use crate::catalog::SourceKind;

    fn found() -> Match {
        Match {
            from: SourceRef {
                kind: SourceKind::Spotify,
                id: "4uLU6hMCjMI75M1A2tKUQC".to_owned(),
            },
            to: SourceRef {
                kind: SourceKind::YouTube,
                id: "dQw4w9WgXcQ".to_owned(),
            },
            scores: MatchScores {
                artist: 100.0,
                title: 91.5,
                duration: 74.081_822_068_171_79,
                overall: 87.25,
            },
        }
    }

    #[test]
    fn a_match_comes_back_with_its_scores_after_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lumen.db");
        let first = reopen(&path);
        first.keep_match(&found()).unwrap();
        drop(first);

        let again = reopen(&path).match_for(&found().from).unwrap();

        assert_eq!(again, Some(found()));
    }

    #[test]
    fn a_later_match_replaces_the_one_kept_before() {
        let (library, _) = memory();
        library.keep_match(&found()).unwrap();
        let better = Match {
            to: SourceRef {
                kind: SourceKind::SoundCloud,
                id: "42".to_owned(),
            },
            ..found()
        };

        library.keep_match(&better).unwrap();

        assert_eq!(library.match_for(&found().from).unwrap(), Some(better));
    }

    #[test]
    fn an_unknown_song_has_no_match() {
        let (library, _) = memory();

        assert_eq!(library.match_for(&found().from).unwrap(), None);
    }
}
