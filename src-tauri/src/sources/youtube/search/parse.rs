//! Reads the songs out of a search reply.

use std::collections::HashSet;

use serde_json::Value;
use uuid::Uuid;

use crate::catalog::{SourceKind, SourceRef, Track};

/// Stands between the artists, the album and the duration.
const SEPARATOR: char = '•';

/// What a search reply Lumen understands holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Songs(Vec<Track>),
    NoResults,
}

/// `None` for a reply with a layout Lumen does not know. Continuations are
/// never read.
///
/// Nothing here matches on words, so every interface language reads the same.
pub fn songs(reply: &Value) -> Option<Parsed> {
    let sections = reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
        ["content"]["sectionListRenderer"]["contents"]
        .as_array()?;
    let mut tracks = Vec::new();
    let mut seen = HashSet::new();
    let mut rows = false;
    let mut nothing_found = false;
    for section in sections {
        // Songs come in a shelf, or in a plain item section. Anything else,
        // such as "showing results for", is passed over.
        let items = section["musicShelfRenderer"]["contents"]
            .as_array()
            .or_else(|| section["itemSectionRenderer"]["contents"].as_array());
        for item in items.into_iter().flatten() {
            nothing_found |= item["messageRenderer"].is_object();
            let row = &item["musicResponsiveListItemRenderer"];
            if !row.is_object() {
                continue;
            }
            rows = true;
            if let Some((video, track)) = song(row)
                && seen.insert(video)
            {
                tracks.push(track);
            }
        }
    }
    if !tracks.is_empty() {
        Some(Parsed::Songs(tracks))
    } else {
        (nothing_found && !rows).then_some(Parsed::NoResults)
    }
}

/// One row, with its video id. `None` without an id, title, artist or duration.
fn song(row: &Value) -> Option<(String, Track)> {
    let video = row["playlistItemData"]["videoId"].as_str().or_else(|| {
        row["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
            ["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
            .as_str()
    })?;
    if !is_video_id(video) {
        return None;
    }
    let columns = row["flexColumns"].as_array()?;
    let title: String = runs(columns.first()?).filter_map(text).collect();
    let details: Vec<&Value> = runs(columns.get(1)?).collect();
    let mut artists: Vec<String> = linked(&details, "_ARTIST").map(str::to_owned).collect();
    if artists.is_empty() {
        // Unlinked artists are everything before the first separator.
        let line: String = details.iter().filter_map(|&run| text(run)).collect();
        let first = line.split(SEPARATOR).next().unwrap_or_default().trim();
        if !first.is_empty() && duration_ms(first).is_none() {
            artists.push(first.to_owned());
        }
    }
    let album = linked(&details, "_ALBUM").next().map(str::to_owned);
    let duration_ms = columns.iter().skip(1).find_map(|column| {
        let line: String = runs(column).filter_map(text).collect();
        line.rsplit(SEPARATOR)
            .find_map(|part| duration_ms(part.trim()))
    })?;
    if title.trim().is_empty() || artists.is_empty() {
        return None;
    }
    let artwork_url = row["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()
        // The last one is the largest.
        .and_then(|sizes| sizes.last())
        .and_then(|size| size["url"].as_str())
        .map(str::to_owned);
    let track = Track {
        id: Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("youtube:{video}").as_bytes()),
        title,
        artists,
        album,
        duration_ms,
        artwork_url,
        isrc: None,
        sources: vec![SourceRef {
            kind: SourceKind::YouTube,
            id: video.to_owned(),
        }],
    };
    Some((video.to_owned(), track))
}

fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The pieces of text in a flex column.
fn runs(column: &Value) -> impl Iterator<Item = &Value> {
    column["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
        .as_array()
        .into_iter()
        .flatten()
}

fn text(run: &Value) -> Option<&str> {
    run["text"].as_str()
}

/// The texts that link to a page whose type ends in `kind`.
fn linked<'a>(runs: &[&'a Value], kind: &str) -> impl Iterator<Item = &'a str> {
    runs.iter().filter_map(move |&run| {
        let page = run["navigationEndpoint"]["browseEndpoint"]
            ["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]
            ["pageType"]
            .as_str()?;
        page.ends_with(kind).then(|| text(run)).flatten()
    })
}

/// `m:ss` or `h:mm:ss`; some languages write full stops instead of colons.
fn duration_ms(text: &str) -> Option<u32> {
    let separator = if text.contains(':') { ':' } else { '.' };
    let parts: Vec<&str> = text.split(separator).collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut seconds: u32 = 0;
    for (at, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let value: u32 = part.parse().ok()?;
        if at > 0 && (part.len() != 2 || value >= 60) {
            return None;
        }
        seconds = seconds.checked_mul(60)?.checked_add(value)?;
    }
    seconds.checked_mul(1_000)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parsed(fixture: &str) -> Option<Parsed> {
        songs(&serde_json::from_str(fixture).unwrap())
    }

    fn found(fixture: &str) -> Vec<Track> {
        match parsed(fixture) {
            Some(Parsed::Songs(tracks)) => tracks,
            other => panic!("expected songs, got {other:?}"),
        }
    }

    const SONGS: &str = include_str!("../fixtures/songs.json");

    #[test]
    fn a_song_has_its_title_artists_album_duration_and_video() {
        let first = found(SONGS).remove(0);

        assert_eq!(first.title, "Paper Lanterns");
        assert_eq!(first.artists, ["Ada Brook", "The Tin Owls"]);
        assert_eq!(first.album.as_deref(), Some("Night Market"));
        assert_eq!(first.duration_ms, 194_000);
        assert_eq!(first.isrc, None);
        assert_eq!(
            first.artwork_url.as_deref(),
            Some("https://art.invalid/fakeVideo01-120.jpg")
        );
        assert_eq!(
            first.sources,
            [SourceRef {
                kind: SourceKind::YouTube,
                id: "fakeVideo01".to_owned(),
            }]
        );
        // The same video is the same Track every time it is found.
        assert_eq!(first.id, found(SONGS)[0].id);
        assert_eq!(first.id.get_version_num(), 5);
        assert_ne!(first.id, found(SONGS)[1].id);
    }

    #[test]
    fn unlinked_artists_and_a_missing_album_are_read() {
        let second = found(SONGS).remove(1);

        assert_eq!(second.title, "Salt Road");
        assert_eq!(second.artists, ["Nils Harbor"]);
        assert_eq!(second.album, None);
        assert_eq!(second.duration_ms, 245_000);
        assert_eq!(second.sources[0].id, "fakeVideo02");
    }

    #[test]
    fn unusable_rows_and_repeats_are_left_out() {
        let tracks = found(SONGS);

        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Paper Lanterns", "Salt Road", "Long Tide"]);
        assert_eq!(tracks[2].duration_ms, 3_723_000);
        assert_eq!(tracks[2].artwork_url, None);
    }

    #[test]
    fn danish_durations_use_full_stops() {
        let tracks = found(include_str!("../fixtures/songs_da.json"));

        let read: Vec<(&str, u32)> = tracks
            .iter()
            .map(|t| (t.artists[0].as_str(), t.duration_ms))
            .collect();
        assert_eq!(read, [("Søren Kværn", 187_000), ("Liv og Mark", 3_609_000)]);
    }

    #[test]
    fn only_a_clock_time_is_a_duration() {
        for text in [
            "3", "2024", "3:7", "3:60", "1:60:00", "a:00", ":30", "1:2:3:4", "12K",
        ] {
            assert_eq!(duration_ms(text), None, "{text}");
        }
        assert_eq!(duration_ms("0:05"), Some(5_000));
        assert_eq!(duration_ms("99999999:00"), None);
    }

    #[test]
    fn an_empty_answer_and_a_corrected_query_are_understood() {
        assert_eq!(
            parsed(include_str!("../fixtures/no_results.json")),
            Some(Parsed::NoResults)
        );
        assert_eq!(
            found(include_str!("../fixtures/corrected_query.json")).len(),
            1
        );
    }

    /// The newer layout ytmusicapi also reads: rows in a plain item section.
    #[test]
    fn rows_in_an_item_section_are_read() {
        let mut reply: Value = serde_json::from_str(SONGS).unwrap();
        let sections = &mut reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
            ["content"]["sectionListRenderer"]["contents"];
        let rows = sections[0]["musicShelfRenderer"]["contents"].take();
        sections[0] = json!({ "itemSectionRenderer": { "contents": rows } });

        assert!(matches!(songs(&reply), Some(Parsed::Songs(tracks)) if tracks.len() == 3));
    }

    #[test]
    fn anything_else_is_unrecognised() {
        let in_sections = |sections: Value| {
            json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [
                { "tabRenderer": { "content": {
                    "sectionListRenderer": { "contents": sections }
                } } }
            ] } } })
        };
        for reply in [
            serde_json::from_str(include_str!("../fixtures/unknown_layout.json")).unwrap(),
            json!({}),
            in_sections(json!([])),
            in_sections(json!([{ "musicShelfRenderer": { "contents": [] } }])),
            // Rows, but not one that can be used.
            in_sections(json!([{ "musicShelfRenderer": { "contents": [
                { "musicResponsiveListItemRenderer": {} }
            ] } }])),
            in_sections(json!([{ "itemSectionRenderer": { "contents": [
                { "somethingElse": {} }
            ] } }])),
        ] {
            assert_eq!(songs(&reply), None, "{reply}");
        }
    }

    /// The fixtures are written by hand and may never hold account data.
    #[test]
    fn no_fixture_holds_account_data() {
        let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/src/sources/youtube/fixtures");
        let mut seen = 0;
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for banned in [
                "visitorData",
                "trackingParams",
                "clickTrackingParams",
                "SAPISID",
                "LOGIN_INFO",
                "datasyncId",
                "obfuscatedGaiaId",
                "@",
                "googleusercontent",
            ] {
                assert!(!text.contains(banned), "{} holds {banned}", path.display());
            }
            seen += 1;
        }
        assert!(seen >= 6, "only {seen} fixtures");
    }
}
