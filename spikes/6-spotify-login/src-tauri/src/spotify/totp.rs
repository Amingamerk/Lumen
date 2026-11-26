//! TOTP (RFC 6238, HMAC-SHA1) and the web player's secret derivation.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

/// `totpVer` of [`CIPHER`]. Spotify rotates both together.
pub const VERSION: u32 = 61;

/// Public constant from the web player bundle. It is the same for every visitor.
const CIPHER: &str = ",7/*F(\"rLJ2oxaKL^f+E1xvP@N";

const STEP_SECS: u64 = 30;

/// RFC 6238 code for `unix_secs`, zero-padded to `digits` (at most 9).
pub fn totp(key: &[u8], unix_secs: u64, digits: u32) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&(unix_secs / STEP_SECS).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    // Dynamic truncation, RFC 4226 section 5.3.
    let offset = usize::from(hash[hash.len() - 1] & 0x0f);
    let word = u32::from_be_bytes(std::array::from_fn(|i| hash[offset + i])) & 0x7fff_ffff;
    let code = word % 10u32.pow(digits);
    format!("{code:0width$}", width = digits as usize)
}

/// HMAC key the web player builds from its obfuscated secret.
pub fn spotify_key(cipher: &str) -> Vec<u8> {
    cipher
        .chars()
        .enumerate()
        .map(|(i, c)| (u32::from(c) ^ (i as u32 % 33 + 9)).to_string())
        .collect::<String>()
        .into_bytes()
}

/// The six-digit code sent as `totp` and `totpServer`.
pub fn spotify_code(unix_secs: u64) -> String {
    totp(&spotify_key(CIPHER), unix_secs, 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1 rows.
    #[test]
    fn matches_rfc_6238_sha1_vectors() {
        let key = b"12345678901234567890";
        for (time, code) in [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ] {
            assert_eq!(totp(key, time, 8), code, "T={time}");
        }
    }

    #[test]
    fn six_digit_codes_keep_leading_zeros() {
        assert_eq!(totp(b"12345678901234567890", 1_234_567_890, 6), "005924");
    }

    #[test]
    fn derives_key_as_decimal_text_of_xored_chars() {
        // Chars "AB": 65 ^ 9 = 72, 66 ^ 10 = 72.
        assert_eq!(spotify_key("AB"), b"7272");
    }

    #[test]
    fn xor_mask_wraps_after_33_chars() {
        let key = spotify_key(&"A".repeat(34));
        // Index 33 uses the same mask as index 0.
        assert!(key.starts_with(b"72"));
        assert!(key.ends_with(b"72"));
    }

    /// Vectors computed with Node's HMAC over the transform copied from the web
    /// player bundle (web-player.4c608093.js, 2026-11-26).
    #[test]
    fn matches_web_player_derivation_for_current_version() {
        assert_eq!(VERSION, 61);
        assert_eq!(
            spotify_key(CIPHER),
            b"376136387538459893883312310911992847112448894410210511297108"
        );
        for (time, code) in [
            (0, "204513"),
            (59, "332823"),
            (1_111_111_109, "611107"),
            (1_795_715_854, "990380"),
            (1_795_715_880, "372779"),
        ] {
            assert_eq!(spotify_code(time), code, "T={time}");
        }
    }
}
