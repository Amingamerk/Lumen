//! ytmusicapi-style browser authentication: the `SAPISIDHASH` Authorization header.

use sha1::{Digest, Sha1};

use crate::cookies::Cookie;

pub const ORIGIN: &str = "https://music.youtube.com";

/// The cookie value the hash is built from: `SAPISID`, else `__Secure-3PAPISID`.
pub fn sapisid<'a>(cookies: &[&'a Cookie]) -> Option<&'a str> {
    ["SAPISID", "__Secure-3PAPISID"].iter().find_map(|name| {
        cookies
            .iter()
            .find(|c| c.name == *name && !c.value.is_empty())
            .map(|c| c.value.as_str())
    })
}

/// `SAPISIDHASH <ts>_<sha1("<ts> <sapisid> <origin>")>`
pub fn sapisidhash(sapisid: &str, unix_ts: u64, origin: &str) -> String {
    let digest = Sha1::digest(format!("{unix_ts} {sapisid} {origin}"));
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("SAPISIDHASH {unix_ts}_{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, value: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: ".youtube.com".into(),
            path: "/".into(),
            secure: true,
            http_only: false,
            expires: None,
        }
    }

    // Digests computed independently: printf '%s' '<input>' | sha1sum
    #[test]
    fn hashes_timestamp_sapisid_and_origin() {
        assert_eq!(
            sapisidhash("fakeSapisid/AbC123", 1_700_000_000, ORIGIN),
            "SAPISIDHASH 1700000000_fa90dc53295b9f3251e626355147af2d028548fd"
        );
        assert_eq!(
            sapisidhash("x", 1, ORIGIN),
            "SAPISIDHASH 1_2c59541a1cab73270e40fb052e27ec4a25b16212"
        );
    }

    #[test]
    fn prefers_sapisid_over_the_third_party_cookie() {
        let third = cookie("__Secure-3PAPISID", "third");
        let first = cookie("SAPISID", "first");

        assert_eq!(sapisid(&[&third, &first]), Some("first"));
    }

    #[test]
    fn falls_back_to_the_third_party_cookie() {
        let other = cookie("__Secure-1PAPISID", "one");
        let third = cookie("__Secure-3PAPISID", "third");

        assert_eq!(sapisid(&[&other, &third]), Some("third"));
    }

    #[test]
    fn finds_nothing_without_either_cookie_or_with_an_empty_one() {
        let empty = cookie("SAPISID", "");
        let other = cookie("SID", "sid");

        assert_eq!(sapisid(&[&empty, &other]), None);
    }
}
