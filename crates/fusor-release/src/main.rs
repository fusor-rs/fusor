//! Packages and smoke-tests CLI release candidates. Nothing here uploads, signs
//! or publishes.
mod archive;
mod package;
mod smoke;

use clap::{Parser, Subcommand};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(
    name = "fusor-release",
    about = "Package and smoke-test CLI candidates"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a reviewable archive and its checksum. Never uploads or publishes.
    Package {
        /// The Rust target triple these binaries were built for
        #[arg(long)]
        target: String,
        /// Where the built binaries are, if not `target/<triple>/release`
        #[arg(long)]
        binary_directory: Option<PathBuf>,
        #[arg(long, default_value = "target/release-artifacts")]
        output: PathBuf,
    },
    /// Create, check, build and preview an application using one built binary.
    Smoke {
        /// The `fusor` executable to exercise
        binary: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Package {
            target,
            binary_directory,
            output,
        } => package::run(&target, binary_directory.as_deref(), &output),
        Command::Smoke { binary } => smoke::run(&binary),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fusor-release: {error}");
            ExitCode::FAILURE
        }
    }
}

/// From the crate's location, so the commands run from any directory.
fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the workspace root")
}

fn workspace_version() -> Result<String> {
    let manifest: toml::Value =
        std::fs::read_to_string(workspace_root().join("Cargo.toml"))?.parse()?;
    Ok(manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("the workspace declares no package version")?
        .to_owned())
}

fn remove_directory(path: &Path) -> Result {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("removing {}: {error}", path.display()).into()),
    }
}
