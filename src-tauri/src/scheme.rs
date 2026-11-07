//! The `hive-file:` scheme (24.2, 24.3): a worktree's files shown in the webview (an image, a
//! PDF), at `hive-file://localhost/<token>/<path>` (`http://hive-file.localhost/…` on Windows),
//! the token issued by the service for a followed worktree. `main.rs` registers it; the bytes
//! come from the service in ranges (`Hive::serve_file`).

use serde_json::Value;
use tauri::http::Response;

/// The scheme's name, as `main.rs` registers it.
pub const SCHEME: &str = "hive-file";

/// Where the webview reaches the scheme: WKWebView takes it as is, WebView2 as an `http` host.
fn origin(macos: bool) -> &'static str {
    if macos {
        "hive-file://localhost/"
    } else {
        "http://hive-file.localhost/"
    }
}

/// `text` percent-encoded but for the unreserved characters (RFC 3986), so any file name is
/// one URL path segment.
fn encoded(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `text` percent-decoded; `None` for a broken escape or bytes that are not UTF-8.
fn decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        rest = after;
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let hex = rest
            .get(..2)
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
        bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
        rest = &rest[2..];
    }
    String::from_utf8(bytes).ok()
}

/// The URL of the file `path` (relative, `/`-separated) of the worktree `token` names, at its
/// `version`, so a changed file gets a new URL and loads again.
pub fn url(macos: bool, token: &str, path: &str, version: &str) -> String {
    let path: Vec<String> = path.split('/').map(encoded).collect();
    let (token, version) = (encoded(token), encoded(version));
    format!("{}{token}/{}?v={version}", origin(macos), path.join("/"))
}

/// The worktree token and the file's relative path of a scheme request's URL path
/// (`/<token>/<path>`, still percent-encoded); the service checks the path.
pub fn request(path: &str) -> Option<(String, String)> {
    let (token, file) = path.strip_prefix('/')?.split_once('/')?;
    Some((decoded(token)?, decoded(file)?))
}

/// Whether the webview may load `url` in a frame (24.3, the PDF): the scheme's own origin on
/// this platform. Never an IPC origin (`ipc_from_app`, 24.14).
pub fn file_url(url: &tauri::Url, macos: bool) -> bool {
    let (scheme, host) = if macos {
        (SCHEME, "localhost")
    } else {
        ("http", "hive-file.localhost")
    };
    url.scheme() == scheme && url.host_str() == Some(host) && url.port().is_none()
}

/// Gives a `file` message's `media` (24.2) the URL the viewer shows it from.
pub fn with_url(file: &mut Value, macos: bool) {
    let path = file["path"].as_str().unwrap_or_default().to_owned();
    let Some(media) = file.get_mut("media") else {
        return;
    };
    let url = match (media["token"].as_str(), media["version"].as_str()) {
        (Some(token), Some(version)) => url(macos, token, &path, version),
        _ => return,
    };
    media["url"] = url.into();
}

/// The CSP of a scheme answer of type `mime`, one place for every type (24.4 adds HTML's).
/// Anything shown as a document (an SVG opened from a PDF's link, a 404) is sandboxed, with
/// no script and no network: in the scheme's origin it could read the token from its URL and
/// fetch the worktree's files. A PDF is drawn by the WebView's viewer, which a sandbox blanks.
pub fn policy(mime: &str) -> &'static str {
    match mime {
        "application/pdf" => "default-src 'none'",
        _ => "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox",
    }
}

/// The scheme's answer: the file's bytes as the service's type `mime`, or why not (404), with
/// its [`policy`]. Never sniffed by the webview.
pub fn response(read: Result<(String, Vec<u8>), String>) -> Response<Vec<u8>> {
    let (status, mime, body) = match read {
        Ok((mime, body)) => (200, mime, body),
        Err(error) => (
            404,
            "text/plain; charset=utf-8".to_owned(),
            error.into_bytes(),
        ),
    };
    Response::builder()
        .status(status)
        .header("Content-Security-Policy", policy(&mime))
        .header("Content-Type", mime)
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_file_url_names_its_worktree_s_token_and_its_path_encoded() {
        let url = url(true, "ab12", "dir/a b+%é.png", "12-ff");
        assert_eq!(
            url,
            "hive-file://localhost/ab12/dir/a%20b%2B%25%C3%A9.png?v=12-ff"
        );
        let path = url.strip_prefix("hive-file://localhost").unwrap();
        let path = path.split('?').next().unwrap();
        assert_eq!(
            request(path),
            Some(("ab12".into(), "dir/a b+%é.png".into()))
        );
        let windows = super::url(false, "t", "a.pdf", "1-2");
        assert_eq!(windows, "http://hive-file.localhost/t/a.pdf?v=1-2");
    }

    #[test]
    fn a_request_needs_a_token_and_a_path_that_decode() {
        assert_eq!(request("/t/a/b.png"), Some(("t".into(), "a/b.png".into())));
        for path in [
            "", "t/a.png", "/t", "/t/a%2", "/t/a%zz", "/t/a%+1", "/t/a%FF", "/%FF/a",
        ] {
            assert_eq!(request(path), None, "{path}");
        }
        assert_eq!(decoded("%41%2f%2F"), Some("A//".into()));
        assert_eq!(decoded("~-._"), Some("~-._".into()));
        assert_eq!(encoded("~-._Az09/"), "~-._Az09%2F");
    }

    #[test]
    fn only_the_scheme_s_own_origin_loads_in_a_frame() {
        let url = |text: &str| tauri::Url::parse(text).unwrap();
        assert!(file_url(&url("hive-file://localhost/t/a.pdf"), true));
        assert!(file_url(&url("http://hive-file.localhost/t/a.pdf"), false));
        for (text, macos) in [
            ("http://hive-file.localhost/t/a.pdf", true),
            ("hive-file://localhost/t/a.pdf", false),
            ("hive-file://evil/t/a.pdf", true),
            ("hive-file://localhost:8080/t/a.pdf", true),
            ("http://hive-file.localhost:8080/t/a.pdf", false),
            ("https://hive-file.localhost/t/a.pdf", false),
            ("http://hive-file.localhost.evil.com/", false),
            ("tauri://localhost/", true),
        ] {
            assert!(!file_url(&url(text), macos), "{text} (macos: {macos})");
        }
    }

    #[test]
    fn a_file_message_s_media_gets_its_url() {
        let mut file = json!({"type": "file", "path": "a b.png", "media": {"mime": "image/png", "token": "t", "version": "4-1"}});
        with_url(&mut file, true);
        assert_eq!(
            file["media"]["url"],
            "hive-file://localhost/t/a%20b.png?v=4-1"
        );
        with_url(&mut file, false);
        assert_eq!(
            file["media"]["url"],
            "http://hive-file.localhost/t/a%20b.png?v=4-1"
        );
        // No media, or an older service's file: unchanged.
        for before in [
            json!({"type": "file", "path": "a", "media": null}),
            json!({"type": "file", "path": "a"}),
            json!({"type": "file", "path": "a", "media": {"token": "t"}}),
        ] {
            let mut after = before.clone();
            with_url(&mut after, true);
            assert_eq!(after, before);
        }
    }

    #[test]
    fn the_answer_carries_the_service_s_type_and_is_never_sniffed() {
        let ok = response(Ok(("image/png".into(), vec![1, 2])));
        assert_eq!(ok.status(), 200);
        assert_eq!(ok.headers()["Content-Type"], "image/png");
        assert_eq!(ok.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(ok.body(), &[1, 2]);
        let failed = response(Err("a.png does not exist".into()));
        assert_eq!(failed.status(), 404);
        assert_eq!(
            failed.headers()["Content-Type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(failed.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(failed.body(), b"a.png does not exist");
        // Every answer has its type's CSP: sandboxed, no script, no network, but a PDF's.
        let sandboxed = "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox";
        for answer in [&ok, &failed] {
            assert_eq!(answer.headers()["Content-Security-Policy"], sandboxed);
        }
        let pdf = response(Ok(("application/pdf".into(), vec![])));
        let csp = &pdf.headers()["Content-Security-Policy"];
        assert_eq!(csp, "default-src 'none'");
    }

    #[test]
    fn an_svg_or_any_other_file_shown_as_a_document_runs_nothing() {
        // An SVG opened from a PDF's link would run its scripts in the scheme's origin.
        for mime in ["image/svg+xml", "application/octet-stream", "text/plain"] {
            let csp = response(Ok((mime.into(), vec![])));
            let csp = csp.headers()["Content-Security-Policy"].to_str().unwrap();
            assert!(csp.ends_with("; sandbox"), "{mime}: {csp}");
            assert!(csp.starts_with("default-src 'none';"), "{mime}: {csp}");
            assert!(!csp.contains("script"), "{mime}: {csp}");
        }
        assert_eq!(policy("application/pdf"), "default-src 'none'");
    }
}
