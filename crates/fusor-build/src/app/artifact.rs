use super::Result;
use crate::html;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub const ARTIFACT_VERSION: u32 = 5;

/// File names inside Cargo's `OUT_DIR`.
pub(crate) const MANIFEST_FILE: &str = "fusor_artifacts.json";
pub(crate) const HTML_FILE: &str = "fusor_app.html";
pub(crate) const MODULE_FILE: &str = "fusor_module.rs";
pub(crate) const LIBRARY_MANIFEST_FILE: &str = "fusor_library.json";
pub(crate) const LIBRARY_TEMPLATES_FILE: &str = "fusor_library.html";

/// Paths are absolute. The CLI discovers this manifest from Cargo JSON messages,
/// never by guessing profile directories or package library filenames.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub version: u32,
    pub html: PathBuf,
    pub module: PathBuf,
    pub loader_offset: usize,
    pub managed_entry: bool,
    pub sources: Vec<SourceArtifact>,
    #[serde(default)]
    pub javascript: Vec<JavaScriptArtifact>,
    /// Every class a `class:name` binding toggles. The markup holds no trace of
    /// them, so CSS tools that scan sources for class names need this list.
    pub classes: BTreeSet<String>,
}

/// A component library's part of the page of each application that depends
/// on it. Paths are absolute. The CLI finds it as it finds [`ArtifactManifest`].
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryManifest {
    pub version: u32,
    /// The library's `<template>` elements.
    pub templates: PathBuf,
    /// The package directory, which Tailwind scans for class names.
    pub root: PathBuf,
    /// The namespace of the library's component identities. Two libraries in
    /// one page must differ.
    pub library: u32,
    pub sources: Vec<SourceArtifact>,
    /// As [`ArtifactManifest::classes`].
    pub classes: BTreeSet<String>,
}

/// Native browser modules discovered from component templates, plus editor types.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JavaScriptArtifact {
    pub id: String,
    pub path: PathBuf,
    pub source: PathBuf,
    pub line: usize,
    pub column: usize,
    pub component: String,
    pub inline: bool,
    pub declaration: PathBuf,
    pub declaration_name: String,
    pub rust_source: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceArtifact {
    pub name: String,
    pub source: PathBuf,
    pub rust: PathBuf,
    pub fingerprint: PathBuf,
    pub map: PathBuf,
    /// Authored Rust is compiled in its native module, not copied into `rust`.
    pub external: Option<PathBuf>,
    pub registration: Option<RegistrationArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationArtifact {
    pub rust: PathBuf,
    pub line: usize,
    pub column: usize,
}

impl ArtifactManifest {
    pub fn read(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(path)?)?;
        supported(manifest.version)?;
        if manifest.sources.is_empty() {
            return Err("fusor artifact manifest contains no source modules".into());
        }
        Ok(manifest)
    }

    /// The entry page with the templates of `libraries` before `</body>`, and
    /// the loader's offset in it.
    pub fn page<'a>(
        &self,
        libraries: impl IntoIterator<Item = &'a LibraryManifest>,
    ) -> Result<(String, usize)> {
        let mut page = fs::read_to_string(&self.html)?;
        let mut loader_offset = self.loader_offset;
        let templates = libraries
            .into_iter()
            .map(|library| fs::read_to_string(&library.templates))
            .collect::<std::io::Result<String>>()?;
        html::insert_before_body_end(&mut page, &templates, Some(&mut loader_offset));
        Ok((page, loader_offset))
    }
}

impl LibraryManifest {
    pub fn read(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&fs::read(path)?)?;
        supported(manifest.version)?;
        Ok(manifest)
    }
}

fn supported(version: u32) -> Result<()> {
    if version != ARTIFACT_VERSION {
        return Err("unsupported fusor artifact version; rebuild with matching tools".into());
    }
    Ok(())
}
