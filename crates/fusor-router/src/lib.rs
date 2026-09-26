//! Typed application URLs. Parsing and formatting are ordinary Rust functions.
//! Enable `browser` for an owned History API router and component outlet.
#[cfg(feature = "browser")]
pub mod browser;
pub mod pattern;
#[cfg(any(feature = "browser", test))]
mod select;
mod url;
pub use url::{AppUrl, BasePath, UrlError, encode_query, encode_segment};

/// Application-defined route identity. Equal routes retain their mounted view.
/// Implement `parse(path(route)) == Some(route)`; the browser adapter checks it.
pub trait Route: Clone + PartialEq + 'static {
    fn parse(url: &AppUrl) -> Option<Self>;
    /// An application-relative absolute path, for example `/issues/42`.
    fn path(&self) -> String;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Location<R> {
    pub url: AppUrl,
    pub route: Option<R>,
}
impl<R: Route> Location<R> {
    pub fn new(url: AppUrl) -> Self {
        let route = R::parse(&url);
        Self { url, route }
    }
}
