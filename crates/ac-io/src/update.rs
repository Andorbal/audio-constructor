//! Whether a newer release is out, for the editor's update check and
//! `audioc version --check`.
//!
//! The feed is the repository's "latest release" document, the same one
//! the releases page shows: its tag names the version and its page is where
//! the downloads are. Asking for it sends nothing but the request (see
//! [`crate::net`]); nothing is downloaded or installed. `AC_UPDATE_FEED`
//! points the check somewhere else (a mirror, or a test server).

use std::cmp::Ordering;
use std::fmt;

use crate::net::{self, Limits};

/// A release version, `major.minor.patch`. Pre-release and build suffixes
/// are ignored: the feed only names finished releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `0.14.0`, `v0.14.0`, or `v1.2.3-rc.1`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let core = text.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let mut next = || parts.next()?.parse::<u64>().ok();
        let v = Version(next()?, next()?, next()?);
        parts.next().is_none().then_some(v)
    }

    /// This build's version.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The newest release, as the feed describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// The release's page, where its downloads are (always `https`).
    pub page: String,
    /// Its notes: the changelog section for that version.
    pub notes: String,
}

/// Where the feed is: `AC_UPDATE_FEED`, or the latest-release document of
/// the repository this was built from.
pub fn feed_url() -> String {
    if let Some(url) = std::env::var("AC_UPDATE_FEED")
        .ok()
        .filter(|u| !u.is_empty())
    {
        return url;
    }
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let path = repo
        .strip_prefix("https://github.com/")
        .unwrap_or("Andorbal/audio-constructor");
    format!("https://api.github.com/repos/{path}/releases/latest")
}

/// The release a feed document describes.
pub fn parse_feed(json: &[u8]) -> Result<Release, String> {
    let doc: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("the release feed isn't JSON: {e}"))?;
    let field = |name: &str| doc.get(name).and_then(serde_json::Value::as_str);
    let tag = field("tag_name").ok_or("the release feed names no version")?;
    let version = Version::parse(tag)
        .ok_or_else(|| format!("the release feed's version {tag:?} isn't X.Y.Z"))?;
    // Only a secure page is offered to open; a feed that points elsewhere
    // gets the repository's releases page instead.
    let page = field("html_url")
        .filter(|u| u.starts_with("https://"))
        .map_or_else(releases_page, str::to_string);
    let notes = field("body").unwrap_or("").replace("\r\n", "\n");
    Ok(Release {
        version,
        page,
        notes,
    })
}

/// The repository's releases page.
pub fn releases_page() -> String {
    format!(
        "{}/releases",
        env!("CARGO_PKG_REPOSITORY").trim_end_matches('/')
    )
}

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// `release` is newer than this build.
    Newer(Release),
    /// This build is the newest (or newer: a development build).
    UpToDate(Version),
}

/// Asks the feed at `url` whether a release newer than `current` is out.
pub fn check(url: &str, current: Version) -> Result<Check, String> {
    let body = net::fetch(url, Limits::DOCUMENT).map_err(|e| e.to_string())?;
    let release = parse_feed(&body)?;
    Ok(match release.version.cmp(&current) {
        Ordering::Greater => Check::Newer(release),
        _ => Check::UpToDate(release.version),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn versions_read_with_or_without_a_v_and_compare_numerically() {
        assert_eq!(Version::parse("0.14.0"), Some(Version(0, 14, 0)));
        assert_eq!(Version::parse("v1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse(" v1.2.3-rc.1 "), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("1.2.3+build.5"), Some(Version(1, 2, 3)));
        for bad in ["", "1.2", "1.2.3.4", "one.two.three", "v", "1..3"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(0, 14, 0).to_string(), "0.14.0");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_feed_gives_the_version_page_and_notes() {
        let r = parse_feed(
            br#"{"tag_name":"v0.15.0","html_url":"https://example.org/r/v0.15.0","body":"Added:\r\n- Things","draft":false}"#,
        )
        .unwrap();
        assert_eq!(r.version, Version(0, 15, 0));
        assert_eq!(r.page, "https://example.org/r/v0.15.0");
        assert_eq!(r.notes, "Added:\n- Things");
        // An insecure page falls back to the releases page.
        let r = parse_feed(br#"{"tag_name":"0.15.0","html_url":"http://x/y"}"#).unwrap();
        assert_eq!(r.page, releases_page());
        assert!(r.notes.is_empty());
        assert!(parse_feed(b"<html>").unwrap_err().contains("isn't JSON"));
        assert!(
            parse_feed(br#"{"name":"x"}"#)
                .unwrap_err()
                .contains("no version")
        );
        assert!(
            parse_feed(br#"{"tag_name":"latest"}"#)
                .unwrap_err()
                .contains("X.Y.Z")
        );
    }

    #[test]
    fn the_default_feed_is_this_repositorys() {
        // Unless AC_UPDATE_FEED is set for the whole test run.
        if std::env::var_os("AC_UPDATE_FEED").is_none() {
            assert_eq!(
                feed_url(),
                "https://api.github.com/repos/Andorbal/audio-constructor/releases/latest"
            );
        }
        assert_eq!(
            releases_page(),
            "https://github.com/Andorbal/audio-constructor/releases"
        );
    }

    fn serve_once(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                line.clear();
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        format!("http://127.0.0.1:{port}/releases/latest")
    }

    #[test]
    fn a_check_says_whether_the_feed_is_newer() {
        let url = serve_once(r#"{"tag_name":"v0.15.0","html_url":"https://example.org/x"}"#);
        match check(&url, Version(0, 14, 0)).unwrap() {
            Check::Newer(r) => assert_eq!(r.version, Version(0, 15, 0)),
            other => panic!("{other:?}"),
        }
        let url = serve_once(r#"{"tag_name":"v0.14.0"}"#);
        assert_eq!(
            check(&url, Version(0, 14, 0)).unwrap(),
            Check::UpToDate(Version(0, 14, 0))
        );
        // A development build newer than the last release is up to date.
        let url = serve_once(r#"{"tag_name":"v0.13.2"}"#);
        assert_eq!(
            check(&url, Version(0, 14, 0)).unwrap(),
            Check::UpToDate(Version(0, 13, 2))
        );
        // Nobody listening: an error worded for people, not a panic.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = check(&format!("http://127.0.0.1:{port}/"), Version(0, 14, 0)).unwrap_err();
        assert!(err.contains("couldn't download"), "{err}");
    }
}
