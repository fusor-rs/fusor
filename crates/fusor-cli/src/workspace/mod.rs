//! A [`Project`] holds facts Cargo resolved, never user preferences. Those stay
//! in [`Context`], so `dev` can re-derive a project without rebuilding flags.
pub(crate) mod metadata;
pub(crate) mod select;

use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::cargo,
};
use fusor_build::app::AppConfig;
use metadata::{Metadata, Package};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// A graph mixing releases of these compiles but generates subtly incompatible
/// code.
const COHORT: &[&str] = &[
    "fusor-core",
    "fusor-build",
    "fusor-macros",
    "fusor-components",
    "fusor-router",
    "fusor-async",
    "fusor-query",
    "fusor-std",
    "fusor-islands",
    "fusor-server",
    "fusor-npm",
    "fusor-worker",
    "fusor-worker-macros",
];

#[derive(Clone)]
pub(crate) struct Project {
    pub id: String,
    pub name: String,
    pub manifest: PathBuf,
    pub root: PathBuf,
    pub workspace: PathBuf,
    pub target: PathBuf,
    /// This application and its transitive path dependencies.
    pub watch_roots: Vec<PathBuf>,
    pub config: AppConfig,
}

impl Project {
    pub fn discover(cx: &Context) -> Result<Self> {
        let located = select::from_filesystem(cx)?.map(|candidate| candidate.manifest);
        Self::discover_at(cx, located.as_deref())
    }

    /// For a caller that already needed the filesystem's answer.
    pub fn discover_at(cx: &Context, located: Option<&Path>) -> Result<Self> {
        let metadata = read_metadata(cx, located)?;
        let package = select::from_metadata(&metadata, cx, located)?;
        cx.reporter.note(format!(
            "selected {} from {}",
            package.name,
            package.manifest_path.display()
        ));
        let root = package.root()?.to_owned();
        let config = AppConfig::load(&package.manifest_path)?;
        validate_targets(package, &config)?;

        let dependencies = metadata.dependency_ids(&package.id)?;
        validate_cohort(&metadata, &dependencies)?;
        let mut watch_roots = path_dependency_roots(&metadata, &dependencies);
        if let Some(delivery) = &config.delivery {
            for unit in delivery.units.values() {
                let unit_package = metadata
                    .packages
                    .iter()
                    .find(|package| package.name == unit.package)
                    .ok_or_else(|| {
                        Error::project(format!("unknown delivery package {}", unit.package))
                            .remedy("name a workspace package in [package.metadata.fusor.delivery]")
                    })?;
                let ids = metadata.dependency_ids(&unit_package.id)?;
                watch_roots.extend(path_dependency_roots(&metadata, &ids));
            }
            watch_roots.sort();
            watch_roots.dedup();
        }

        Ok(Self {
            id: package.id.clone(),
            name: package.name.clone(),
            manifest: package.manifest_path.clone(),
            root,
            workspace: metadata.workspace_root.clone(),
            target: metadata.target_directory.clone(),
            watch_roots,
            config,
        })
    }

    pub fn rediscover(&self, cx: &Context) -> Result<Self> {
        Self::discover(&cx.for_manifest(self.manifest.clone(), Some(self.name.clone())))
    }

    pub fn output(&self, cx: &Context) -> PathBuf {
        cx.output
            .clone()
            .unwrap_or_else(|| self.root.join(&self.config.output))
    }

    /// Publication renames directories. Doing that to an authored folder, or
    /// through a symlink, would lose data, so an existing output must be empty
    /// or carry our own output manifest.
    pub fn validate_output(&self, cx: &Context) -> Result {
        let output = self.output(cx);
        // Stop at the package or at Cargo's target directory, where
        // `build --site` builds each application first.
        for path in output
            .ancestors()
            .take_while(|path| *path != self.root && *path != self.target)
        {
            if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(Error::project(format!(
                    "output path contains a symlink: {}",
                    path.display()
                ))
                .remedy("publish to a real directory; fusor never renames through a symlink"));
            }
        }
        let occupied = output.exists()
            && (!output.is_dir()
                || (fs::read_dir(&output)?.next().is_some()
                    && !output.join(layout::OUTPUT_MANIFEST).is_file()));
        if occupied {
            return Err(Error::project(format!(
                "{} is not empty and is not owned by fusor",
                output.display()
            ))
            .remedy("set output in [package.metadata.fusor] to a directory fusor owns"));
        }
        Ok(())
    }

    pub fn flags(&self, cx: &Context, command: &mut Command) {
        if cx.verbose {
            command.arg("--verbose");
        }
        // Cargo's per-crate progress is noise next to our own; its diagnostics
        // still arrive as JSON messages. `--verbose` brings the progress back.
        if !cx.verbose {
            command.arg("--quiet");
        }
        command.arg("--color").arg(cx.color.as_str());
        for features in &cx.features {
            command.arg("--features").arg(features);
        }
        command
            .arg("--manifest-path")
            .arg(&self.manifest)
            .arg("--package")
            .arg(&self.id);
        if cx.offline {
            command.arg("--offline");
        }
        if cx.locked {
            command.arg("--locked");
        }
    }
}

pub(crate) fn read_metadata(cx: &Context, located: Option<&Path>) -> Result<Metadata> {
    let mut command = cargo();
    command.args(["metadata", "--format-version", "1"]);
    if let Some(path) = located {
        command.arg("--manifest-path").arg(path);
        if let Some(directory) = path.parent() {
            command.current_dir(directory);
        }
    }
    if cx.offline {
        command.arg("--offline");
    }
    if cx.locked {
        command.arg("--locked");
    }
    for features in &cx.features {
        command.arg("--features").arg(features);
    }
    let output = command
        .output()
        .map_err(|error| Error::tooling(format!("could not run cargo metadata: {error}")))?;
    if !output.status.success() {
        return Err(Error::project(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ))
        .remedy("run `fusor install` if Cargo.lock is missing or stale"));
    }
    Metadata::decode(&output.stdout)
}

/// The wrong crate shape is clearer here than as a Cargo error later.
fn validate_targets(package: &Package, config: &AppConfig) -> Result {
    match &config.delivery {
        None if !package.has_crate_type("cdylib") => Err(Error::project(format!(
            "{} does not build a Wasm library",
            package.name
        ))
        .remedy("add crate-type = [\"cdylib\", \"rlib\"] to its [lib] section")),
        Some(delivery) => {
            let binary = delivery.binary.as_deref().unwrap_or(&package.name);
            if package.has_target(binary, "bin") {
                Ok(())
            } else {
                Err(
                    Error::project(format!("islands application {} has no renderer binary named {binary}", package.name))
                        .remedy("add the binary target, or set delivery.binary when its name differs from the package"),
                )
            }
        }
        None => Ok(()),
    }
}

/// Registry packages cannot change under a running `fusor dev`.
fn path_dependency_roots(metadata: &Metadata, ids: &BTreeSet<String>) -> Vec<PathBuf> {
    metadata
        .packages
        .iter()
        .filter(|package| package.source.is_none() && ids.contains(&package.id))
        .filter_map(|package| package.manifest_path.parent().map(Path::to_owned))
        .collect()
}

fn validate_cohort(metadata: &Metadata, dependencies: &BTreeSet<String>) -> Result {
    let cli = env!("CARGO_PKG_VERSION");
    for package in metadata
        .packages
        .iter()
        .filter(|package| dependencies.contains(&package.id))
    {
        if COHORT.contains(&package.name.as_str()) && package.version != cli {
            return Err(Error::project(format!(
                "this application uses {} {}, but you are running fusor {cli}",
                package.name, package.version
            ))
            .remedy(format!(
                "install the matching CLI with `cargo install fusor-cli --version ={} --locked`",
                package.version
            )));
        }
        if package.name == "wasm-bindgen" && package.version != layout::BINDGEN_VERSION {
            return Err(Error::project(format!(
                "this application depends on wasm-bindgen {}, but fusor {cli} generates code for {}",
                package.version,
                layout::BINDGEN_VERSION
            ))
            .remedy(format!(
                "set wasm-bindgen = \"={}\" in the application's Cargo.toml",
                layout::BINDGEN_VERSION
            )));
        }
    }
    Ok(())
}
