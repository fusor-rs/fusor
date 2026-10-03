//! Cargo integration independent of browser delivery metadata.

use super::GeneratedSource;
use crate::ExtractError;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    env, fs,
    path::{Component, Path, PathBuf},
};

type Result<T> = std::result::Result<T, crate::BuildError>;

/// Version of the backend build manifest and namespaced include layout.
pub const OUTPUT_VERSION: u32 = 1;

/// An explicit set of package-local templates, independent of web delivery.
///
/// Each package compiles the templates it owns, then includes the generated
/// implementations in the Rust module owning its component types. An external
/// build helper may discover sources using its own configuration first.
#[derive(Debug, Clone)]
pub struct BuildInputs {
    root: PathBuf,
    sources: Vec<Source>,
}

/// A validated HTML source with a stable package-relative include identity.
#[derive(Debug, Clone)]
pub struct Source {
    path: PathBuf,
    canonical: PathBuf,
}

impl Source {
    /// Package-relative path, also used beneath the backend output directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Canonical source path for diagnostics and dependency tracking.
    pub fn canonical(&self) -> &Path {
        &self.canonical
    }
}

impl BuildInputs {
    /// Validate explicit paths without loading a Cargo metadata namespace.
    ///
    /// Absolute paths, `..`, duplicate files, non-files, and symlinks escaping
    /// the package are rejected. Sources are compiled in deterministic order.
    pub fn new(
        package_root: impl AsRef<Path>,
        paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<Self> {
        let root = package_root.as_ref().canonicalize()?;
        let mut seen = BTreeSet::new();
        let mut sources = Vec::new();
        for path in paths {
            let path = path.as_ref();
            relative_path(path)?;
            let canonical = root.join(path).canonicalize()?;
            if !canonical.starts_with(&root) || !canonical.is_file() {
                return Err(format!(
                    "HTML source must be a file inside its package: {}",
                    path.display()
                )
                .into());
            }
            if !seen.insert(canonical.clone()) {
                return Err(
                    format!("HTML source registered more than once: {}", path.display()).into(),
                );
            }
            sources.push(Source {
                path: path.to_owned(),
                canonical,
            });
        }
        if sources.is_empty() {
            return Err("backend compilation requires at least one source".into());
        }
        sources.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self { root, sources })
    }

    /// Validate sources relative to Cargo's current package.
    pub fn from_cargo(paths: impl IntoIterator<Item = impl AsRef<Path>>) -> Result<Self> {
        let root = env::var_os("CARGO_MANIFEST_DIR")
            .ok_or("backend compilation must run from build.rs")?;
        Self::new(root, paths)
    }

    pub fn package_root(&self) -> &Path {
        &self.root
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }
}

/// One generated template and the source map a diagnostic adapter must read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSource {
    /// Canonical authored HTML path.
    pub source: PathBuf,
    /// Package-relative include identity.
    pub template: PathBuf,
    /// Canonical generated Rust path.
    pub rust: PathBuf,
    /// Canonical source-map path in the existing `fusor-source-map-v1` format.
    pub source_map: PathBuf,
}

/// Backend-specific manifest; it does not modify the browser artifact schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputManifest {
    pub version: u32,
    pub namespace: String,
    pub sources: Vec<OutputSource>,
}

impl OutputManifest {
    /// Read a manifest and reject an incompatible output contract.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let manifest: Self = serde_json::from_str(&fs::read_to_string(path)?)?;
        if manifest.version != OUTPUT_VERSION {
            return Err("unsupported fusor backend output version; rebuild the package".into());
        }
        namespace(&manifest.namespace)?;
        Ok(manifest)
    }
}

/// Compile validated inputs into Cargo's `OUT_DIR` without web configuration.
///
/// `lower` uses the supported compiler backend facade and may perform additional
/// backend-specific checks. All sources are lowered successfully before output
/// files are changed. Writes use the isolated layout
/// `OUT_DIR/fusor_backends/<namespace>/<package-relative-path>.rs`, with adjacent
/// `.map` files and a `manifest.json`. Browser `compile_app()` output is separate.
/// Include the Rust with `include!(concat!(env!("OUT_DIR"),
/// "/fusor_backends/memory/ui/panel.html.rs"))`, or an external helper's macro
/// expanding to that expression. Namespaces must be unique per helper/backend.
///
/// Emit `cargo::rerun-if-changed` separately for the helper's configuration and
/// discovery directories. This function watches every validated HTML source.
pub fn compile_cargo(
    inputs: &BuildInputs,
    output_namespace: &str,
    lower: impl FnMut(&Source, &str) -> std::result::Result<GeneratedSource, ExtractError>,
) -> Result<OutputManifest> {
    let out = env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?;
    for source in inputs.sources() {
        println!("cargo::rerun-if-changed={}", source.canonical.display());
    }
    compile_into(inputs, Path::new(&out), output_namespace, lower)
}

fn compile_into(
    inputs: &BuildInputs,
    out: &Path,
    output_namespace: &str,
    mut lower: impl FnMut(&Source, &str) -> std::result::Result<GeneratedSource, ExtractError>,
) -> Result<OutputManifest> {
    namespace(output_namespace)?;
    let out = out.canonicalize()?;
    let mut generated = Vec::new();
    for source in inputs.sources() {
        // Recheck identity in case a helper changed files after validation.
        let canonical = inputs.root.join(&source.path).canonicalize()?;
        if canonical != source.canonical {
            return Err("HTML source changed identity after validation".into());
        }
        let html = fs::read_to_string(&canonical)?;
        let output = lower(source, &html)
            .map_err(|error| crate::app::SourceError::extracted(&source.canonical, error))?;
        generated.push((source, output));
    }
    let directory = out.join("fusor_backends").join(output_namespace);
    let mut manifest = OutputManifest {
        version: OUTPUT_VERSION,
        namespace: output_namespace.to_owned(),
        sources: Vec::new(),
    };
    for (source, generated) in generated {
        let path = directory.join(&source.path);
        let rust = append_extension(&path, "rs");
        let map = append_extension(&path, "map");
        confined_write(&out, &rust, &generated.rust)?;
        confined_write(&out, &map, &generated.source_map.to_string())?;
        manifest.sources.push(OutputSource {
            source: source.canonical.clone(),
            template: source.path.clone(),
            rust,
            source_map: map,
        });
    }
    confined_write(
        &out,
        &directory.join("manifest.json"),
        &serde_json::to_string_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn append_extension(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    name.into()
}

fn namespace(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(
            "backend output namespace must contain only ASCII letters, digits, '_' or '-'".into(),
        );
    }
    Ok(())
}

fn relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || path.to_str().is_none()
    {
        return Err(format!(
            "HTML paths must be UTF-8 package-relative paths without '..': {}",
            path.display()
        )
        .into());
    }
    Ok(())
}

fn confined_write(out: &Path, path: &Path, text: &str) -> Result<()> {
    let relative = path.strip_prefix(out)?;
    let mut ancestor = out.to_owned();
    for part in relative.components() {
        ancestor.push(part);
        match fs::symlink_metadata(&ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("backend output must not follow symlinks".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    fs::create_dir_all(path.parent().ok_or("output file has no parent")?)?;
    fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceMap;

    fn source() -> (tempfile::TempDir, BuildInputs) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("ui")).unwrap();
        fs::write(root.path().join("ui/panel.html"), "<template></template>").unwrap();
        let inputs = BuildInputs::new(root.path(), ["ui/panel.html"]).unwrap();
        (root, inputs)
    }

    fn lower(_: &Source, _: &str) -> std::result::Result<GeneratedSource, ExtractError> {
        Ok(GeneratedSource {
            rust: "const VALUE: u32 = 42;\n".into(),
            source_map: SourceMap::default(),
        })
    }

    #[test]
    fn explicit_inputs_reject_escape_duplicates_and_directories() {
        let (root, _) = source();
        for paths in [
            vec!["../outside.html"],
            vec!["ui"],
            vec!["ui/panel.html", "ui/panel.html"],
        ] {
            assert!(BuildInputs::new(root.path(), paths).is_err());
        }
    }

    #[test]
    fn namespaces_preserve_browser_and_other_backend_outputs() {
        let (_root, inputs) = source();
        let out = tempfile::tempdir().unwrap();
        let browser = out.path().join("fusor_templates/ui/panel.html.rs");
        fs::create_dir_all(browser.parent().unwrap()).unwrap();
        fs::write(&browser, "browser output").unwrap();
        let first = compile_into(&inputs, out.path(), "memory", lower).unwrap();
        let second = compile_into(&inputs, out.path(), "other", lower).unwrap();
        assert_ne!(first.sources[0].rust, second.sources[0].rust);
        assert_eq!(fs::read_to_string(browser).unwrap(), "browser output");
        let loaded =
            OutputManifest::read(out.path().join("fusor_backends/memory/manifest.json")).unwrap();
        assert_eq!(loaded.sources[0].template, Path::new("ui/panel.html"));
        assert!(compile_into(&inputs, out.path(), "../escape", lower).is_err());
    }

    #[test]
    fn failed_lowering_does_not_publish_output() {
        let (_root, inputs) = source();
        let out = tempfile::tempdir().unwrap();
        let result = compile_into(&inputs, out.path(), "memory", |_, _| {
            Err(ExtractError {
                line: 4,
                column: 2,
                message: "unsupported operation".into(),
            })
        });
        let error = result.unwrap_err();
        assert!(matches!(error, crate::BuildError::Source(_)), "{error}");
        assert_eq!(
            error.to_string(),
            format!(
                "{}:4:2: unsupported operation",
                inputs.sources()[0].canonical().display()
            )
        );
        assert!(!out.path().join("fusor_backends").exists());
    }

    #[cfg(unix)]
    #[test]
    fn output_symlinks_cannot_escape_out_dir() {
        let (_root, inputs) = source();
        let out = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), out.path().join("fusor_backends")).unwrap();
        assert!(compile_into(&inputs, out.path(), "memory", lower).is_err());
        assert!(fs::read_dir(elsewhere.path()).unwrap().next().is_none());
    }
}
