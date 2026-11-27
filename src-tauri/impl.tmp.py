p="src/sources/youtube/search/parse.rs"
s=open(p,encoding="utf-8").read()
a=s.index("fn duration_ms(")
b=s.index("#[cfg(test)]")
code='''/// `None` for a reply with a layout Lumen does not know. Continuations are
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
    let details = runs(columns.get(1)?).collect::<Vec<_>>();
    let mut artists: Vec<String> = linked(&details, "_ARTIST").map(str::to_owned).collect();
    if artists.is_empty() {
        // Unlinked artists are everything before the first separator.
        let first: String = details
            .iter()
            .filter_map(|run| text(run))
            .collect::<String>()
            .split(SEPARATOR)
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !first.is_empty() && duration_ms(&first).is_none() {
            artists.push(first);
        }
    }
    let album = linked(&details, "_ALBUM").next().map(str::to_owned);
    let duration_ms = columns.iter().skip(1).rev().find_map(|column| {
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

'''
s=s[:a]+code+s[b:]
# drop the stub doc line
s=s.replace("/// `None` for a reply with a layout Lumen does not know.\n/// `None` for","/// `None` for")
s=s.replace('''use serde_json::Value;

use crate::catalog::Track;
''','''use std::collections::HashSet;

use serde_json::Value;
use uuid::Uuid;

use crate::catalog::{SourceKind, SourceRef, Track};

/// Stands between the artists, the album and the duration.
const SEPARATOR: char = '\u{2022}';
''')
s=s.replace('''    use super::*;
    use crate::catalog::{SourceKind, SourceRef};
''','''    use super::*;
''')
open(p,"w",encoding="utf-8",newline="\n").write(s)
