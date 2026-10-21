//! `audioc version` and its update check, against a local feed.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;

/// A one-request server answering with `body`, standing in for the
/// release feed.
fn feed(body: &'static str) -> String {
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
    format!("http://127.0.0.1:{port}/latest")
}

fn audioc(feed: Option<&str>, args: &[&str]) -> (bool, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_audioc"));
    cmd.args(args);
    if let Some(url) = feed {
        cmd.env("AC_UPDATE_FEED", url);
    }
    let out = cmd.output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn version_prints_without_asking_anyone() {
    // A feed that would fail if asked.
    let (ok, out, _) = audioc(Some("http://127.0.0.1:1/never"), &["version"]);
    assert!(ok);
    assert_eq!(out, format!("audioc {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn version_check_names_a_newer_release_and_where_to_get_it() {
    let url = feed(r#"{"tag_name":"v99.0.0","html_url":"https://example.org/releases/v99.0.0"}"#);
    let (ok, out, err) = audioc(Some(&url), &["version", "--check"]);
    assert!(ok, "{err}");
    assert!(out.contains("A newer release is out: 99.0.0"), "{out}");
    assert!(
        out.contains("https://example.org/releases/v99.0.0"),
        "{out}"
    );
}

#[test]
fn version_check_says_when_this_is_the_newest() {
    let url = feed(r#"{"tag_name":"v0.0.1"}"#);
    let (ok, out, err) = audioc(Some(&url), &["version", "--check"]);
    assert!(ok, "{err}");
    assert!(out.contains("This is the newest release"), "{out}");
}

#[test]
fn version_check_fails_with_a_reason_when_offline() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (ok, _, err) = audioc(
        Some(&format!("http://127.0.0.1:{port}/")),
        &["version", "--check"],
    );
    assert!(!ok);
    assert!(err.contains("couldn't download"), "{err}");
    let (ok, _, err) = audioc(Some("http://example.org/feed"), &["version", "--check"]);
    assert!(!ok);
    assert!(err.contains("isn't secure"), "{err}");
}
