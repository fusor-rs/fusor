use crate::{SourceError, SourceMapError};
use std::{error::Error, fmt, io, path::StripPrefixError};

/// A failed build-script or external-backend compilation step.
/// Source diagnostics retain the authored file and optional HTML location.
pub enum BuildError {
    Io(io::Error),
    Configuration(String),
    Manifest(toml::de::Error),
    Rust(syn::Error),
    Json(serde_json::Error),
    Source(SourceError),
    SourceMap(SourceMapError),
    Path(StripPrefixError),
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Configuration(message) => formatter.write_str(message),
            Self::Manifest(error) => error.fmt(formatter),
            Self::Rust(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::Source(error) => error.fmt(formatter),
            Self::SourceMap(error) => error.fmt(formatter),
            Self::Path(error) => error.fmt(formatter),
        }
    }
}

// Cargo reports build-script errors with Debug; preserve authored locations.
impl fmt::Debug for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for BuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Configuration(_) => None,
            Self::Manifest(error) => Some(error),
            Self::Rust(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Source(error) => Some(error),
            Self::SourceMap(error) => Some(error),
            Self::Path(error) => Some(error),
        }
    }
}

macro_rules! from_error {
    ($($variant:ident($type:ty)),* $(,)?) => {
        $(impl From<$type> for BuildError {
            fn from(error: $type) -> Self {
                Self::$variant(error)
            }
        })*
    };
}

from_error!(
    Io(io::Error),
    Configuration(String),
    Manifest(toml::de::Error),
    Rust(syn::Error),
    Json(serde_json::Error),
    Source(SourceError),
    SourceMap(SourceMapError),
    Path(StripPrefixError),
);

impl From<&str> for BuildError {
    fn from(message: &str) -> Self {
        Self::Configuration(message.to_owned())
    }
}
