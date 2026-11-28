//! Invented music for the static catalog. No real artists, no remote artwork.

use uuid::Uuid;

use super::{Album, Catalog, Playlist, SourceKind, SourceRef, Track};

use SourceKind::{SoundCloud, Spotify, YouTube};

/// The generated playlist that proves long lists are windowed.
pub const LONG_PLAYLIST_ID: &str = "long-haul";

const ON_YOUTUBE: &[SourceKind] = &[YouTube];
const ON_SOUNDCLOUD: &[SourceKind] = &[SoundCloud];
const ON_YOUTUBE_AND_SOUNDCLOUD: &[SourceKind] = &[YouTube, SoundCloud];
/// A Spotify Track with a Match on YouTube Music.
const MATCHED: &[SourceKind] = &[Spotify, YouTube];
/// A Spotify Track without a Match yet.
const UNMATCHED: &[SourceKind] = &[Spotify];

/// Title, artists, album, length in seconds and sources. A Track's id is its
/// position in this list, starting at 1.
type TrackSeed = (
    &'static str,
    &'static [&'static str],
    Option<&'static str>,
    u32,
    &'static [SourceKind],
);

#[rustfmt::skip]
const TRACKS: &[TrackSeed] = &[
    // 1-4
    ("Harbor Lights", &["Mira Sol"], Some("Low Tide"), 201, ON_YOUTUBE),
    ("Salt on the Window", &["Mira Sol"], Some("Low Tide"), 242, ON_YOUTUBE),
    ("Slow Return", &["Mira Sol"], Some("Low Tide"), 178, ON_YOUTUBE),
    ("Breakwater", &["Mira Sol", "Juno Park"], Some("Low Tide"), 224, ON_YOUTUBE),
    // 5-7
    ("Night Ferry", &["The Quiet Pier"], Some("Crossings"), 312, ON_SOUNDCLOUD),
    ("Far Bank", &["The Quiet Pier"], Some("Crossings"), 271, ON_SOUNDCLOUD),
    ("Signal Fire", &["The Quiet Pier"], Some("Crossings"), 363, ON_SOUNDCLOUD),
    // 8-11
    ("Lamplighter", &["Juno Park"], Some("Greenhouse Hours"), 189, MATCHED),
    ("Condensation", &["Juno Park"], Some("Greenhouse Hours"), 167, MATCHED),
    ("Seedlings", &["Juno Park"], Some("Greenhouse Hours"), 213, UNMATCHED),
    ("Night Watering", &["Juno Park"], Some("Greenhouse Hours"), 251, MATCHED),
    // 12-14
    ("Paper Weather", &["Paper Lanterns"], Some("Paper Weather"), 185, ON_YOUTUBE),
    ("Umbrella Stand", &["Paper Lanterns"], Some("Paper Weather"), 159, ON_YOUTUBE),
    ("Forecast", &["Paper Lanterns"], Some("Paper Weather"), 232, ON_YOUTUBE),
    // 15-17
    ("North Platform", &["Søren Vale"], Some("North Platform"), 260, MATCHED),
    ("Last Train Home", &["Søren Vale"], Some("North Platform"), 238, MATCHED),
    ("Timetable", &["Søren Vale"], Some("North Platform"), 171, UNMATCHED),
    // 18-20
    ("Static Bloom", &["Kestrel Row"], Some("Static Bloom"), 284, ON_SOUNDCLOUD),
    ("Tape Hiss Garden", &["Kestrel Row"], Some("Static Bloom"), 196, ON_SOUNDCLOUD),
    ("Antenna", &["Kestrel Row"], Some("Static Bloom"), 329, ON_SOUNDCLOUD),
    // 21-23
    ("Simmer", &["Ines Marlow", "Tomo Hadi"], Some("Kitchen Radio"), 175, ON_YOUTUBE),
    ("Sunday Stock", &["Ines Marlow"], Some("Kitchen Radio"), 221, ON_YOUTUBE),
    ("Open Window", &["Tomo Hadi"], Some("Kitchen Radio"), 182, ON_YOUTUBE),
    // 24-26
    ("Small Hours", &["Low Orbit Choir"], Some("Small Hours"), 375, ON_YOUTUBE_AND_SOUNDCLOUD),
    ("Streetlamp Hymn", &["Low Orbit Choir"], Some("Small Hours"), 248, ON_YOUTUBE),
    ("Quarter to Four", &["Low Orbit Choir"], Some("Small Hours"), 207, ON_YOUTUBE),
    // 27-29
    ("Rain on Corrugated Iron", &["Cold Harbour Radio"], None, 422, ON_SOUNDCLOUD),
    ("Tram Depot, Early Shift", &["Cold Harbour Radio"], None, 258, ON_SOUNDCLOUD),
    ("Pier at Slack Water", &["Cold Harbour Radio"], None, 350, ON_SOUNDCLOUD),
];

/// Id, title, artists and source. An album holds the Tracks that name it.
type AlbumSeed = (
    &'static str,
    &'static str,
    &'static [&'static str],
    SourceKind,
);

#[rustfmt::skip]
const ALBUMS: &[AlbumSeed] = &[
    ("low-tide", "Low Tide", &["Mira Sol"], YouTube),
    ("crossings", "Crossings", &["The Quiet Pier"], SoundCloud),
    ("greenhouse-hours", "Greenhouse Hours", &["Juno Park"], Spotify),
    ("paper-weather", "Paper Weather", &["Paper Lanterns"], YouTube),
    ("north-platform", "North Platform", &["Søren Vale"], Spotify),
    ("static-bloom", "Static Bloom", &["Kestrel Row"], SoundCloud),
    ("kitchen-radio", "Kitchen Radio", &["Ines Marlow", "Tomo Hadi"], YouTube),
    ("small-hours", "Small Hours", &["Low Orbit Choir"], YouTube),
];

/// Id, name, description, source and Track numbers.
type PlaylistSeed = (
    &'static str,
    &'static str,
    &'static str,
    SourceKind,
    &'static [u128],
);

#[rustfmt::skip]
const PLAYLISTS: &[PlaylistSeed] = &[
    ("night-shift", "Night shift", "After midnight", YouTube, &[24, 25, 26, 1, 3]),
    ("tunnel-mixes", "Tunnel mixes", "Low and echoing", SoundCloud, &[5, 7, 20, 18]),
    ("liked-on-spotify", "Liked on Spotify", "Your liked songs", Spotify, &[8, 9, 15, 16, 17, 10]),
    ("sunday-cooking", "Sunday cooking", "Pot on low", YouTube, &[21, 22, 23, 13]),
    ("field-recordings", "Field recordings", "Places, as heard", SoundCloud, &[27, 28, 29]),
    ("long-drive-home", "Songs for the long drive home after a late shift", "Windows down", Spotify, &[16, 11, 9, 15]),
];

pub(super) fn catalog() -> Catalog {
    let tracks: Vec<Track> = (1..).zip(TRACKS).map(track).collect();
    let albums = ALBUMS
        .iter()
        .map(|&(id, title, artists, source)| Album {
            id: id.to_owned(),
            title: title.to_owned(),
            artists: owned(artists),
            source,
            tracks: tracks
                .iter()
                .filter(|t| t.album.as_deref() == Some(title))
                .map(|t| t.id)
                .collect(),
        })
        .collect();
    let playlists = PLAYLISTS
        .iter()
        .map(|&(id, name, description, source, numbers)| Playlist {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            source,
            tracks: numbers.iter().copied().map(Uuid::from_u128).collect(),
        })
        .collect();

    let (long_playlist, long_tracks) = long_playlist();
    Catalog::new(tracks, playlists, albums).with_unsearched(long_playlist, long_tracks)
}

const LONG_PLAYLIST_LENGTH: u128 = 5_000;
/// Generated Track ids start here, clear of the hand-written ones.
const FIRST_LONG_ID: u128 = 10_000;

const FIRST_WORDS: &[&str] = &[
    "Amber", "Quiet", "Copper", "Northern", "Paper", "Salt", "Evening", "Hollow", "Glass",
    "Winter", "Borrowed", "Slow", "Distant", "Velvet", "Open", "Last", "Silver", "Second",
    "Harbor",
];
const SECOND_WORDS: &[&str] = &[
    "Lantern", "Tide", "Window", "Signal", "Orchard", "Platform", "Weather", "Static", "Ferry",
    "Garden", "Current", "Rooftop", "Meadow", "Station", "Ember", "Compass", "Letters",
];
const LONG_ARTISTS: &[&str] = &[
    "Mira Sol",
    "The Quiet Pier",
    "Juno Park",
    "Paper Lanterns",
    "Søren Vale",
    "Kestrel Row",
    "Ines Marlow",
    "Tomo Hadi",
    "Low Orbit Choir",
    "Cold Harbour Radio",
    "Ada Fenwick",
];

/// 5,000 Spotify Tracks built from the word lists, the same ones every time.
/// Generating them keeps the binary and start-up small.
fn long_playlist() -> (Playlist, Vec<Track>) {
    let tracks: Vec<Track> = (0..LONG_PLAYLIST_LENGTH).map(long_track).collect();
    let playlist = Playlist {
        id: LONG_PLAYLIST_ID.to_owned(),
        name: "The long haul".to_owned(),
        description: "Five thousand tracks, end to end".to_owned(),
        source: Spotify,
        tracks: tracks.iter().map(|t| t.id).collect(),
    };
    (playlist, tracks)
}

fn long_track(n: u128) -> Track {
    // The remainder is below the list's length, so both casts are exact.
    let pick = |words: &[&'static str], at: u128| words[(at % words.len() as u128) as usize];

    let mut title = format!("{} {}", pick(FIRST_WORDS, n), pick(SECOND_WORDS, n));
    // Now and then a title too long for its column.
    if n.is_multiple_of(97) {
        title.push_str(", as Heard From the Far End of the Platform After the Last Train");
    }
    // Twelve Tracks to an album, all by the same artist.
    let disc = n / 12;
    // A Spotify Track mostly has a Match on YouTube Music, sometimes only on
    // SoundCloud, and sometimes none.
    let kinds: &[SourceKind] = match n % 10 {
        7 => &[Spotify, SoundCloud],
        9 => UNMATCHED,
        _ => MATCHED,
    };

    Track {
        id: Uuid::from_u128(FIRST_LONG_ID + n),
        title,
        artists: vec![pick(LONG_ARTISTS, disc).to_owned()],
        album: Some(format!(
            "{} {}",
            pick(SECOND_WORDS, disc / 7),
            pick(FIRST_WORDS, disc)
        )),
        // From 2:00 to 5:59, spread out.
        duration_ms: (120 + (n * 37 % 240) as u32) * 1000,
        artwork_url: None,
        isrc: None,
        sources: source_refs(FIRST_LONG_ID + n, kinds),
    }
}

fn track((number, &(title, artists, album, seconds, kinds)): (u128, &TrackSeed)) -> Track {
    Track {
        id: Uuid::from_u128(number),
        title: title.to_owned(),
        artists: owned(artists),
        album: album.map(str::to_owned),
        duration_ms: seconds * 1000,
        artwork_url: None,
        isrc: None,
        sources: source_refs(number, kinds),
    }
}

fn source_refs(number: u128, kinds: &[SourceKind]) -> Vec<SourceRef> {
    kinds
        .iter()
        .map(|&kind| SourceRef {
            kind,
            id: format!("fixture-{number}"),
        })
        .collect()
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}
