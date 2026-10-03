//! Worker discovery reads compiled metadata, never authored Rust text.
use super::{
    BuildMode, Publication,
    cargo::{Mode, compile_worker},
    wasm,
};
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::checked,
    workspace::Project,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, process::Command};

#[derive(Deserialize)]
struct Capabilities {
    pool: bool,
}
#[derive(Serialize)]
pub(crate) struct Artifacts {
    version: u32,
    ordinary: String,
    threaded: Option<String>,
    generation: String,
    base: String,
}
impl Artifacts {
    pub fn boot(&self) -> Result<String> {
        Ok(include_str!("workers/artifact.js")
            .replace("__FUSOR_WORKER_ARTIFACT__", &serde_json::to_string(self)?))
    }
}

pub(crate) fn build(
    publication: &Publication<'_>,
    bindgen: &Path,
    managed_entry: bool,
    mode: BuildMode,
) -> Result<Option<Artifacts>> {
    let package = publication.generated().join(layout::PACKAGE);
    let Some(pool) = capabilities(&package)? else {
        return Ok(None);
    };
    if !managed_entry {
        return Err(Error::project("automatic workers require an <App> entry")
            .remedy("replace manual wasm_bindgen(start) mounting with an <App> entry"));
    }
    if pool {
        ensure_toolchain(publication.cx, mode == BuildMode::Development)?;
    }
    let ordinary = artifact(publication, bindgen, mode, false)?;
    let threaded = pool
        .then(|| artifact(publication, bindgen, mode, true))
        .transpose()?;
    if pool {
        fs::write(publication.staging().join(layout::WORKER_HEADERS), b"{}")?;
    }
    Ok(Some(Artifacts {
        version: 1,
        ordinary,
        threaded,
        generation: publication.generation.clone(),
        base: publication.project.config.base_path.clone(),
    }))
}

fn capabilities(package: &Path) -> Result<Option<bool>> {
    let metadata = fs::read_to_string(package.join(layout::APP_TYPES))?;
    let declarations: Vec<_> = metadata
        .lines()
        .filter(|line| line.starts_with("// fusor-worker:"))
        .collect();
    if declarations.is_empty() && !metadata.contains("function __fusor_worker_manifest(") {
        return Ok(None);
    }
    if declarations
        .iter()
        .any(|line| !line.starts_with("// fusor-worker:1:"))
    {
        return Err(Error::compile("unsupported compiled worker protocol")
            .remedy("use matching fusor CLI and worker crate versions"));
    }
    let pool = declarations.iter().any(|line| line.ends_with(":pool"))
        || pool_marker(&package.join(layout::APP_WASM))?;
    Ok((!declarations.is_empty() || pool).then_some(pool))
}

fn pool_marker(path: &Path) -> Result<bool> {
    let output = Command::new(std::env::var_os("FUSOR_NODE").unwrap_or_else(|| "node".into()))
        .args([
            "--input-type=module",
            "-e",
            include_str!("workers/inspect.mjs"),
        ])
        .arg(path)
        .output()
        .map_err(|error| {
            Error::tooling(format!("worker metadata inspection needs Node.js: {error}"))
                .remedy("install Node.js 22 or newer and rebuild the application")
        })?;
    if !output.status.success() {
        return Err(Error::compile(format!(
            "worker artifact inspection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
        .remedy("install Node 22 or newer and rebuild the application"));
    }
    Ok(serde_json::from_slice::<Capabilities>(&output.stdout)?.pool)
}

fn artifact(
    publication: &Publication<'_>,
    bindgen: &Path,
    mode: BuildMode,
    threaded: bool,
) -> Result<String> {
    let compilation = compile_worker(
        publication.cx,
        publication.project,
        Mode::Build {
            release: mode == BuildMode::Release,
        },
        threaded,
    )?;
    let binary = compilation
        .wasm
        .ok_or_else(|| Error::compile("Cargo emitted no worker WebAssembly library"))?;
    let directory = if threaded {
        layout::THREADED_DIRECTORY
    } else {
        layout::WORKER_DIRECTORY
    };
    let root = publication.generated().join(directory);
    let package = root.join(layout::PACKAGE);
    wasm::bindgen(bindgen, &binary, &package, layout::APP_NAME, mode)?;
    if threaded {
        patch_thread_host(&package)?;
    }
    fs::write(
        root.join(layout::BOOT_MODULE),
        include_str!("workers/boot.js"),
    )?;
    Ok(format!(
        "{}/{directory}/{}",
        publication.url_prefix(),
        layout::BOOT_MODULE
    ))
}

fn ensure_toolchain(cx: &Context, install: bool) -> Result {
    let available = crate::process::rustup("rustup")
        .args([
            "run",
            layout::WORKER_TOOLCHAIN,
            "rustc",
            "--print",
            "sysroot",
        ])
        .output()?;
    if available.status.success() {
        let root = std::path::PathBuf::from(String::from_utf8_lossy(&available.stdout).trim());
        if root
            .join("lib/rustlib/src/rust/library/Cargo.lock")
            .is_file()
        {
            return Ok(());
        }
    }
    if !install || cx.offline {
        return Err(Error::tooling(format!(
            "shared-memory workers require {} with rust-src",
            layout::WORKER_TOOLCHAIN
        ))
        .remedy("run `fusor install` while online to prepare the worker toolchain"));
    }
    cx.reporter.step(format!(
        "Installing worker toolchain {} ...",
        layout::WORKER_TOOLCHAIN
    ));
    checked(Command::new("rustup").args([
        "toolchain",
        "install",
        layout::WORKER_TOOLCHAIN,
        "--profile",
        "minimal",
        "--component",
        "rust-src",
        "--target",
        layout::TARGET,
        "--no-self-update",
    ]))
}

pub(crate) fn prepare(cx: &Context, project: &Project) -> Result {
    let metadata = crate::workspace::read_metadata(cx, Some(&project.manifest))?;
    let dependencies = metadata.dependency_ids(&project.id)?;
    if !metadata
        .packages
        .iter()
        .any(|package| package.name == "fusor-worker" && dependencies.contains(&package.id))
    {
        return Ok(());
    }
    // Build the normal artifact to ask the actual feature/cfg graph whether it
    // uses pools. Ordinary-only applications keep their stable toolchain.
    let compilation = super::cargo::compile(cx, project, Mode::Build { release: false })?;
    let Some(wasm) = compilation.wasm else {
        return Ok(());
    };
    let directory = project.target.join(layout::WORKER_DISCOVERY);
    let bindgen = crate::toolchain::bindgen::resolve()?;
    wasm::bindgen(
        &bindgen,
        &wasm,
        &directory,
        layout::APP_NAME,
        BuildMode::Debug,
    )?;
    if capabilities(&directory)? == Some(true) {
        ensure_toolchain(cx, true)?;
        compile_worker(cx, project, Mode::Build { release: false }, true)?;
    }
    Ok(())
}

fn patch_thread_host(package: &Path) -> Result {
    let mut found = false;
    for entry in fs::read_dir(package.join("snippets"))? {
        let path = entry?.path().join("src/workerHelpers.no-bundler.js");
        if path.is_file() {
            if found {
                return Err(Error::compile(
                    "multiple Rayon worker adapters in one artifact",
                ));
            }
            fs::write(path, include_str!("workers/rayon.js"))?;
            found = true;
        }
    }
    if !found {
        return Err(
            Error::compile("threaded worker artifact is missing the pinned Rayon adapter")
                .remedy("use the pinned wasm-bindgen-rayon backend through fusor-worker"),
        );
    }
    Ok(())
}
