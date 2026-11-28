//! The addresses the webview asks for: `lumen://art/<size>/<track id>`. The
//! webview never learns where a picture really lives.

use uuid::Uuid;

use super::Size;

/// What a protocol path names, or `None` if it names anything else. Nothing of
/// the path reaches the file system: it only picks a size and a track.
pub fn parse(path: &str) -> Option<(Size, Uuid)> {
    let mut parts = path.strip_prefix('/')?.split('/');
    let (Some("art"), Some(size), Some(id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let size = Size::from_name(size)?;
    // Only the plain hyphenated form: no braces, no `urn:uuid:`, no padding.
    let canonical = id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
    let id = Uuid::parse_str(id).ok().filter(|_| canonical)?;
    Some((size, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b9d3f0e-5c5e-5a53-8f6a-3c1f6a7d2b11";

    #[test]
    fn a_size_and_a_track_id_are_read() {
        let id = Uuid::parse_str(ID).unwrap();
        assert_eq!(parse(&format!("/art/64/{ID}")), Some((Size::Small, id)));
        assert_eq!(parse(&format!("/art/300/{ID}")), Some((Size::Large, id)));
    }

    #[test]
    fn other_paths_are_refused() {
        for path in [
            "",
            "/",
            "/art",
            "/art/64",
            &format!("/art/64/{ID}/"),
            &format!("/art/64/{ID}/extra"),
            &format!("/art/65/{ID}"),
            &format!("/art/0064/{ID}"),
            &format!("/art/-64/{ID}"),
            &format!("/other/64/{ID}"),
            &format!("art/64/{ID}"),
            &format!("//art/64/{ID}"),
            "/art/64/..",
            "/art/64/../../Windows/win.ini",
            "/art/64/..%2F..%2Fsecret",
            "/art/64/%2e%2e",
            "/art/64/C:/Windows",
            r"/art/64/..\..\secret",
            &format!("/art/64/{{{ID}}}"),
            &format!("/art/64/urn:uuid:{ID}"),
            &format!("/art/64/{}", ID.replace('-', "")),
        ] {
            assert_eq!(parse(path), None, "{path}");
        }
    }
}
