//! YouTube session cookies: parsing, filtering and rendering. Values are secrets: this module
//! never puts them in an error or `Debug` output.

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum CookieError {
    #[error("line {0} is not a Netscape cookie line (7 tab-separated fields)")]
    MalformedLine(usize),
    #[error("no youtube.com cookies found in the input")]
    NoYoutubeCookies,
    #[error("input is neither a Netscape cookies.txt nor a Cookie header")]
    UnknownFormat,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// A leading dot means the cookie also applies to subdomains.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds. `None` for a session cookie.
    pub expires: Option<u64>,
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("domain", &self.domain)
            .finish_non_exhaustive()
    }
}

/// Keeps only well-formed cookies for youtube.com and its subdomains.
pub fn youtube_only(cookies: Vec<Cookie>) -> Vec<Cookie> {
    cookies
        .into_iter()
        .filter(|c| is_youtube_domain(&c.domain) && is_well_formed(c))
        .collect()
}

fn is_youtube_domain(domain: &str) -> bool {
    let domain = domain
        .strip_prefix('.')
        .unwrap_or(domain)
        .to_ascii_lowercase();
    domain == "youtube.com" || domain.ends_with(".youtube.com")
}

/// Rejects anything that could break out of a header or a Netscape line.
fn is_well_formed(cookie: &Cookie) -> bool {
    let clean = |s: &str| !s.chars().any(char::is_control);
    is_valid_name(&cookie.name)
        && clean(&cookie.value)
        && !cookie.value.contains(';')
        && clean(&cookie.domain)
        && cookie.path.starts_with('/')
        && clean(&cookie.path)
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, ';' | '=' | ',' | '"'))
}

/// Accepts a Netscape `cookies.txt` or a `Cookie:` request header.
pub fn parse_import(input: &str) -> Result<Vec<Cookie>, CookieError> {
    let parsed = if looks_like_netscape(input) {
        parse_netscape(input)?
    } else {
        parse_header(input)?
    };
    let cookies = youtube_only(parsed);
    if cookies.is_empty() {
        return Err(CookieError::NoYoutubeCookies);
    }
    Ok(cookies)
}

const HTTP_ONLY_PREFIX: &str = "#HttpOnly_";

fn looks_like_netscape(input: &str) -> bool {
    let head = input.trim_start();
    head.starts_with("# Netscape")
        || head.starts_with("# HTTP Cookie File")
        || input
            .lines()
            .any(|line| line.starts_with(HTTP_ONLY_PREFIX) || line.split('\t').count() >= 6)
}

fn parse_netscape(input: &str) -> Result<Vec<Cookie>, CookieError> {
    let mut cookies = Vec::new();
    for (index, line) in input.trim_start_matches('\u{feff}').lines().enumerate() {
        let (line, http_only) = match line.strip_prefix(HTTP_ONLY_PREFIX) {
            Some(rest) => (rest, true),
            None if line.starts_with('#') || line.trim().is_empty() => continue,
            None => (line, false),
        };
        let cookie = netscape_line(line, http_only).ok_or(CookieError::MalformedLine(index + 1))?;
        cookies.push(cookie);
    }
    Ok(cookies)
}

fn netscape_line(line: &str, http_only: bool) -> Option<Cookie> {
    let fields: Vec<&str> = line.split('\t').collect();
    // Editors that trim trailing tabs drop the field of an empty value.
    let (domain, subdomains, path, secure, expires, name, value) = match fields[..] {
        [d, s, p, sec, e, n] => (d, s, p, sec, e, n, ""),
        [d, s, p, sec, e, n, v] => (d, s, p, sec, e, n, v),
        _ => return None,
    };
    let expires = match expires {
        "" => 0.0,
        text => text.parse::<f64>().ok()?,
    };
    let domain = if subdomains.eq_ignore_ascii_case("TRUE") && !domain.starts_with('.') {
        format!(".{domain}")
    } else {
        domain.to_owned()
    };
    Some(Cookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain,
        path: path.to_owned(),
        secure: secure.eq_ignore_ascii_case("TRUE"),
        http_only,
        expires: (expires > 0.0).then_some(expires as u64),
    })
}

/// A `Cookie` header carries no domain or flags, so assume the site-wide youtube.com cookie.
fn parse_header(input: &str) -> Result<Vec<Cookie>, CookieError> {
    let strip_name = |line: &str| {
        let (name, rest) = line.split_at_checked("cookie:".len())?;
        name.eq_ignore_ascii_case("cookie:")
            .then(|| rest.to_owned())
    };
    let lines: Vec<&str> = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let header = match lines.iter().find_map(|line| strip_name(line)) {
        Some(header) => header,
        None => match lines[..] {
            [only] => only.to_owned(),
            _ => return Err(CookieError::UnknownFormat),
        },
    };

    let mut cookies = Vec::new();
    for pair in header.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, value) = pair
            .split_once('=')
            .filter(|(name, _)| is_valid_name(name))
            .ok_or(CookieError::UnknownFormat)?;
        cookies.push(Cookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: ".youtube.com".to_owned(),
            path: "/".to_owned(),
            secure: true,
            http_only: false,
            expires: None,
        });
    }
    if cookies.is_empty() {
        return Err(CookieError::UnknownFormat);
    }
    Ok(cookies)
}

/// Netscape `cookies.txt` content, as yt-dlp reads it with `--cookies`.
pub fn netscape_file(cookies: &[Cookie]) -> String {
    let flag = |on: bool| if on { "TRUE" } else { "FALSE" };
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let prefix = if c.http_only { HTTP_ONLY_PREFIX } else { "" };
        out.push_str(&format!(
            "{prefix}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.domain,
            // Python's cookie jar rejects a file where this disagrees with the leading dot.
            flag(c.domain.starts_with('.')),
            c.path,
            flag(c.secure),
            c.expires.unwrap_or(0),
            c.name,
            c.value,
        ));
    }
    out
}

/// The cookies a browser would send with an HTTPS request to `host` + `path`.
pub fn matching<'a>(cookies: &'a [Cookie], host: &str, path: &str, now: u64) -> Vec<&'a Cookie> {
    cookies
        .iter()
        .filter(|c| {
            domain_matches(&c.domain, host)
                && path_matches(&c.path, path)
                && c.expires.is_none_or(|at| at > now)
        })
        .collect()
}

fn domain_matches(domain: &str, host: &str) -> bool {
    match domain.strip_prefix('.') {
        Some(suffix) => {
            host == suffix
                || host
                    .strip_suffix(suffix)
                    .is_some_and(|sub| sub.ends_with('.'))
        }
        None => host == domain,
    }
}

fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    request_path
        .strip_prefix(cookie_path)
        .is_some_and(|rest| rest.is_empty() || cookie_path.ends_with('/') || rest.starts_with('/'))
}

/// Value of a `Cookie` request header.
pub fn header_value(cookies: &[&Cookie]) -> String {
    cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            secure: true,
            http_only: false,
            expires: None,
        }
    }

    const NETSCAPE: &str = "# Netscape HTTP Cookie File\n\
        # a comment\n\
        \n\
        .youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tfake-sapisid\n\
        #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tfake-login\r\n\
        .google.com\tTRUE\t/\tTRUE\t1900000000\tSID\tfake-google\n\
        .example.org\tTRUE\t/\tFALSE\t0\tother\tfake-other\n";

    #[test]
    fn parses_a_netscape_file_and_keeps_only_youtube_cookies() {
        let cookies = parse_import(NETSCAPE).unwrap();

        assert_eq!(
            cookies,
            vec![
                Cookie {
                    expires: Some(1_900_000_000),
                    ..cookie("SAPISID", "fake-sapisid", ".youtube.com")
                },
                Cookie {
                    http_only: true,
                    ..cookie("LOGIN_INFO", "fake-login", ".youtube.com")
                },
            ]
        );
    }

    #[test]
    fn parses_a_netscape_line_without_the_file_header() {
        let line = ".youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc";

        assert_eq!(
            parse_import(line).unwrap(),
            vec![cookie("SAPISID", "abc", ".youtube.com")]
        );
    }

    #[test]
    fn reports_a_malformed_netscape_line_by_number_only() {
        let input =
            "# Netscape HTTP Cookie File\n.youtube.com TRUE / TRUE 0 SAPISID secret-value\n";

        let error = parse_import(input).unwrap_err();

        assert_eq!(error, CookieError::MalformedLine(2));
        assert!(!error.to_string().contains("secret-value"));
    }

    #[test]
    fn rejects_a_netscape_file_without_youtube_cookies() {
        let input = ".google.com\tTRUE\t/\tTRUE\t0\tSID\tfake";

        assert_eq!(parse_import(input), Err(CookieError::NoYoutubeCookies));
    }

    #[test]
    fn parses_a_cookie_header_with_or_without_the_header_name() {
        let expected = vec![
            cookie("SAPISID", "abc/def", ".youtube.com"),
            cookie("PREF", "f4=4000000&tz=UTC", ".youtube.com"),
        ];

        for input in [
            "Cookie: SAPISID=abc/def; PREF=f4=4000000&tz=UTC",
            "cookie:SAPISID=abc/def;PREF=f4=4000000&tz=UTC;",
            "  SAPISID=abc/def; PREF=f4=4000000&tz=UTC\n",
        ] {
            assert_eq!(parse_import(input).unwrap(), expected, "input: {input}");
        }
    }

    #[test]
    fn finds_the_cookie_line_in_a_pasted_block_of_request_headers() {
        let input = "accept: */*\r\ncookie: SAPISID=abc\r\nuser-agent: UA/1.0\r\n";

        assert_eq!(
            parse_import(input).unwrap(),
            vec![cookie("SAPISID", "abc", ".youtube.com")]
        );
    }

    #[test]
    fn rejects_input_that_is_neither_format() {
        for input in ["", "   \n ", "hello world", "accept: */*\nhost: x"] {
            assert_eq!(
                parse_import(input),
                Err(CookieError::UnknownFormat),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn youtube_only_rejects_lookalike_domains_and_unsafe_values() {
        let cookies = vec![
            cookie("a", "1", ".youtube.com"),
            cookie("b", "2", "music.youtube.com"),
            cookie("c", "3", "youtube.com"),
            cookie("d", "4", "notyoutube.com"),
            cookie("e", "5", ".youtube.com.example.org"),
            cookie("f", "line\nbreak", ".youtube.com"),
            cookie("g", "tab\there", ".youtube.com"),
            cookie("h", "semi;colon", ".youtube.com"),
            cookie("bad name", "6", ".youtube.com"),
            cookie("", "7", ".youtube.com"),
            Cookie {
                path: "/\n.example.org".into(),
                ..cookie("i", "8", ".youtube.com")
            },
        ];

        let names: Vec<_> = youtube_only(cookies).into_iter().map(|c| c.name).collect();

        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn renders_netscape_content_that_parses_back_unchanged() {
        let cookies = vec![
            Cookie {
                expires: Some(1_900_000_000),
                ..cookie("SAPISID", "abc", ".youtube.com")
            },
            Cookie {
                http_only: true,
                secure: false,
                ..cookie("LOGIN_INFO", "x:y", "music.youtube.com")
            },
        ];

        let content = netscape_file(&cookies);

        assert_eq!(
            content,
            "# Netscape HTTP Cookie File\n\
             .youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc\n\
             #HttpOnly_music.youtube.com\tFALSE\t/\tFALSE\t0\tLOGIN_INFO\tx:y\n"
        );
        assert_eq!(parse_import(&content).unwrap(), cookies);
    }

    #[test]
    fn selects_the_cookies_a_browser_would_send() {
        let cookies = vec![
            cookie("domain_wide", "1", ".youtube.com"),
            cookie("music_host", "2", "music.youtube.com"),
            cookie("apex_host_only", "3", "youtube.com"),
            cookie("www_host", "4", "www.youtube.com"),
            Cookie {
                expires: Some(999),
                ..cookie("expired", "5", ".youtube.com")
            },
            Cookie {
                path: "/youtubei".into(),
                ..cookie("path_prefix", "6", ".youtube.com")
            },
            Cookie {
                path: "/you".into(),
                ..cookie("other_path", "7", ".youtube.com")
            },
        ];

        let sent = matching(&cookies, "music.youtube.com", "/youtubei/v1/browse", 1_000);

        assert_eq!(
            sent.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["domain_wide", "music_host", "path_prefix"]
        );
        assert_eq!(
            header_value(&sent),
            "domain_wide=1; music_host=2; path_prefix=6"
        );
    }

    #[test]
    fn debug_output_never_shows_the_value() {
        let printed = format!("{:?}", cookie("SAPISID", "secret-value", ".youtube.com"));

        assert!(printed.contains("SAPISID"), "{printed}");
        assert!(!printed.contains("secret-value"), "{printed}");
    }
}
