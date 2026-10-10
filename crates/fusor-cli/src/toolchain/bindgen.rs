//! The glue and the Rust it binds must come from the same wasm-bindgen, so every
//! path here ends with the binary reporting the exact pinned version.
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    process::{cargo, checked},
    transaction::Staging,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

/// Far above the real size (~10 MB), so a wrong URL cannot stream forever.
const DOWNLOAD_LIMIT: u64 = 100 * 1024 * 1024;
const ENTRY_LIMIT: u64 = 256 * 1024 * 1024;

fn executable_name() -> String {
    format!("wasm-bindgen{}", env::consts::EXE_SUFFIX)
}

/// `FUSOR_WASM_BINDGEN`, then the cache, then `PATH`. An explicit override is
/// authoritative even when wrong: saying so beats quietly using another.
pub(crate) fn resolve() -> Result<PathBuf> {
    let cached = super::tools_root()
        .ok()
        .map(|root| root.join("bin").join(executable_name()));
    let binary = env::var_os("FUSOR_WASM_BINDGEN")
        .map(PathBuf::from)
        .or_else(|| cached.filter(|path| path.is_file()))
        .unwrap_or_else(|| "wasm-bindgen".into());
    let output = Command::new(&binary)
        .arg("--version")
        .output()
        .map_err(|_| {
            Error::tooling(format!(
                "wasm-bindgen {} is not available",
                layout::BINDGEN_VERSION
            ))
            .remedy("run `fusor install`, or set FUSOR_WASM_BINDGEN to its path")
        })?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || reported.trim() != expected_version() {
        return Err(Error::tooling(format!(
            "{} does not report wasm-bindgen {}",
            binary.display(),
            layout::BINDGEN_VERSION
        ))
        .remedy("run `fusor install`, or correct FUSOR_WASM_BINDGEN"));
    }
    Ok(binary)
}

fn expected_version() -> String {
    format!("wasm-bindgen {}", layout::BINDGEN_VERSION)
}

pub(crate) fn install(cx: &Context, project_root: &Path) -> Result {
    if resolve().is_ok() {
        return Ok(());
    }
    if env::var_os("FUSOR_WASM_BINDGEN").is_some() {
        return resolve().map(drop);
    }
    if cx.offline {
        return Err(Error::tooling(format!(
            "wasm-bindgen {} is not in the tool cache or on PATH",
            layout::BINDGEN_VERSION
        ))
        .remedy("run `fusor install` while online, or set FUSOR_WASM_BINDGEN"));
    }
    let root = super::tools_root()?;
    if !download(cx, &root)? {
        cx.reporter.step(format!(
            "Compiling wasm-bindgen {} from source; no prebuilt tool supports this host, so this may take several minutes ...",
            layout::BINDGEN_VERSION
        ));
        // Cargo owns its own install lock and only installs after a successful
        // compile, so an interrupted run cannot leave a broken binary behind.
        checked(
            cargo()
                .args([
                    "install",
                    "wasm-bindgen-cli",
                    "--version",
                    layout::BINDGEN_VERSION,
                    "--locked",
                    "--root",
                ])
                .arg(&root)
                .current_dir(project_root),
        )?;
    }
    resolve().map(drop)
}

/// `false` when upstream has no prebuilt archive for this host.
fn download(cx: &Context, root: &Path) -> Result<bool> {
    // Upstream ships a musl archive for x86-64 Linux; it runs on glibc hosts.
    let host = match env!("FUSOR_HOST") {
        "x86_64-unknown-linux-gnu" => "x86_64-unknown-linux-musl",
        host => host,
    };
    let archive = format!("wasm-bindgen-{}-{host}.tar.gz", layout::BINDGEN_VERSION);
    let releases: BTreeMap<String, String> =
        serde_json::from_str(include_str!("tool-releases.json"))?;
    let Some(checksum) = releases.get(&archive) else {
        return Ok(false);
    };
    let url = format!(
        "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/{}/{archive}",
        layout::BINDGEN_VERSION
    );
    cx.reporter.step(format!(
        "Downloading wasm-bindgen {} for {host} ...",
        layout::BINDGEN_VERSION
    ));
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .build()
        .get(&url)
        .call()
        .map_err(|error| {
            Error::tooling(format!("could not download {url}: {error}"))
                .remedy("retry `fusor install`; no project files were changed")
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(DOWNLOAD_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > DOWNLOAD_LIMIT {
        return Err(Error::tooling(
            "the wasm-bindgen archive exceeds the download limit",
        ));
    }
    if format!("{:x}", Sha256::digest(&bytes)) != *checksum {
        return Err(Error::tooling(
            "the downloaded wasm-bindgen archive does not match its pinned checksum; it was not executed or cached",
        ));
    }
    extract(root, &bytes)?;
    Ok(true)
}

/// The binary must run before it is renamed into the cache.
fn extract(root: &Path, bytes: &[u8]) -> Result {
    fs::create_dir_all(root)?;
    let staging = root.join(format!(
        "{}{}",
        layout::INSTALL_PREFIX,
        crate::pipeline::publish::generation()?
    ));
    fs::create_dir(&staging)?;
    let _cleanup = Staging(staging.clone());
    let name = executable_name();
    let binary = staging.join(&name);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut found = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry
            .path()?
            .file_name()
            .is_none_or(|file| file != name.as_str())
        {
            continue;
        }
        if found || !entry.header().entry_type().is_file() || entry.size() > ENTRY_LIMIT {
            return Err(Error::tooling(
                "the wasm-bindgen archive has an unexpected entry",
            ));
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&binary)?;
        std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
        }
        found = true;
    }
    if !found {
        return Err(Error::tooling(
            "the verified wasm-bindgen archive contains no wasm-bindgen binary",
        ));
    }
    if !reports_expected_version(&binary)? {
        return Err(Error::tooling(
            "the downloaded wasm-bindgen cannot run on this host",
        ));
    }
    let bin = root.join("bin");
    fs::create_dir_all(&bin)?;
    // Another installation may win this race. Its binary was validated the
    // same way, so a matching one is usable; anything else is a real failure.
    if let Err(error) = fs::rename(&binary, bin.join(&name)) {
        if !reports_expected_version(&bin.join(&name)).unwrap_or(false) {
            return Err(error.into());
        }
    }
    Ok(())
}

fn reports_expected_version(binary: &Path) -> Result<bool> {
    let Ok(output) = Command::new(binary).arg("--version").output() else {
        return Ok(false);
    };
    Ok(output.status.success()
        && String::from_utf8_lossy(&output.stdout).trim() == expected_version())
}
