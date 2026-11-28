//! Albums, artists and playlists in a search reply, and the top result card.

use serde_json::Value;
use uuid::Uuid;

use super::parse::{SEPARATOR, artwork, linked, runs, song, text, track};
use crate::catalog::{Hit, HitKind, QUICK_SONGS, Top, Track};

/// The video type of a song, as against a music video or an upload.
const SONG_TYPE: &str = "MUSIC_VIDEO_TYPE_ATV";

/// What a row leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Song,
    Hit(HitKind),
    /// A kind of result Lumen does not show, such as a podcast or a video.
    Ignored,
}

/// What a row is. `None` if it leads nowhere Lumen can tell. In the unfiltered
/// reply a music video is told from a song by its video type; a filtered reply
/// holds only what was asked for.
pub(super) fn classify(row: &Value, typed: bool) -> Option<Class> {
    if super::parse::video_of(row).is_some() {
        let video_type = row["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]
            ["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]
            ["watchEndpointMusicSupportedConfigs"]["watchEndpointMusicConfig"]["musicVideoType"]
            .as_str();
        let other = typed && video_type.is_some_and(|kind| kind != SONG_TYPE);
        return Some(if other { Class::Ignored } else { Class::Song });
    }
    let endpoint = &row["navigationEndpoint"]["browseEndpoint"];
    endpoint["browseId"].as_str()?;
    Some(page_kind(endpoint).map_or(Class::Ignored, Class::Hit))
}

/// The kind of page a browse endpoint opens. The page type says; the id's
/// prefix is the fallback.
fn page_kind(endpoint: &Value) -> Option<HitKind> {
    let page = endpoint["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]
        ["pageType"]
        .as_str();
    if let Some(page) = page {
        return match page {
            "MUSIC_PAGE_TYPE_ALBUM" => Some(HitKind::Album),
            "MUSIC_PAGE_TYPE_ARTIST" => Some(HitKind::Artist),
            "MUSIC_PAGE_TYPE_PLAYLIST" => Some(HitKind::Playlist),
            _ => None,
        };
    }
    let id = endpoint["browseId"].as_str()?;
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| id.starts_with(prefix));
    if starts(&["MPRE"]) {
        Some(HitKind::Album)
    } else if starts(&["MPLA", "UC"]) {
        Some(HitKind::Artist)
    } else if starts(&["VL", "VM", "RD"]) {
        Some(HitKind::Playlist)
    } else {
        None
    }
}

/// An album, artist or playlist row. `None` without a usable id or title.
pub(super) fn hit(row: &Value, kind: HitKind, typed: bool) -> Option<Hit> {
    let browse_id = row["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()?;
    let columns = row["flexColumns"].as_array()?;
    let title: String = runs(columns.first()?).filter_map(text).collect();
    let details: Vec<&Value> = columns
        .get(1)
        .map(|c| runs(c).collect())
        .unwrap_or_default();
    build(kind, browse_id, title, &details, typed, artwork(row))
}

/// A hit from a title and the runs of its second line.
fn build(
    kind: HitKind,
    browse_id: &str,
    title: String,
    details: &[&Value],
    typed: bool,
    artwork_url: Option<String>,
) -> Option<Hit> {
    let sane = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    if title.trim().is_empty() || browse_id.is_empty() || !browse_id.bytes().all(sane) {
        return None;
    }
    let parts: Vec<String> = details
        .split(|run| text(run).is_some_and(|t| t.trim() == SEPARATOR.to_string()))
        .map(|part| part.iter().filter_map(|&run| text(run)).collect::<String>())
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect();
    let (by, detail) = match kind {
        HitKind::Album => {
            // The first part is the kind of release: "Album", "Single" or "EP".
            let rest = parts.get(1..).unwrap_or_default();
            let mut by: Vec<String> = linked(details, "_ARTIST").map(str::to_owned).collect();
            if by.is_empty()
                && let Some(first) = rest.first().filter(|part| !is_year(part))
            {
                by.push(first.clone());
            }
            (by, rest.last().filter(|part| is_year(part)).cloned())
        }
        HitKind::Playlist => {
            let rest = parts.get(usize::from(typed)..).unwrap_or_default();
            match rest {
                [] => (Vec::new(), None),
                [only] => (Vec::new(), Some(only.clone())),
                [author, .., last] => (vec![author.clone()], Some(last.clone())),
            }
        }
        HitKind::Artist | HitKind::Song => {
            let detail = (parts.len() > 1).then(|| parts.last().cloned()).flatten();
            (Vec::new(), detail)
        }
    };
    Some(Hit {
        id: Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("youtube:browse:{browse_id}").as_bytes(),
        ),
        kind,
        title,
        by,
        detail,
        artwork_url,
    })
}

fn is_year(text: &str) -> bool {
    text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit())
}

/// The Top Result card: the best match and the songs that can be played from
/// it. `None` for a card Lumen cannot read, or for a best match of a kind it
/// does not show.
pub(super) fn top(card: &Value) -> Option<Top> {
    let title_run = &card["title"]["runs"][0];
    let title = title_run["text"].as_str()?.to_owned();
    let subtitle: Vec<&Value> = card["subtitle"]["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let artwork_url = artwork(card);
    let video = card["onTap"]["watchEndpoint"]["videoId"]
        .as_str()
        .or_else(|| title_run["navigationEndpoint"]["watchEndpoint"]["videoId"].as_str());

    let mut songs: Vec<Track> = Vec::new();
    let best = if let Some(video) = video {
        let (_, track) = track(video, title, &[subtitle], true, artwork_url)?;
        let best = Hit {
            id: track.id,
            kind: HitKind::Song,
            title: track.title.clone(),
            by: track.artists.clone(),
            detail: track.album.clone(),
            artwork_url: track.artwork_url.clone(),
        };
        songs.push(track);
        best
    } else {
        let endpoint = &title_run["navigationEndpoint"]["browseEndpoint"];
        let kind = page_kind(endpoint)?;
        build(
            kind,
            endpoint["browseId"].as_str()?,
            title,
            &subtitle,
            true,
            artwork_url,
        )?
    };

    let rows = card["contents"].as_array().into_iter().flatten();
    for item in rows {
        let row = &item["musicResponsiveListItemRenderer"];
        if songs.len() >= QUICK_SONGS || classify(row, true) != Some(Class::Song) {
            continue;
        }
        if let Some((_, track)) = song(row, true)
            && !songs.iter().any(|known| known.id == track.id)
        {
            songs.push(track);
        }
    }
    Some(Top { best, songs })
}
