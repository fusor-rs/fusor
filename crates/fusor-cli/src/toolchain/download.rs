//! Pinned prebuilt tools: a release is verified before anything runs, and a
//! binary enters the cache only after it has run on this host.
use crate::{
    error::{Error, Result},
    layout,
    transaction::Staging,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Between reads, not for the whole transfer, so a large tool on a slow
/// connection still completes.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// `None` when no release is pinned under this name.
pub(super) fn checksum(release: &str) -> Result<Option<String>> {
    let mut releases: BTreeMap<String, String> =
        serde_json::from_str(include_str!("tool-releases.json"))?;
    Ok(releases.remove(release))
}

/// A directory beside the cached tools, removed when the guard drops.
pub(super) fn staging(root: &Path) -> Result<(PathBuf, Staging)> {
    fs::create_dir_all(root)?;
    let staging = root.join(format!(
        "{}{}",
        layout::INSTALL_PREFIX,
        crate::pipeline::publish::generation()?
    ));
    fs::create_dir(&staging)?;
    Ok((staging.clone(), Staging(staging)))
}

/// Streams `url` into `file`, failing past `limit` bytes or on a checksum
/// mismatch.
pub(super) fn fetch(tool: &str, url: &str, checksum: &str, limit: u64, file: &Path) -> Result {
    let response = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| {
            Error::tooling(format!("could not download {url}: {error}"))
                .remedy("retry `fusor install`; no project files were changed")
        })?;
    let mut reader = response.into_reader().take(limit + 1);
    let mut output = fs::File::create_new(file)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut received = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        received += read as u64;
    }
    output.flush()?;
    if received > limit {
        return Err(Error::tooling(format!(
            "the {tool} download exceeds its size limit"
        )));
    }
    if format!("{:x}", hasher.finalize()) != checksum {
        return Err(Error::tooling(format!(
            "the downloaded {tool} does not match its pinned checksum; it was not executed or cached"
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn make_executable(binary: &Path) -> Result {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(binary, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Windows runs a file by its extension.
#[cfg(not(unix))]
pub(super) fn make_executable(_binary: &Path) -> Result {
    Ok(())
}

/// `runs` reports whether a binary is the expected tool and works here.
pub(super) fn install(
    tool: &str,
    binary: &Path,
    installed: &Path,
    runs: impl Fn(&Path) -> bool,
) -> Result {
    if !runs(binary) {
        return Err(Error::tooling(format!(
            "the downloaded {tool} cannot run on this host"
        )));
    }
    if let Some(bin) = installed.parent() {
        fs::create_dir_all(bin)?;
    }
    // Another installation may win this race. Its binary was validated the
    // same way, so a matching one is usable; anything else is a real failure.
    if let Err(error) = fs::rename(binary, installed) {
        if !runs(installed) {
            return Err(error.into());
        }
    }
    Ok(())
}
