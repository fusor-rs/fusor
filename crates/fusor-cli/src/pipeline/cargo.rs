//! Artifact paths come from Cargo's JSON messages, not guessed profile
//! directories, so a changed profile cannot silently publish a stale site.
use super::{declarations, diagnostics::Diagnostics};
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::cargo,
    workspace::Project,
};
use fusor_build::app::ArtifactManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, ChildStdout, Stdio},
};

#[derive(Clone, Copy)]
pub(crate) enum Mode {
    Check,
    Build { release: bool },
}

pub(crate) struct Compilation {
    pub manifest: ArtifactManifest,
    pub wasm: Option<PathBuf>,
    /// The native renderer of an islands application.
    pub executable: Option<PathBuf>,
}

pub(crate) fn compile(cx: &Context, project: &Project, mode: Mode) -> Result<Compilation> {
    compile_target(cx, project, mode, None)
}

pub(crate) fn compile_worker(
    cx: &Context,
    project: &Project,
    mode: Mode,
    threaded: bool,
) -> Result<Compilation> {
    compile_target(cx, project, mode, Some(threaded))
}

fn compile_target(
    cx: &Context,
    project: &Project,
    mode: Mode,
    worker: Option<bool>,
) -> Result<Compilation> {
    let mut command = compilation_command(cx, project, mode, worker)?;
    let mut child = command.stdout(Stdio::piped()).spawn().map_err(|error| {
        Error::tooling(format!("could not run cargo: {error}"))
            .remedy("install Rust from https://rustup.rs")
    })?;
    let stdout = child.stdout.take().expect("Cargo stdout was piped");
    let mut diagnostics = Diagnostics::default();
    // Always reap Cargo, even after a malformed message; a compiler left
    // running would block the next build on the lock file.
    let streamed = read_messages(stdout, project, &mut diagnostics);
    let status = reap(&mut child, streamed.is_err())?;
    let streamed = streamed?;
    if !status.success() {
        return Err(Error::compile(
            "Rust compilation failed; see the compiler diagnostics above",
        ));
    }
    let manifest = streamed.artifact.ok_or_else(|| {
        Error::project("the application emitted no Fusor artifact")
            .remedy("call fusor_build::compile_app() from the package's build.rs")
    })?;
    declarations::write(&project.root, &manifest.javascript)?;
    Ok(Compilation {
        manifest,
        wasm: streamed.wasm,
        executable: streamed.executable,
    })
}

#[derive(Default)]
struct Streamed {
    artifact: Option<ArtifactManifest>,
    wasm: Option<PathBuf>,
    executable: Option<PathBuf>,
}

fn compilation_command(
    cx: &Context,
    project: &Project,
    mode: Mode,
    worker: Option<bool>,
) -> Result<std::process::Command> {
    let mut command = if worker == Some(true) {
        let mut command = crate::process::rustup("rustup");
        command.args(["run", layout::WORKER_TOOLCHAIN, "cargo"]);
        command
    } else {
        cargo()
    };
    if let Some(threaded) = worker {
        command.env("FUSOR_WORKER_BUILD", "1");
        if threaded {
            worker_rustflags(&mut command, &project.workspace)?;
        }
    } else {
        command.env_remove("FUSOR_WORKER_BUILD");
    }
    command.current_dir(&project.workspace).arg(match mode {
        Mode::Check => "check",
        Mode::Build { .. } => "build",
    });
    if let Some(delivery) = &project.config.delivery {
        command.args(["--bin", delivery.binary.as_deref().unwrap_or(&project.name)]);
    } else {
        command.args(["--lib", "--target", layout::TARGET]);
    }
    if let Some(threaded) = worker {
        command
            .arg("--target-dir")
            .arg(project.target.join(if threaded {
                layout::THREADED_TARGET
            } else {
                layout::WORKER_TARGET
            }));
        if threaded {
            command.args(["-Z", "build-std=std,panic_abort"]);
        }
    }
    command.arg("--message-format=json");
    if matches!(mode, Mode::Build { release: true }) {
        command.arg("--release");
    }
    project.flags(cx, &mut command);

    Ok(command)
}

#[derive(Default, Deserialize)]
struct CargoFlags {
    #[serde(default)]
    build: FlagConfig,
    #[serde(default)]
    target: BTreeMap<String, FlagConfig>,
}

#[derive(Default, Deserialize)]
struct FlagConfig {
    rustflags: Option<Flags>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Flags {
    String(String),
    Array(Vec<String>),
}

fn worker_rustflags(command: &mut std::process::Command, workspace: &Path) -> Result {
    for (key, separator) in [("CARGO_ENCODED_RUSTFLAGS", "\x1f"), ("RUSTFLAGS", " ")] {
        if let Ok(mut flags) = std::env::var(key) {
            if !flags.is_empty() {
                flags.push_str(separator);
            }
            flags.push_str(&layout::WORKER_RUSTFLAGS.replace('\x1f', separator));
            command.env(key, flags);
            return Ok(());
        }
    }
    let output = crate::process::rustup("rustup")
        .args([
            "run",
            layout::WORKER_TOOLCHAIN,
            "cargo",
            "-Z",
            "unstable-options",
            "config",
            "get",
            "--format",
            "json",
        ])
        .current_dir(workspace)
        .output()?;
    if !output.status.success() {
        return Err(Error::tooling(format!(
            "could not read worker Cargo configuration: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut config: CargoFlags = serde_json::from_slice(&output.stdout)?;
    if std::env::var_os("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS").is_some() {
        config
            .target
            .entry(layout::TARGET.into())
            .or_default()
            .rustflags
            .get_or_insert_with(|| Flags::Array(Vec::new()));
    }
    // Extend each existing source; Cargo still chooses matching target rules or
    // the build fallback. Adding a new target rule would hide build.rustflags.
    append_flags(command, "build.rustflags", config.build.rustflags)?;
    for (target, flags) in config.target {
        if flags.rustflags.is_some() {
            append_flags(
                command,
                &format!("target.{target:?}.rustflags"),
                flags.rustflags,
            )?;
        }
    }
    Ok(())
}

fn append_flags(command: &mut std::process::Command, key: &str, flags: Option<Flags>) -> Result {
    let flags = match flags {
        Some(Flags::String(flags)) => Flags::String(format!(
            "{flags} {}",
            layout::WORKER_RUSTFLAGS.replace('\x1f', " ")
        )),
        // Cargo concatenates arrays, so only append the additional flags.
        _ => Flags::Array(
            layout::WORKER_RUSTFLAGS
                .split('\x1f')
                .map(str::to_owned)
                .collect(),
        ),
    };
    command
        .arg("--config")
        .arg(format!("{key}={}", serde_json::to_string(&flags)?));
    Ok(())
}

fn read_messages(
    stdout: ChildStdout,
    project: &Project,
    diagnostics: &mut Diagnostics,
) -> Result<Streamed> {
    let mut streamed = Streamed::default();
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            // A build script may print to stdout. Forward it verbatim.
            eprintln!("{line}");
            continue;
        };
        let selected = message["package_id"].as_str() == Some(&project.id);
        match message["reason"].as_str() {
            Some("build-script-executed") => {
                if let Some(path) = artifact_path(&message) {
                    let manifest = ArtifactManifest::read(Path::new(path))?;
                    // A diagnostic in a dependency still points at its HTML.
                    diagnostics.register(&manifest, &project.workspace)?;
                    if selected {
                        streamed.artifact = Some(manifest);
                    }
                }
            }
            Some("compiler-artifact") if selected => {
                if let Some(path) = message["executable"].as_str() {
                    streamed.executable = Some(path.into());
                }
                for file in message["filenames"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if Path::new(file)
                        .extension()
                        .is_some_and(|name| name == "wasm")
                    {
                        streamed.wasm = Some(file.into());
                    }
                }
            }
            Some("compiler-message") => diagnostics.print(&message["message"], &project.workspace),
            _ => {}
        }
    }
    Ok(streamed)
}

fn artifact_path(message: &Value) -> Option<&str> {
    message["env"].as_array()?.iter().find_map(|pair| {
        (pair[0].as_str() == Some("FUSOR_ARTIFACT_MANIFEST"))
            .then(|| pair[1].as_str())
            .flatten()
    })
}

fn reap(child: &mut Child, failed: bool) -> Result<std::process::ExitStatus> {
    if failed {
        let _ = child.kill();
    }
    Ok(child.wait()?)
}
