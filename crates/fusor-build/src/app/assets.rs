//! `[package.metadata.fusor.assets]`: URL paths and the files published there.
use super::Result;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

/// The files one entry publishes. Written as a path or pattern, a list of
/// them, or `{ files = …, suffix = ".txt" }`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    untagged,
    expecting = "a path or pattern, a list of them, or { files = ..., suffix = \".txt\" }"
)]
pub enum AssetSource {
    Files(AssetFiles),
    Suffixed(SuffixedAssets),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AssetFiles {
    One(AssetPattern),
    Many(Vec<AssetPattern>),
}

/// Appends `suffix` to every published file name, such as `.txt` so a browser
/// displays source code instead of interpreting it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuffixedAssets {
    pub files: AssetFiles,
    pub suffix: String,
}

/// A package-relative path or glob pattern. `..` may lead outside the package.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct AssetPattern(String);

impl AssetSource {
    pub fn patterns(&self) -> &[AssetPattern] {
        let (Self::Files(files) | Self::Suffixed(SuffixedAssets { files, .. })) = self;
        match files {
            AssetFiles::One(pattern) => std::slice::from_ref(pattern),
            AssetFiles::Many(patterns) => patterns,
        }
    }

    pub fn suffix(&self) -> Option<&str> {
        match self {
            Self::Files(_) => None,
            Self::Suffixed(suffixed) => Some(&suffixed.suffix),
        }
    }

    /// `output` is the application's output directory, which no pattern may search.
    pub(super) fn validate(&self, url: &str, output: &Path) -> Result<()> {
        if !valid_url(url) {
            return Err(format!(
                "asset URL {url:?} must be '/', a directory such as '/source/' or a file \
                 such as '/install.sh', without empty, '.' or '..' segments"
            )
            .into());
        }
        if self.patterns().is_empty() {
            return Err(format!("asset URL {url:?} lists no files").into());
        }
        if self
            .suffix()
            .is_some_and(|suffix| suffix.is_empty() || suffix.contains(['/', '\\']))
        {
            return Err(format!(
                "the suffix for asset URL {url:?} must be nonempty file name text \
                 such as \".txt\""
            )
            .into());
        }
        for pattern in self.patterns() {
            pattern.validate(output)?;
        }
        Ok(())
    }
}

impl AssetPattern {
    /// Splits at the first component containing a wildcard (`*`, `?` or `[`):
    /// the literal path before it and the pattern from it on. Matches are
    /// published relative to the literal path.
    pub fn split(&self) -> (&str, Option<&str>) {
        let mut start = 0;
        for component in self.0.split('/') {
            if component.contains(['*', '?', '[']) {
                return (
                    self.0[..start].trim_end_matches('/'),
                    Some(&self.0[start..]),
                );
            }
            start += component.len() + 1;
        }
        (&self.0, None)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(&self, output: &Path) -> Result<()> {
        if self.0.is_empty() || Path::new(&self.0).has_root() {
            return Err(format!(
                "asset pattern {:?} must be a nonempty package-relative path",
                self.0
            )
            .into());
        }
        let base = Path::new(self.split().0);
        let inside_package = base
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if inside_package && (base.starts_with(output) || output.starts_with(base)) {
            return Err(format!(
                "asset pattern {:?} overlaps the output directory; start it with a directory \
                 such as \"public/\"",
                self.0
            )
            .into());
        }
        Ok(())
    }
}

/// `/`, a directory such as `/source/`, or a file such as `/install.sh`.
fn valid_url(url: &str) -> bool {
    if url == "/" {
        return true;
    }
    let Some(path) = url.strip_prefix('/') else {
        return false;
    };
    path.strip_suffix('/')
        .unwrap_or(path)
        .split('/')
        .all(|segment| !matches!(segment, "" | "." | "..") && !segment.contains('\\'))
}
