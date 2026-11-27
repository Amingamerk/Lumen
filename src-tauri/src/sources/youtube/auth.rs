//! The web client's request signature, as ytmusicapi computes it.

use sha1::{Digest, Sha1};

use super::cookies::Cookie;
use super::session::ORIGIN;

/// The cookies that can sign a request, in the order of preference.
const SIGNING_COOKIES: [&str; 2] = ["SAPISID", "__Secure-3PAPISID"];

/// The `Authorization` header for `cookies` at `now_s`, or `None` without a
/// cookie to sign with: `SAPISIDHASH <ts>_<sha1("<ts> <sapisid> <origin>")>`.
pub fn authorization(cookies: &[&Cookie], now_s: u64) -> Option<String> {
    let sapisid = SIGNING_COOKIES.iter().find_map(|name| {
        cookies
            .iter()
            .find(|cookie| cookie.name == *name && !cookie.value.is_empty())
    })?;
    let digest = Sha1::digest(format!("{now_s} {} {ORIGIN}", sapisid.value));
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("SAPISIDHASH {now_s}_{hex}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::youtube::session::fake::cookie;

    fn valued(name: &str, value: &str) -> Cookie {
        Cookie {
            value: value.to_owned(),
            ..cookie(name, ".youtube.com")
        }
    }

    // Digests computed independently: printf '%s' '<input>' | sha1sum, where the
    // input is "<ts> <sapisid> https://music.youtube.com".
    const FIRST: &str = "SAPISIDHASH 1700000000_fa90dc53295b9f3251e626355147af2d028548fd";
    const SHORT: &str = "SAPISIDHASH 1_2c59541a1cab73270e40fb052e27ec4a25b16212";

    #[test]
    fn signs_the_time_the_sapisid_and_the_origin() {
        for (value, now_s, expected) in [
            ("fakeSapisid/AbC123", 1_700_000_000, FIRST),
            ("x", 1, SHORT),
        ] {
            let sapisid = valued("SAPISID", value);

            assert_eq!(authorization(&[&sapisid], now_s).as_deref(), Some(expected));
        }
    }

    #[test]
    fn prefers_sapisid_and_falls_back_to_the_third_party_cookie() {
        let other = valued("__Secure-1PAPISID", "other");
        let third = valued("__Secure-3PAPISID", "x");
        let first = valued("SAPISID", "fakeSapisid/AbC123");

        assert_eq!(
            authorization(&[&third, &first], 1_700_000_000).as_deref(),
            Some(FIRST)
        );
        assert_eq!(authorization(&[&other, &third], 1).as_deref(), Some(SHORT));
    }

    #[test]
    fn signs_nothing_without_a_usable_cookie() {
        let empty = valued("SAPISID", "");
        let other = valued("SID", "sid");

        assert_eq!(authorization(&[&empty, &other], 1), None);
    }
}
