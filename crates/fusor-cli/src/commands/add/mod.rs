//! Edits dependencies only, never application source. The manifest edit is
//! delegated to `cargo add` so comments, formatting and feature merging behave
//! as Cargo defines them. On failure, see [`OwnedFile`] for what is restored.
mod source;

use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::cargo,
    toolchain,
    transaction::OwnedFile,
    workspace::Project,
};
use clap::ValueEnum;
use source::{Source, inherited};
use std::fs;
use toml_edit::DocumentMut;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Capability {
    Router,
    Async,
    Query,
    Forms,
    Actions,
    Javascript,
}

impl Capability {
    /// Query brings async because its API takes an async request context.
    fn packages(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            Self::Router => &[("fusor-router", &["browser"])],
            Self::Async => &[("fusor-async", &["browser"])],
            Self::Query => &[("fusor-query", &["browser"]), ("fusor-async", &["browser"])],
            Self::Forms => &[("fusor-std", &["forms", "browser"])],
            Self::Actions => &[("fusor-std", &["actions", "browser"])],
            Self::Javascript => &[("fusor-core", &["javascript"])],
        }
    }

    fn is_javascript(self) -> bool {
        matches!(self, Self::Javascript)
    }
}

pub(crate) fn run(cx: &Context, capability: Capability, dry_run: bool) -> Result {
    // A dry run must not create or rewrite a lockfile.
    let mut selection = cx.clone();
    selection.locked = true;
    let project = super::application(&selection, super::Prepare::None)?;

    let document: DocumentMut = fs::read_to_string(&project.manifest)?.parse()?;
    let workspace_manifest = project.workspace.join("Cargo.toml");
    let workspace: DocumentMut = fs::read_to_string(&workspace_manifest)?.parse()?;
    let source = Source::resolve(&project, &document, &workspace)?;

    for (name, features) in capability.packages() {
        source.check_existing(&project, &document, &workspace, name)?;
        cx.reporter.result(format!(
            "{}: {name} ={}, features [{}]{}",
            project.manifest.display(),
            env!("CARGO_PKG_VERSION"),
            features.join(", "),
            if inherited(&document, name) {
                " (features added only to this workspace member)"
            } else {
                ""
            }
        ));
    }
    if capability.is_javascript() {
        cx.reporter.result(format!(
            "{}: esbuild {}; resolve package-lock.json and install npm dependencies",
            project.root.join("package.json").display(),
            layout::ESBUILD_VERSION
        ));
    }
    if dry_run {
        cx.reporter.result("Dry run: no files changed.");
        return Ok(());
    }
    if cx.locked {
        return Err(Error::usage("`fusor add` changes declared dependencies")
            .remedy("omit --locked and --frozen; use --offline for cached resolution"));
    }

    let mut files = owned_files(&project, &workspace_manifest, capability)?;
    let addition = Addition {
        document: &document,
        capability,
        source: &source,
    };
    let result = addition.apply(cx, &project, &mut files);
    if let Err(error) = result {
        for file in &files {
            file.rollback(&cx.reporter)?;
        }
        return Err(error.remedy("the files this operation wrote were restored; retry `fusor add` once the problem is resolved"));
    }
    cx.reporter
        .result("Capability added; the Cargo dependency graph was validated.");
    Ok(())
}

struct Addition<'a> {
    document: &'a DocumentMut,
    capability: Capability,
    source: &'a Source,
}

impl Addition<'_> {
    fn apply(&self, cx: &Context, project: &Project, files: &mut [OwnedFile]) -> Result {
        let Self {
            document,
            capability,
            source,
        } = self;
        for (name, features) in capability.packages() {
            let mut command = cargo();
            let requirement = if !source.is_checkout() && !inherited(document, name) {
                format!("{name}@={}", env!("CARGO_PKG_VERSION"))
            } else {
                name.to_string()
            };
            command
                .arg("add")
                .arg(requirement)
                .arg("--manifest-path")
                .arg(&project.manifest)
                .arg("--no-optional")
                .arg("--features")
                .arg(features.join(","))
                .current_dir(&project.root);
            if !inherited(document, name) {
                if source.is_checkout() {
                    command.arg("--path").arg(source.crate_path(name)?);
                } else if let Some(registry) = source.registry() {
                    command.arg("--registry").arg(registry);
                }
            }
            if cx.offline {
                command.arg("--offline");
            }
            if cx.quiet {
                command.arg("--quiet");
            }
            // A failing `cargo add` may still have written.
            let status = command.status();
            capture(files)?;
            if !status?.success() {
                return Err(Error::project(format!("`cargo add {name}` failed")));
            }
        }
        if capability.is_javascript() {
            add_esbuild(cx, project, files)?;
        }
        let mut locked = cx.clone();
        locked.locked = true;
        project.rediscover(&locked).map(drop)
    }
}

fn add_esbuild(cx: &Context, project: &Project, files: &mut [OwnedFile]) -> Result {
    let path = project.root.join("package.json");
    let mut npm: serde_json::Value = if path.is_file() {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        serde_json::json!({"name": project.name, "private": true, "type": "module"})
    };
    if !npm.is_object() {
        return Err(Error::project("package.json must contain an object"));
    }
    let mut present = false;
    for section in ["dependencies", "devDependencies"] {
        let Some(dependencies) = npm.get(section) else {
            continue;
        };
        let dependencies = dependencies
            .as_object()
            .ok_or_else(|| Error::project(format!("package.json {section} must be an object")))?;
        if let Some(version) = dependencies.get("esbuild") {
            if version != layout::ESBUILD_VERSION {
                return Err(Error::project(format!(
                    "the existing esbuild version {version} differs from the supported {}",
                    layout::ESBUILD_VERSION
                ))
                .remedy("align it deliberately before retrying"));
            }
            present = true;
        }
    }
    if !present {
        if npm.get("devDependencies").is_none() {
            npm["devDependencies"] = serde_json::json!({});
        }
        npm["devDependencies"]
            .as_object_mut()
            .ok_or_else(|| Error::project("package.json devDependencies must be an object"))?
            .insert("esbuild".into(), layout::ESBUILD_VERSION.into());
    }
    let mut bytes = serde_json::to_vec_pretty(&npm)?;
    bytes.push(b'\n');
    fs::write(&path, bytes)?;
    capture(files)?;
    let installed = toolchain::node::npm(cx, &project.root, &["install"]);
    capture(files)?;
    installed
}

fn owned_files(
    project: &Project,
    workspace_manifest: &std::path::Path,
    capability: Capability,
) -> Result<Vec<OwnedFile>> {
    let mut paths = vec![
        project.manifest.clone(),
        workspace_manifest.to_owned(),
        project.workspace.join("Cargo.lock"),
    ];
    if capability.is_javascript() {
        paths.extend([
            project.root.join("package.json"),
            project.root.join("package-lock.json"),
        ]);
    }
    paths.sort();
    paths.dedup();
    paths.into_iter().map(OwnedFile::snapshot).collect()
}

fn capture(files: &mut [OwnedFile]) -> Result {
    for file in files {
        file.capture()?;
    }
    Ok(())
}
