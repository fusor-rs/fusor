//! Tailwind CSS publishes one self-contained binary per platform, so styling
//! needs no Node. Its version is pinned like wasm-bindgen's.
use super::download;
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

/// Far above the real size (~110 MB), so a wrong URL cannot stream forever.
const DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// The release asset's platform suffix for this CLI's host.
fn platform() -> Option<&'static str> {
    Some(match env!("FUSOR_HOST") {
        "aarch64-apple-darwin" => "macos-arm64",
        "x86_64-apple-darwin" => "macos-x64",
        "aarch64-unknown-linux-gnu" => "linux-arm64",
        "x86_64-unknown-linux-gnu" => "linux-x64",
        "aarch64-unknown-linux-musl" => "linux-arm64-musl",
        "x86_64-unknown-linux-musl" => "linux-x64-musl",
        "x86_64-pc-windows-msvc" => "windows-x64.exe",
        _ => return None,
    })
}

fn root() -> Result<PathBuf> {
    super::tool_root("tailwindcss", layout::TAILWIND_VERSION)
}

fn executable_name() -> String {
    format!("tailwindcss{}", env::consts::EXE_SUFFIX)
}

/// `FUSOR_TAILWIND`, then the cache. `PATH` is not searched: a global
/// `tailwindcss` is usually an npm shim for another version.
pub(crate) fn resolve() -> Result<PathBuf> {
    let binary = match env::var_os("FUSOR_TAILWIND") {
        Some(binary) => PathBuf::from(binary),
        None => root()?.join("bin").join(executable_name()),
    };
    if !reports_expected_version(&binary) {
        return Err(Error::tooling(format!(
            "Tailwind CSS {} is not available at {}",
            layout::TAILWIND_VERSION,
            binary.display()
        ))
        .remedy(format!(
            "run `fusor install`, or set FUSOR_TAILWIND to a tailwindcss {} binary",
            layout::TAILWIND_VERSION
        )));
    }
    Ok(binary)
}

pub(crate) fn install(cx: &Context) -> Result {
    if resolve().is_ok() || env::var_os("FUSOR_TAILWIND").is_some() {
        return resolve().map(drop);
    }
    if cx.offline {
        return Err(Error::tooling(format!(
            "Tailwind CSS {} is not in the tool cache",
            layout::TAILWIND_VERSION
        ))
        .remedy("run `fusor install` while online, or set FUSOR_TAILWIND"));
    }
    let platform = platform().ok_or_else(|| {
        Error::tooling(format!(
            "Tailwind CSS publishes no standalone binary for {}",
            env!("FUSOR_HOST")
        ))
        .remedy(format!(
            "set FUSOR_TAILWIND to a tailwindcss {} binary",
            layout::TAILWIND_VERSION
        ))
    })?;
    let release = format!("tailwindcss-v{}-{platform}", layout::TAILWIND_VERSION);
    let checksum = download::checksum(&release)?
        .ok_or_else(|| Error::internal(format!("no pinned checksum for {release}")))?;
    let url = format!(
        "https://github.com/tailwindlabs/tailwindcss/releases/download/v{}/tailwindcss-{platform}",
        layout::TAILWIND_VERSION
    );
    cx.reporter.step(format!(
        "Downloading Tailwind CSS {} for {} ...",
        layout::TAILWIND_VERSION,
        env!("FUSOR_HOST")
    ));
    let root = root()?;
    let (staging, _cleanup) = download::staging(&root)?;
    let binary = staging.join(executable_name());
    download::fetch("Tailwind CSS", &url, &checksum, DOWNLOAD_LIMIT, &binary)?;
    download::make_executable(&binary)?;
    download::install(
        "Tailwind CSS",
        &binary,
        &root.join("bin").join(executable_name()),
        reports_expected_version,
    )
}

/// `--help` opens with `≈ tailwindcss v<version>`.
fn reports_expected_version(binary: &Path) -> bool {
    let expected = format!("tailwindcss v{}", layout::TAILWIND_VERSION);
    Command::new(binary)
        .arg("--help")
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.trim_end().ends_with(&expected))
        })
}
