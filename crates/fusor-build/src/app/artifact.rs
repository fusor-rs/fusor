use super::Result;
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
        if manifest.version != ARTIFACT_VERSION {
            return Err("unsupported fusor artifact version; rebuild with matching tools".into());
        }
        if manifest.sources.is_empty() {
            return Err("fusor artifact manifest contains no source modules".into());
        }
        Ok(manifest)
    }
}
