//! Rust, the Wasm target, wasm-bindgen, Tailwind CSS and npm packages.
//! `build` and `check` never prepare; `install`, `new` and `dev` do, and may
//! use the network.
pub(crate) mod bindgen;
mod download;
pub(crate) mod node;
pub(crate) mod rust;
pub(crate) mod tailwind;

use crate::{
    context::Context,
    error::{Error, Result},
    process::{cargo, checked},
    workspace::Project,
};
use std::{env, path::PathBuf};

pub(crate) fn cache_dir() -> Result<PathBuf> {
    if let Some(root) = env::var_os("FUSOR_CACHE_DIR") {
        return Ok(PathBuf::from(root));
    }
    let home = |variable: &str| {
        env::var_os(variable).ok_or_else(|| {
            Error::tooling(format!("{variable} is not set"))
                .remedy("set FUSOR_CACHE_DIR to a writable directory")
        })
    };
    Ok(if cfg!(target_os = "windows") {
        PathBuf::from(home("LOCALAPPDATA")?).join("Fusor/Cache")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(home("HOME")?).join("Library/Caches/fusor")
    } else if let Some(root) = env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(root).join("fusor")
    } else {
        PathBuf::from(home("HOME")?).join(".cache/fusor")
    })
}

/// Keyed by version and host, so projects on different releases do not fight
/// over one binary.
fn tool_root(tool: &str, version: &str) -> Result<PathBuf> {
    Ok(cache_dir()?
        .join(tool)
        .join(version)
        .join(env!("FUSOR_HOST")))
}

/// `explicit` is `install` and `new`: fetch Cargo dependencies and always
/// restore `node_modules`. `dev` only repairs what is missing.
pub(crate) fn prepare(
    cx: &Context,
    project: &Project,
    explicit: bool,
    initial_javascript: bool,
) -> Result {
    if explicit {
        cx.reporter.step(if cx.locked {
            "Fetching Cargo dependencies from Cargo.lock ..."
        } else {
            "Fetching Cargo dependencies ..."
        });
        let mut fetch = cargo();
        fetch
            .arg("fetch")
            .arg("--manifest-path")
            .arg(&project.manifest)
            .current_dir(&project.root);
        if cx.offline {
            fetch.arg("--offline");
        }
        if cx.locked {
            fetch.arg("--locked");
        }
        if cx.quiet {
            fetch.arg("--quiet");
        }
        checked(&mut fetch)?;
    }
    rust::install_target(cx, &project.root)?;
    bindgen::install(cx, &project.root)?;
    if project.config.tailwind.is_some() {
        tailwind::install(cx)?;
    }
    node::prepare(cx, project, explicit, initial_javascript)?;
    if explicit {
        cx.reporter
            .done("Prepared dependencies and WebAssembly tools");
    }
    Ok(())
}
