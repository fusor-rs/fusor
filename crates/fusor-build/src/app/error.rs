use crate::{ExtractError, location};
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// A problem with one file or directory of the application, shown as
/// `path[:line:column]: message`.
pub struct SourceError {
    pub path: PathBuf,
    /// One-based HTML line and character column, when available.
    pub location: Option<(usize, usize)>,
    pub message: String,
}

impl SourceError {
    pub(crate) fn new(path: &Path, message: impl fmt::Display) -> Self {
        Self {
            path: path.to_owned(),
            location: None,
            message: message.to_string(),
        }
    }

    pub(crate) fn at(path: &Path, line: usize, column: usize, message: impl fmt::Display) -> Self {
        Self {
            location: Some((line, column)),
            ..Self::new(path, message)
        }
    }

    pub(crate) fn at_offset(
        path: &Path,
        source: &str,
        offset: usize,
        message: impl fmt::Display,
    ) -> Self {
        let (line, column) = location(source, offset);
        Self::at(path, line, column, message)
    }

    pub(crate) fn extracted(path: &Path, error: ExtractError) -> Self {
        Self::at(path, error.line, error.column, error.message)
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.path.display())?;
        if let Some((line, column)) = self.location {
            write!(formatter, ":{line}:{column}")?;
        }
        write!(formatter, ": {}", self.message)
    }
}

// A build script's `main` reports errors with Debug; keep the readable location.
impl fmt::Debug for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for SourceError {}
