//! Application URLs relative to a base path, and their encoding.
use crate::Route;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use std::{borrow::Cow, fmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlError(pub &'static str);
impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for UrlError {}

/// Encoded URL components relative to the application's base path. Query and
/// fragment omit their delimiters. Decode path *segments*, not the entire path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppUrl {
    pub path: String,
    pub query: String,
    pub fragment: String,
}
impl AppUrl {
    pub fn parse(value: &str) -> Result<Self, UrlError> {
        if !value.starts_with('/')
            || value.starts_with("//")
            || value.chars().any(|c| c.is_control() || c == '\\')
        {
            return Err(UrlError("expected an application-relative absolute path"));
        }
        let (without_fragment, fragment) = value.split_once('#').unwrap_or((value, ""));
        let (path, query) = without_fragment
            .split_once('?')
            .unwrap_or((without_fragment, ""));
        let url = Self {
            path: path.into(),
            query: query.into(),
            fragment: fragment.into(),
        };
        let segments = url.segments()?;
        if segments.iter().any(|s| s == "." || s == "..") {
            return Err(UrlError("dot path segments are not routes"));
        }
        Ok(url)
    }
    pub fn segments(&self) -> Result<Vec<String>, UrlError> {
        self.path
            .strip_prefix('/')
            .ok_or(UrlError("path must start with '/'"))?
            .split('/')
            .map(decode_segment)
            .collect()
    }
    /// Path segments for routing: a trailing slash adds no segment.
    pub fn route_segments(&self) -> Result<Vec<String>, UrlError> {
        let mut segments = self.segments()?;
        if segments.last().is_some_and(String::is_empty) {
            segments.pop();
        }
        Ok(segments)
    }
    /// Repeated query keys are retained, in order, using URL form encoding rules.
    pub fn query_pairs(&self) -> impl Iterator<Item = (Cow<'_, str>, Cow<'_, str>)> {
        form_urlencoded::parse(self.query.as_bytes())
    }
    pub fn query_first(&self, name: &str) -> Option<String> {
        self.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    }
}
impl fmt::Display for AppUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)?;
        if !self.query.is_empty() {
            write!(f, "?{}", self.query)?;
        }
        if !self.fragment.is_empty() {
            write!(f, "#{}", self.fragment)?;
        }
        Ok(())
    }
}

/// Decode one path segment, rejecting a `%` that does not start an escape.
fn decode_segment(segment: &str) -> Result<String, UrlError> {
    let mut bytes = segment.bytes();
    while let Some(byte) = bytes.next() {
        let escape = byte == b'%';
        if escape
            && !(bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return Err(UrlError("invalid percent encoding in path"));
        }
    }
    percent_decode_str(segment)
        .decode_utf8()
        .map(Cow::into_owned)
        .map_err(|_| UrlError("path is not UTF-8"))
}

/// Encode one dynamic path segment. A slash in a value stays inside that segment.
pub fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}
/// Encode query pairs without discarding repeated keys.
pub fn encode_query<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasePath(String);
impl BasePath {
    pub fn new(value: &str) -> Result<Self, UrlError> {
        if !value.starts_with('/')
            || !value.ends_with('/')
            || value.contains("//")
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
        {
            return Err(UrlError(
                "base must be '/' or a slash-delimited path such as '/tools/issues/'",
            ));
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn href(&self, route: &impl Route) -> Result<String, UrlError> {
        let url = AppUrl::parse(&route.path())?;
        if Route::parse(&url).as_ref() != Some(route) {
            return Err(UrlError("route path does not parse back to the same route"));
        }
        Ok(format!("{}{}", self.0.trim_end_matches('/'), url))
    }
    /// Strip only an exact path prefix, never a similarly-named sibling path.
    pub fn strip(&self, absolute_path: &str) -> Result<AppUrl, UrlError> {
        let path = absolute_path
            .strip_prefix(&self.0)
            .ok_or(UrlError("URL is outside the application base"))?;
        AppUrl::parse(&format!("/{path}"))
    }
}
