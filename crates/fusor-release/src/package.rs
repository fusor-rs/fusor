//! The binary must report the expected version and the archive must unpack
//! before it is announced.
use crate::{Result, archive, remove_directory, workspace_root, workspace_version};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const BINARY: &str = "fusor";

pub(super) fn run(target: &str, binary_directory: Option<&Path>, output: &Path) -> Result {
    if target.is_empty()
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(format!("invalid release target {target:?}; use a Rust target identifier containing only letters, digits, '-' or '_'").into());
    }
    let root = workspace_root();
    let version = workspace_version()?;
    let windows = target.contains("windows");
    let suffix = if windows { ".exe" } else { "" };
    let binaries = binary_directory
        .map(Path::to_owned)
        .unwrap_or_else(|| root.join("target").join(target).join("release"));

    let name = format!("fusor-{version}-{target}");
    fs::create_dir_all(output)?;
    let stage = output.join(format!(".staging-{name}"));
    remove_directory(&stage)?;
    fs::create_dir_all(&stage)?;

    let source = binaries.join(format!("{BINARY}{suffix}"));
    verify_version(&source, &version)?;
    fs::copy(&source, stage.join(format!("{BINARY}{suffix}")))?;
    fs::copy(root.join("LICENSE"), stage.join("LICENSE"))?;
    fs::copy(
        root.join("crates/fusor-cli/README.md"),
        stage.join("README.md"),
    )?;
    fs::write(stage.join("build.json"), build_metadata(&version, target)?)?;

    let archive_path = output.join(format!(
        "{name}{}",
        if windows { ".zip" } else { ".tar.gz" }
    ));
    if windows {
        archive::zip(&stage, &name, &archive_path)?;
        archive::verify_zip(&archive_path, entry_count(&stage)?)?;
    } else {
        archive::tar_gz(&stage, &name, &archive_path)?;
        run_from_archive(&archive_path, &name, suffix)?;
    }
    write_checksum(&archive_path)?;
    fs::remove_dir_all(&stage)?;
    println!("{}", archive_path.display());
    Ok(())
}

fn verify_version(binary: &Path, version: &str) -> Result {
    let reported = version_output(Command::new(binary).arg("--version"))?;
    if reported != format!("fusor {version}") {
        return Err(format!(
            "{}: reports {reported:?}, expected fusor {version}",
            binary.display()
        )
        .into());
    }
    Ok(())
}

fn build_metadata(version: &str, target: &str) -> Result<String> {
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "version": version,
        "target": target,
        "rustc": version_output(Command::new("rustc").arg("--version"))?,
    }))? + "\n")
}

fn version_output(command: &mut Command) -> Result<String> {
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "{} --version failed with {}: {}",
            command.get_program().to_string_lossy(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Also proves the executable bit survived.
fn run_from_archive(archive_path: &Path, name: &str, suffix: &str) -> Result {
    let unpacked = archive_path.with_extension("unpacked");
    remove_directory(&unpacked)?;
    fs::create_dir_all(&unpacked)?;
    let file = fs::File::open(archive_path)?;
    tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(&unpacked)?;
    let executable = unpacked.join(name).join(format!("{BINARY}{suffix}"));
    let status = Command::new(&executable)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(format!("{} --help failed from the archive", executable.display()).into());
    }
    fs::remove_dir_all(&unpacked)?;
    Ok(())
}

fn entry_count(stage: &Path) -> Result<usize> {
    fs::read_dir(stage)?.try_fold(0, |count, entry| {
        entry?;
        Ok(count + 1)
    })
}

/// `sha256sum`-compatible.
fn write_checksum(archive_path: &Path) -> Result<PathBuf> {
    let name = archive_path
        .file_name()
        .ok_or("the archive has no filename")?
        .to_string_lossy();
    let digest = format!("{:x}", Sha256::digest(fs::read(archive_path)?));
    let checksum = archive_path.with_file_name(format!("{name}.sha256"));
    fs::write(&checksum, format!("{digest}  {name}\n"))?;
    Ok(checksum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_target_paths_before_creating_output() {
        let output =
            std::env::temp_dir().join(format!("fusor-invalid-target-{}", std::process::id()));
        remove_directory(&output).unwrap();
        for target in ["", "../release", "/tmp/target", "custom.json", "a\\b"] {
            let error = run(target, None, &output).unwrap_err().to_string();
            assert!(error.contains("invalid release target"), "{error}");
            assert!(!output.exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_version_command_cannot_validate_a_binary() {
        use std::os::unix::fs::PermissionsExt;
        let binary =
            std::env::temp_dir().join(format!("fusor-failed-version-{}", std::process::id()));
        fs::write(
            &binary,
            "#!/bin/sh\nprintf 'fusor test-version\\n'\nexit 7\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let error = verify_version(&binary, "test-version")
            .unwrap_err()
            .to_string();
        assert!(error.contains("failed with exit status: 7"), "{error}");
        fs::remove_file(binary).unwrap();
    }
}
