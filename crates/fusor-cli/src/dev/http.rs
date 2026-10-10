//! Request policy with no socket and no lock. Only files inside the published
//! site are served.
use crate::{error::Result, layout, pipeline::manifest::OutputManifest};
use hyper::{Method, Response, header::HeaderValue};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
pub(crate) type Body = Response<Vec<u8>>;

pub(crate) fn error_response(message: &str) -> Body {
    build_response(500, "text/plain", message.as_bytes().to_vec(), false)
}

pub(crate) struct Route<'a> {
    pub directory: &'a Path,
    pub base: &'a str,
    pub history_fallback: &'a [String],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Serving {
    Development,
    Preview,
}

pub(crate) fn respond(
    method: &Method,
    url: &str,
    accept: &str,
    route: Route,
    serving: Serving,
) -> Body {
    let Route {
        directory,
        base,
        history_fallback,
    } = route;
    let url = url.split('?').next().unwrap_or("");
    if !matches!(method, &Method::GET | &Method::HEAD) {
        return build_response(405, "text/plain", b"Method not allowed".to_vec(), false);
    }
    let Some(relative) = url.strip_prefix(base) else {
        // Redirect a base path missing its trailing slash, so relative URLs
        // resolve.
        if base != "/" && url == base.trim_end_matches('/') {
            return Response::builder()
                .status(308)
                .header("Location", base)
                .body(Vec::new())
                .expect("validated base path");
        }
        return build_response(404, "text/plain", b"Not found".to_vec(), false);
    };
    if serving == Serving::Development {
        if let Some(response) = development_endpoint(relative, directory) {
            return response;
        }
    }
    let immutable = relative
        .strip_prefix(&format!("{}/", layout::GENERATED))
        .is_some_and(|path| path.contains('/'));
    let mut response = match read_file(directory, relative) {
        Ok(Some((path, body))) => build_response(200, mime(&path), body, immutable),
        Ok(None) if document_fallback(relative, accept, history_fallback) => {
            return respond(method, base, "", route, serving);
        }
        Ok(None) => build_response(404, "text/plain", b"Not found".to_vec(), false),
        Err(error) => error_response(&error.to_string()),
    };
    if directory.join(layout::WORKER_HEADERS).is_file() {
        response.headers_mut().insert(
            "Cross-Origin-Opener-Policy",
            HeaderValue::from_static("same-origin"),
        );
        response.headers_mut().insert(
            "Cross-Origin-Embedder-Policy",
            HeaderValue::from_static("require-corp"),
        );
    }
    response
}

fn development_endpoint(relative: &str, directory: &Path) -> Option<Body> {
    let generated = |name: &str| format!("{}/{name}", layout::GENERATED);
    if relative == generated("version") {
        return Some(match OutputManifest::read(directory) {
            Ok(output) => build_response(
                200,
                "text/plain",
                format!("{}:{}", output.generation, output.revision()).into_bytes(),
                false,
            ),
            Err(error) => error_response(&error.to_string()),
        });
    }
    if relative == generated("update") {
        return Some(match update(directory) {
            Ok(body) => build_response(200, "application/json", body, false),
            Err(error) => error_response(&error.to_string()),
        });
    }
    None
}

fn update(directory: &Path) -> Result<Vec<u8>> {
    let output = OutputManifest::read(directory)?;
    Ok(serde_json::to_vec(&serde_json::json!({
        "generation": output.generation,
        "revision": output.revision(),
        "reload_after": output.reload_after(),
        "html": fs::read_to_string(directory.join("index.html"))?
    }))?)
}

/// Canonicalizing both sides is what makes the containment check hold against
/// links.
fn read_file(directory: &Path, relative: &str) -> std::io::Result<Option<(PathBuf, Vec<u8>)>> {
    let Some(relative) = decode_path(relative) else {
        return Ok(None);
    };
    let root = directory.canonicalize()?;
    let path = match directory.join(relative).canonicalize() {
        Ok(path) => path,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if !path.starts_with(&root) || !fs::metadata(&path)?.is_file() {
        return Ok(None);
    }
    let body = fs::read(&path)?;
    Ok(Some((path, body)))
}

pub(crate) fn build_response(
    status: u16,
    content_type: &str,
    body: Vec<u8>,
    immutable: bool,
) -> Body {
    let cache = if status == 200 && immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-store"
    };
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Content-Length", body.len())
        .header("Cache-Control", cache)
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("static response headers")
}

/// Only for requests that asked for HTML, under a declared prefix, for nothing
/// that looks like a file. Guessing would turn a missing asset into a 200.
fn document_fallback(relative: &str, accept: &str, prefixes: &[String]) -> bool {
    if !accepts_html(accept) {
        return false;
    }
    let Some(path) = decode_path(relative) else {
        return false;
    };
    if path
        .components()
        .any(|part| part.as_os_str().to_string_lossy().contains('.'))
        || ["api", "assets", layout::GENERATED]
            .iter()
            .any(|prefix| path.starts_with(prefix))
    {
        return false;
    }
    let path = format!("/{}", path.to_string_lossy().replace('\\', "/"));
    prefixes.iter().any(|prefix| {
        prefix == "/"
            || &path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn accepts_html(accept: &str) -> bool {
    accept.split(',').any(|entry| {
        let mut parts = entry.trim().split(';');
        parts
            .next()
            .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/html"))
            && !parts.any(|parameter| {
                parameter
                    .trim()
                    .strip_prefix("q=")
                    .is_some_and(|q| !q.parse::<f32>().is_ok_and(|weight| weight > 0.0))
            })
    })
}

/// Rejects traversal, backslashes, null bytes and dotfiles rather than
/// normalizing them.
fn decode_path(url: &str) -> Option<PathBuf> {
    let mut bytes = url.bytes();
    let mut decoded = Vec::new();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)?;
            let low = (bytes.next()? as char).to_digit(16)?;
            (high * 16 + low) as u8
        } else {
            byte
        });
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded.contains(['\0', '\\']) {
        return None;
    }
    let path = PathBuf::from(decoded);
    if path.components().any(
        |part| !matches!(part, Component::Normal(name) if !name.to_string_lossy().starts_with('.')),
    ) {
        return None;
    }
    Some(if path.as_os_str().is_empty() {
        "index.html".into()
    } else {
        path
    })
}

fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|name| name.to_str())
        .unwrap_or("")
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" | "map" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_are_not_found_but_missing_site_roots_are_server_errors() {
        let directory = std::env::temp_dir().join(format!(
            "fusor-http-{}",
            crate::pipeline::publish::generation().unwrap()
        ));
        let _cleanup = crate::transaction::Staging(directory.clone());
        let respond = |path| {
            super::respond(
                &Method::GET,
                path,
                "text/html",
                Route {
                    directory: &directory,
                    base: "/",
                    history_fallback: &["/".into()],
                },
                Serving::Preview,
            )
        };
        assert_eq!(respond("/article").status(), 500);
        fs::create_dir(&directory).unwrap();
        assert_eq!(respond("/missing.js").status(), 404);
        fs::write(directory.join("index.html"), "home").unwrap();
        assert_eq!(respond("/article").body(), b"home");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("cycle", directory.join("cycle")).unwrap();
            assert_eq!(respond("/cycle").status(), 500);
        }
    }

    #[test]
    fn history_fallback_is_explicit_and_only_for_documents() {
        let prefixes = vec!["/articles".into(), "/about".into()];
        for path in [
            "articles",
            "articles/1",
            "articles/1/",
            "articles/%32",
            "about",
        ] {
            assert!(
                document_fallback(path, "text/html,application/xhtml+xml;q=0.9", &prefixes),
                "{path}"
            );
        }
        for path in [
            "article",
            "articles-old",
            "about-face",
            "api/articles",
            "__fusor/pkg",
            "articles/a.js",
            "articles/a%2Ewasm",
            "articles/../secret",
            "articles/%2e%2e/secret",
            "articles/%",
            "src/lib.rs",
        ] {
            assert!(!document_fallback(path, "text/html", &prefixes), "{path}");
        }
        assert!(!document_fallback("articles/1", "*/*", &prefixes));
        assert!(!document_fallback("articles/1", "text/html;q=0", &prefixes));
        assert!(!document_fallback("articles/1", "text/html", &[]));
    }

    #[test]
    fn request_paths_decode_without_escaping_or_exposing_internal_files() {
        assert_eq!(decode_path(""), Some("index.html".into()));
        assert_eq!(
            decode_path("hello%20world.svg"),
            Some("hello world.svg".into())
        );
        for path in [
            "../Cargo.toml",
            "%2e%2e/Cargo.toml",
            "%2fetc/passwd",
            "%00",
            "%zz",
            "%",
            "a%5cb",
            ".fusor-output.json",
            "/index.html",
        ] {
            assert!(decode_path(path).is_none(), "accepted {path}");
        }
    }
}
