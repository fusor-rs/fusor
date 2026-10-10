//! Doctor changes nothing and runs without Cargo, since one of the problems it
//! reports is that Rust is missing. Every check is a file read or a version
//! query.
use crate::{
    context::Context,
    error::{Error, Result},
    layout, toolchain,
    workspace::select,
};
use std::{fs, path::Path};

pub(crate) fn run(cx: &Context) -> Result {
    let mut problems = Vec::new();
    match select::from_filesystem(cx) {
        Ok(Some(candidate)) => {
            cx.reporter
                .result(format!("Manifest: {}", candidate.manifest.display()))?;
            inspect(cx, &candidate.manifest, &mut problems)?;
        }
        Ok(None) => problems.push(
            "no Cargo.toml was found; run doctor inside an application or pass --manifest-path"
                .to_owned(),
        ),
        Err(error) => problems.push(error.to_string()),
    }
    match toolchain::bindgen::resolve() {
        Ok(binary) => cx.reporter.result(format!(
            "wasm-bindgen {}: {}",
            layout::BINDGEN_VERSION,
            binary.display()
        ))?,
        Err(error) => problems.push(error.to_string()),
    }

    if problems.is_empty() {
        cx.reporter.result(
            "Local prerequisites are ready. Run `fusor check --frozen` to validate the dependency graph and application.",
        )?;
        return Ok(());
    }
    for problem in &problems {
        cx.reporter.warn(problem);
    }
    Err(
        Error::project(format!("doctor found {} problem(s)", problems.len()))
            .remedy("no files were changed and no tools were installed"),
    )
}

fn inspect(cx: &Context, manifest: &Path, problems: &mut Vec<String>) -> Result {
    let document: toml::Value = fs::read_to_string(manifest)?.parse()?;
    let root = manifest
        .parent()
        .ok_or_else(|| Error::internal("manifest has no parent directory"))?;

    check_cohort(&document, problems)?;
    check_application(manifest, &document, problems);
    check_lockfile(cx, root, problems)?;
    check_toolchain(cx, root, problems)?;
    check_javascript(root, problems);
    Ok(())
}

/// Otherwise this surfaces later as a confusing resolution error.
fn check_cohort(document: &toml::Value, problems: &mut Vec<String>) -> Result {
    let cli = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
    for section in ["dependencies", "build-dependencies"] {
        let Some(dependencies) = document.get(section).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, dependency) in dependencies {
            if name != "fusor" && !name.starts_with("fusor-") {
                continue;
            }
            let required = dependency
                .as_str()
                .or_else(|| dependency.get("version").and_then(toml::Value::as_str));
            let Some(required) = required else { continue };
            if !semver::VersionReq::parse(required)?.matches(&cli) {
                problems.push(format!(
                    "{name} requires {required}, which this CLI {cli} does not satisfy; install the corresponding CLI release"
                ));
            }
        }
    }
    Ok(())
}

fn check_application(manifest: &Path, document: &toml::Value, problems: &mut Vec<String>) {
    let declared = document
        .get("package")
        .and_then(|package| package.get("metadata"))
        .and_then(|metadata| metadata.get("fusor"))
        .is_some();
    if !declared {
        problems.push(
            "this manifest is not an application; select one with --manifest-path (workspace roots are not applications)"
                .to_owned(),
        );
    } else if let Err(error) = fusor_build::app::AppConfig::load(manifest) {
        problems.push(format!("invalid [package.metadata.fusor]: {error}"));
    }
}

fn check_lockfile(cx: &Context, root: &Path, problems: &mut Vec<String>) -> Result {
    let lock = root
        .ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|file| file.is_file());
    match lock {
        Some(lock) => {
            // A corrupt lock is clearer here than as a Cargo failure.
            let _: toml::Value = fs::read_to_string(&lock)?.parse()?;
            cx.reporter.result(format!(
                "Cargo lockfile: {} (graph resolution is checked by `fusor check`)",
                lock.display()
            ))?;
        }
        None => problems.push("Cargo.lock is missing; run `fusor install`".to_owned()),
    }
    Ok(())
}

fn check_toolchain(cx: &Context, root: &Path, problems: &mut Vec<String>) -> Result {
    if let Err(error) = toolchain::rust::preflight(cx, root, false) {
        problems.push(error.to_string());
    } else if !toolchain::rust::target_ready(root)? {
        problems.push(format!(
            "the {} target is missing; run `fusor install`",
            layout::TARGET
        ));
    }
    Ok(())
}

fn check_javascript(root: &Path, problems: &mut Vec<String>) {
    if !root.join("package.json").is_file() {
        return;
    }
    if !root.join("package-lock.json").is_file() {
        problems.push(
            "package-lock.json is missing; run `fusor add javascript` for the initial resolution"
                .to_owned(),
        );
    }
    if !root.join("node_modules").is_dir() {
        problems
            .push("JavaScript dependencies are missing; run `fusor install --locked`".to_owned());
    }
}
