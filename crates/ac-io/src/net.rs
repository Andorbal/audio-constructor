//! Fetching small files over HTTPS: block files and packs imported from a
//! link, library indexes, and the release feed the update check reads.
//!
//! Every fetch is bounded: by size (the caller says how much it expects,
//! and a longer body is an error, not a truncation) and by time. Plain
//! `http` is refused except to this machine (`localhost`, `127.0.0.1`,
//! `[::1]`), where tests and local servers live; a redirect may not leave
//! `https` either. Certificates are checked against the system's trust
//! store, and the usual proxy variables (`HTTPS_PROXY`, `ALL_PROXY`,
//! `NO_PROXY`) are honoured. Nothing is sent but the request itself and a
//! `User-Agent` naming the program and its version.
//!
//! Not built for the browser, whose page fetches for itself.

use std::fmt;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig};

/// How much a fetch may take, in bytes and in time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub max_bytes: u64,
    pub timeout: Duration,
}

impl Limits {
    /// A JSON document: an index, a block, the release feed.
    pub const DOCUMENT: Limits = Limits {
        max_bytes: 4 << 20,
        timeout: Duration::from_secs(20),
    };
    /// A pack or a preview: audio inside.
    pub const PACK: Limits = Limits {
        max_bytes: 256 << 20,
        timeout: Duration::from_secs(300),
    };
}

/// Why a fetch failed, worded for the person who asked for it.
#[derive(Clone, Debug, PartialEq)]
pub enum FetchError {
    /// Not a link this can follow.
    BadUrl(String),
    /// `http` to somewhere other than this machine.
    Insecure(String),
    /// The server answered with an error status.
    Status(u16),
    /// The body is longer than the limit.
    TooLarge { limit: u64 },
    /// No answer in time.
    TimedOut,
    /// Anything else on the way: no connection, a bad certificate, a broken
    /// response.
    Network(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::BadUrl(why) => write!(f, "not a link that can be opened: {why}"),
            FetchError::Insecure(url) => write!(
                f,
                "{url} isn't secure; use an https:// link (http:// works only on this computer)"
            ),
            FetchError::Status(404) => f.write_str("the server has nothing at that address (404)"),
            FetchError::Status(code) => write!(f, "the server answered with an error ({code})"),
            FetchError::TooLarge { limit } => write!(
                f,
                "the file is larger than the {} MB this accepts",
                limit.div_ceil(1 << 20)
            ),
            FetchError::TimedOut => f.write_str("the server didn't answer in time"),
            FetchError::Network(why) => write!(f, "couldn't download it: {why}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Whether `url` may be fetched: `https`, or `http` to this machine. Returns
/// whether it's local (and so may stay on `http`).
pub fn check_url(url: &str) -> Result<bool, FetchError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| FetchError::BadUrl(format!("{url} has no https:// at the start")))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_of(authority);
    if host.is_empty() {
        return Err(FetchError::BadUrl(format!("{url} names no server")));
    }
    let local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    match scheme.to_ascii_lowercase().as_str() {
        "https" => Ok(local),
        "http" if local => Ok(true),
        "http" => Err(FetchError::Insecure(url.to_string())),
        other => Err(FetchError::BadUrl(format!(
            "{other}:// links aren't supported, only https://"
        ))),
    }
}

/// The host part of `user@host:port`, keeping an IPv6 literal's brackets.
fn host_of(authority: &str) -> &str {
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if hostport.starts_with('[') {
        return hostport.find(']').map_or(hostport, |end| &hostport[..=end]);
    }
    hostport.split(':').next().unwrap_or("")
}

/// Downloads `url` within `limits`.
pub fn fetch(url: &str, limits: Limits) -> Result<Vec<u8>, FetchError> {
    let local = check_url(url)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(limits.timeout))
        // A redirect must stay on https too (except on this machine).
        .https_only(!local)
        .user_agent(concat!("audio-constructor/", env!("CARGO_PKG_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent();
    let mut response = agent.get(url).call().map_err(error)?;
    // A declared length over the limit fails before anything is read.
    if let Some(len) = response.body().content_length()
        && len > limits.max_bytes
    {
        return Err(FetchError::TooLarge {
            limit: limits.max_bytes,
        });
    }
    let too_large = FetchError::TooLarge {
        limit: limits.max_bytes,
    };
    // ureq's limit counts a body of exactly the limit as over it.
    let body = response
        .body_mut()
        .with_config()
        .limit(limits.max_bytes.saturating_add(1))
        .read_to_vec()
        .map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => too_large.clone(),
            e => error(e),
        })?;
    if body.len() as u64 > limits.max_bytes {
        return Err(too_large);
    }
    Ok(body)
}

fn error(e: ureq::Error) -> FetchError {
    match e {
        ureq::Error::StatusCode(code) => FetchError::Status(code),
        ureq::Error::Timeout(_) => FetchError::TimedOut,
        ureq::Error::RequireHttpsOnly(url) => FetchError::Insecure(url),
        ureq::Error::BodyExceedsLimit(limit) => FetchError::TooLarge { limit },
        e => FetchError::Network(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Serves `responses` in order, one per connection, on a local port.
    fn serve(responses: Vec<Vec<u8>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                // Read the request head before answering.
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                    line.clear();
                }
                let _ = stream.write_all(&response);
            }
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn ok(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    #[test]
    fn only_https_or_this_machine() {
        assert_eq!(check_url("https://example.org/a.acblock"), Ok(false));
        assert_eq!(check_url("HTTPS://example.org"), Ok(false));
        assert_eq!(check_url("http://localhost:8000/x"), Ok(true));
        assert_eq!(check_url("http://127.0.0.1/x"), Ok(true));
        assert_eq!(check_url("http://[::1]:9/x"), Ok(true));
        assert!(matches!(
            check_url("http://example.org/a"),
            Err(FetchError::Insecure(_))
        ));
        // A user name doesn't make a host local.
        assert!(matches!(
            check_url("http://localhost@example.org/"),
            Err(FetchError::Insecure(_))
        ));
        assert!(matches!(
            check_url("file:///etc/passwd"),
            Err(FetchError::BadUrl(_))
        ));
        assert!(matches!(
            check_url("example.org/a"),
            Err(FetchError::BadUrl(_))
        ));
        assert!(matches!(
            check_url("https:///nohost"),
            Err(FetchError::BadUrl(_))
        ));
    }

    #[test]
    fn a_file_downloads() {
        let base = serve(vec![ok(b"{\"format\":1}")]);
        let body = fetch(&format!("{base}/index.json"), Limits::DOCUMENT).unwrap();
        assert_eq!(body, b"{\"format\":1}");
    }

    #[test]
    fn errors_are_reported_not_returned_as_bodies() {
        let base = serve(vec![
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        let err = fetch(&format!("{base}/missing"), Limits::DOCUMENT).unwrap_err();
        assert_eq!(err, FetchError::Status(404));
        assert!(err.to_string().contains("404"));
    }

    #[test]
    fn a_body_over_the_limit_is_refused_declared_or_not() {
        let limits = Limits {
            max_bytes: 16,
            timeout: Duration::from_secs(5),
        };
        let base = serve(vec![ok(&[b'x'; 17])]);
        assert_eq!(
            fetch(&base, limits),
            Err(FetchError::TooLarge { limit: 16 })
        );
        // No length given: the read stops at the limit.
        let mut undeclared = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        undeclared.extend_from_slice(&[b'x'; 64]);
        let base = serve(vec![undeclared]);
        assert_eq!(
            fetch(&base, limits),
            Err(FetchError::TooLarge { limit: 16 })
        );
        // Exactly the limit is fine.
        let base = serve(vec![ok(&[b'y'; 16])]);
        assert_eq!(fetch(&base, limits).unwrap().len(), 16);
    }

    #[test]
    fn a_silent_server_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(1500));
        });
        let limits = Limits {
            max_bytes: 1024,
            timeout: Duration::from_millis(300),
        };
        let err = fetch(&format!("http://127.0.0.1:{port}/"), limits).unwrap_err();
        assert_eq!(err, FetchError::TimedOut, "{err}");
        hold.join().unwrap();
    }

    #[test]
    fn a_redirect_is_followed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let moved = format!(
            "HTTP/1.1 302 Found\r\nLocation: {base}/new\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        thread::spawn(move || {
            for response in [moved.into_bytes(), ok(b"here")] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                    line.clear();
                }
                let _ = stream.write_all(&response);
            }
        });
        assert_eq!(
            fetch(&format!("{base}/old"), Limits::DOCUMENT).unwrap(),
            b"here"
        );
    }
}
