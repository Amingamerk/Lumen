//! Reads what a search reply holds: songs, albums, artists and playlists, and
//! the top result card.

use std::collections::HashSet;

use serde_json::Value;
use uuid::Uuid;

use super::hits::{self, Class};
use crate::catalog::{HitKind, QUICK_SONGS, Results, SearchKind, SourceKind, SourceRef, Track};

/// Stands between the artists, the album and the duration.
pub(super) const SEPARATOR: char = '•';

/// What a search reply Lumen understands holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Found(Box<Results>),
    NoResults,
}

/// `None` for a reply with a layout Lumen does not know. Continuations are
/// never read, and a shelf or row of a shape Lumen does not know is skipped.
///
/// A row is told apart by where it leads and what it plays, never by its words,
/// so every interface language reads the same. Only the unfiltered reply puts a
/// type word ("Song", "Album") first in each row's second line.
pub fn results(reply: &Value, kind: SearchKind) -> Option<Parsed> {
    let sections = reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
        ["content"]["sectionListRenderer"]["contents"]
        .as_array()?;
    let typed = kind == SearchKind::All;
    let mut found = Results::default();
    let mut seen = HashSet::new();
    let mut unusable = false;
    let mut nothing_found = false;
    let mut understood = false;
    for section in sections {
        let card = &section["musicCardShelfRenderer"];
        if card.is_object() {
            if found.top.is_none() {
                found.top = hits::top(card);
            }
            understood |= found.top.is_some();
            continue;
        }
        // Rows come in a shelf, or in a plain item section. Anything else,
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
            match hits::classify(row, typed) {
                None => unusable = true,
                Some(Class::Ignored) => understood = true,
                Some(Class::Song) => match song(row, typed) {
                    Some((video, track)) => {
                        if seen.insert(video) {
                            found.songs.push(track);
                        }
                    }
                    None => unusable = true,
                },
                Some(Class::Hit(kind)) => match hits::hit(row, kind, typed) {
                    Some(hit) => {
                        if seen.insert(hit.id.to_string()) {
                            match kind {
                                HitKind::Album => found.albums.push(hit),
                                HitKind::Artist => found.artists.push(hit),
                                HitKind::Playlist | HitKind::Song => found.playlists.push(hit),
                            }
                        }
                    }
                    None => unusable = true,
                },
            }
        }
    }
    if let Some(top) = &mut found.top {
        top.songs.truncate(QUICK_SONGS);
    }
    if !found.is_empty() {
        Some(Parsed::Found(Box::new(found)))
    } else {
        ((nothing_found || understood) && !unusable).then_some(Parsed::NoResults)
    }
}

/// One song row, with its video id. `None` without an id, title, artist or
/// duration.
pub(super) fn song(row: &Value, typed: bool) -> Option<(String, Track)> {
    let video = video_of(row)?;
    let columns = row["flexColumns"].as_array()?;
    let title: String = runs(columns.first()?).filter_map(text).collect();
    let lines: Vec<Vec<&Value>> = columns.iter().skip(1).map(|c| runs(c).collect()).collect();
    track(video, title, &lines, typed, artwork(row))
}

/// The video a row plays.
pub(super) fn video_of(row: &Value) -> Option<&str> {
    let video = row["playlistItemData"]["videoId"].as_str().or_else(|| {
        row["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
            ["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
            .as_str()
    })?;
    is_video_id(video).then_some(video)
}

/// A Track from a title and its lines of details. The first line names the
/// artists and the album; any line may end with the duration. `typed` means the
/// first line starts with a type word.
pub(super) fn track(
    video: &str,
    title: String,
    lines: &[Vec<&Value>],
    typed: bool,
    artwork_url: Option<String>,
) -> Option<(String, Track)> {
    let details = lines.first()?;
    let mut artists: Vec<String> = linked(details, "_ARTIST").map(str::to_owned).collect();
    if artists.is_empty() {
        // Unlinked artists are everything before the first separator, after the
        // type word.
        let line: String = details.iter().filter_map(|&run| text(run)).collect();
        let first = line
            .split(SEPARATOR)
            .nth(usize::from(typed))
            .unwrap_or_default()
            .trim();
        if !first.is_empty() && duration_ms(first).is_none() {
            artists.push(first.to_owned());
        }
    }
    let album = linked(details, "_ALBUM").next().map(str::to_owned);
    let duration_ms = lines.iter().find_map(|line| {
        let line: String = line.iter().filter_map(|&run| text(run)).collect();
        line.rsplit(SEPARATOR)
            .find_map(|part| duration_ms(part.trim()))
    })?;
    if title.trim().is_empty() || artists.is_empty() {
        return None;
    }
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

/// The largest picture of a row or card.
pub(super) fn artwork(item: &Value) -> Option<String> {
    item["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()
        // The last one is the largest.
        .and_then(|sizes| sizes.last())
        .and_then(|size| size["url"].as_str())
        .map(str::to_owned)
}

fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The pieces of text in a flex column.
pub(super) fn runs(column: &Value) -> impl Iterator<Item = &Value> {
    column["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
        .as_array()
        .into_iter()
        .flatten()
}

pub(super) fn text(run: &Value) -> Option<&str> {
    run["text"].as_str()
}

/// The texts that link to a page whose type ends in `kind`.
pub(super) fn linked<'a>(runs: &[&'a Value], kind: &str) -> impl Iterator<Item = &'a str> {
    runs.iter().filter_map(move |&run| {
        let page = run["navigationEndpoint"]["browseEndpoint"]
            ["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]
            ["pageType"]
            .as_str()?;
        page.ends_with(kind).then(|| text(run)).flatten()
    })
}

/// `m:ss` or `h:mm:ss`; some languages write full stops instead of colons.
pub(super) fn duration_ms(text: &str) -> Option<u32> {
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
    use crate::catalog::Hit;

    fn parsed(fixture: &str) -> Option<Parsed> {
        results(&serde_json::from_str(fixture).unwrap(), SearchKind::Songs)
    }

    fn found(fixture: &str) -> Vec<Track> {
        match parsed(fixture) {
            Some(Parsed::Found(found)) => found.songs,
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

        assert!(
            matches!(results(&reply, SearchKind::Songs), Some(Parsed::Found(f)) if f.songs.len() == 3)
        );
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
            assert_eq!(results(&reply, SearchKind::Songs), None, "{reply}");
        }
    }

    const EVERYTHING: &str = include_str!("../fixtures/search_all.json");

    fn everything() -> Results {
        let reply: Value = serde_json::from_str(EVERYTHING).unwrap();
        match results(&reply, SearchKind::All) {
            Some(Parsed::Found(found)) => *found,
            other => panic!("expected results, got {other:?}"),
        }
    }

    fn titles(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.title.as_str()).collect()
    }

    #[test]
    fn a_reply_with_every_shelf_gives_songs_albums_artists_and_playlists() {
        let found = everything();

        // The music video is not a song, and the song that is also on the card
        // is listed once.
        let songs: Vec<&str> = found.songs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(songs, ["Paper Lanterns", "Harbor Lights"]);
        assert_eq!(
            found.songs[1].artists,
            ["Nils Harbor"],
            "after the type word"
        );
        assert_eq!(titles(&found.albums), ["Night Market", "Low Tide"]);
        assert_eq!(titles(&found.artists), ["Ada Brook", "Ada Brooks Band"]);
        assert_eq!(
            titles(&found.playlists),
            ["Ada Brook Essentials", "Rainy day Ada"]
        );

        let album = &found.albums[0];
        assert_eq!(album.kind, HitKind::Album);
        assert_eq!(album.by, ["Ada Brook"]);
        assert_eq!(album.detail.as_deref(), Some("2021"));
        assert_eq!(
            album.artwork_url.as_deref(),
            Some("https://art.invalid/night-226.jpg")
        );
        assert_eq!(found.albums[1].by, ["Ada Brook", "Mira Sol"]);

        let artist = &found.artists[0];
        assert_eq!(artist.kind, HitKind::Artist);
        assert_eq!(artist.detail.as_deref(), Some("1.2M subscribers"));
        assert_eq!(found.artists[1].detail, None);
        assert_eq!(found.artists[1].artwork_url, None);

        let playlist = &found.playlists[0];
        assert_eq!(playlist.kind, HitKind::Playlist);
        assert_eq!(playlist.by, ["Lumen Radio"]);
        assert_eq!(playlist.detail.as_deref(), Some("50 songs"));
        assert_eq!(found.playlists[1].by, ["A listener"]);
    }

    #[test]
    fn a_hit_is_the_same_every_time_and_differs_from_the_next() {
        let (first, second) = (everything(), everything());

        assert_eq!(first.albums[0].id, second.albums[0].id);
        assert_ne!(first.albums[0].id, first.albums[1].id);
        assert_eq!(first.albums[0].id.get_version_num(), 5);
    }

    #[test]
    fn the_top_result_is_the_top_card_with_three_quick_songs() {
        let top = everything().top.expect("a top result");

        assert_eq!(top.best.kind, HitKind::Artist);
        assert_eq!(top.best.title, "Ada Brook");
        assert_eq!(top.best.detail.as_deref(), Some("1.2M subscribers"));
        assert_eq!(
            top.best.artwork_url.as_deref(),
            Some("https://art.invalid/ada-226.jpg")
        );
        let titles: Vec<&str> = top.songs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Paper Lanterns", "Salt Road", "Long Tide"]);
        assert_eq!(top.songs[1].artists, ["Ada Brook"]);
        assert_eq!(top.songs[2].duration_ms, 3_723_000);
    }

    #[test]
    fn a_top_song_is_played_first_from_its_own_card() {
        let mut reply: Value = serde_json::from_str(EVERYTHING).unwrap();
        let card = &mut reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
            ["content"]["sectionListRenderer"]["contents"][0]["musicCardShelfRenderer"];
        card["title"]["runs"][0] = json!({
            "text": "Paper Lanterns",
            "navigationEndpoint": { "watchEndpoint": { "videoId": "fakeVideo01" } }
        });
        card["subtitle"]["runs"] = json!([
            { "text": "Song" }, { "text": " • " }, { "text": "Ada Brook" },
            { "text": " • " }, { "text": "3:14" }
        ]);

        let Some(Parsed::Found(found)) = results(&reply, SearchKind::All) else {
            panic!("expected results");
        };

        let top = found.top.unwrap();
        assert_eq!(top.best.kind, HitKind::Song);
        assert_eq!(top.best.id, top.songs[0].id);
        assert_eq!(top.songs[0].title, "Paper Lanterns");
        // The same song among the card's own rows is not listed twice.
        let titles: Vec<&str> = top.songs.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Paper Lanterns", "Salt Road", "Long Tide"]);
    }

    #[test]
    fn a_reply_without_a_top_card_has_no_top_result() {
        let reply: Value = serde_json::from_str(SONGS).unwrap();

        let Some(Parsed::Found(found)) = results(&reply, SearchKind::Songs) else {
            panic!("expected results");
        };

        assert_eq!(found.top, None);
    }

    #[test]
    fn a_shelf_or_row_of_an_unknown_shape_is_skipped_not_fatal() {
        // The podcast shelf and the unknown carousel are in the fixture; nothing
        // of them shows, and everything else is read.
        let found = everything();
        assert!(
            found
                .albums
                .iter()
                .chain(&found.artists)
                .chain(&found.playlists)
                .all(|hit| !hit.title.contains("Podcast"))
        );
        assert_eq!(
            found.albums.len() + found.artists.len() + found.playlists.len(),
            6
        );

        // Only unknown shelves is an answer Lumen does not understand.
        let only_unknown = json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [
            { "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
                { "musicCarouselShelfRenderer": { "contents": [{ "somethingNew": {} }] } }
            ] } } } }
        ] } } });
        assert_eq!(results(&only_unknown, SearchKind::All), None);
    }

    #[test]
    fn results_of_a_kind_lumen_does_not_show_are_an_answer_with_nothing_in_it() {
        let mut reply: Value = serde_json::from_str(EVERYTHING).unwrap();
        let sections = &mut reply["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
            ["content"]["sectionListRenderer"]["contents"];
        let podcasts = sections[4].clone();
        *sections = json!([podcasts]);

        assert_eq!(results(&reply, SearchKind::All), Some(Parsed::NoResults));
    }

    #[test]
    fn a_filtered_reply_has_no_type_word_before_a_playlist_author() {
        let hit_row = json!({
            "navigationEndpoint": { "browseEndpoint": {
                "browseId": "VLPLfakeList",
                "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": {
                    "pageType": "MUSIC_PAGE_TYPE_PLAYLIST" } } } },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    { "text": "Salt Mix" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    { "text": "A listener" }, { "text": " • " }, { "text": "12 songs" }] } } }
            ]
        });
        let reply = json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [
            { "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
                { "musicShelfRenderer": { "contents": [
                    { "musicResponsiveListItemRenderer": hit_row }
                ] } }
            ] } } } }
        ] } } });

        let Some(Parsed::Found(found)) = results(&reply, SearchKind::Playlists) else {
            panic!("expected results");
        };

        assert_eq!(found.playlists[0].by, ["A listener"]);
        assert_eq!(found.playlists[0].detail.as_deref(), Some("12 songs"));
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
        assert!(seen >= 7, "only {seen} fixtures");
    }
}
