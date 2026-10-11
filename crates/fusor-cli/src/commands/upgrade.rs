//! Replaces this executable with the latest stable release, the way it was
//! installed: through Cargo when Cargo tracks it, otherwise with the release's
//! own installer, which downloads, verifies and places the binary.
use crate::{
    context::Context,
    error::{Error, Result},
    process::{cargo, checked},
    toolchain,
    transaction::Staging,
};
use semver::Version;
use serde::Deserialize;
use std::{
    cmp::Ordering,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// A directory laid out like a release (`latest.json`, then `<tag>/install.sh`
/// and the archives), so the upgrade can be tested without GitHub.
const RELEASE_BASE: &str = "FUSOR_RELEASE_BASE";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

enum Installer {
    Cargo,
    Script,
}

/// Both installers and `cargo install` place the binary in `<root>/bin`.
struct Installation {
    root: PathBuf,
    installer: Installer,
}

pub(crate) fn run(cx: &Context) -> Result {
    cx.reporter.banner("upgrade");
    if cx.offline {
        return Err(
            Error::usage("upgrading downloads a release, which --offline forbids")
                .remedy("run `fusor upgrade` without --offline"),
        );
    }
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let executable = fs::canonicalize(env::current_exe()?)?;
    let installation = installation(&executable)?;
    cx.reporter.step("Checking for a newer release ...");
    let release = latest_release()?;
    let version = stable_version(&release)?;
    if version.cmp_precedence(&current) != Ordering::Greater {
        cx.reporter
            .done(format!("fusor {current} is the latest release"));
        return Ok(());
    }
    cx.reporter
        .step(format!("Upgrading fusor {current} to {version} ..."));
    let moved = move_aside(&executable)?;
    let installed = match installation.installer {
        Installer::Cargo => install_with_cargo(&installation.root, &version),
        Installer::Script => install_with_script(&installation.root, &release, &version),
    };
    if let Err(error) = installed {
        if let Some(moved) = moved {
            fs::rename(moved, &executable)?;
        }
        return Err(error);
    }
    verify(&executable, &version)?;
    cx.reporter
        .done(format!("Upgraded fusor {current} to {version}"));
    Ok(())
}

fn installation(executable: &Path) -> Result<Installation> {
    let root = executable
        .parent()
        .filter(|directory| directory.ends_with("bin"))
        .and_then(Path::parent)
        .ok_or_else(|| {
            Error::tooling(format!(
                "{} was not installed by the fusor installer or `cargo install`",
                executable.display()
            ))
            .remedy("install fusor from https://fusor.build, then run `fusor upgrade`")
        })?;
    let installer = if tracked_by_cargo(root, executable)? {
        Installer::Cargo
    } else {
        Installer::Script
    };
    Ok(Installation {
        root: root.to_owned(),
        installer,
    })
}

/// `cargo install --list` prints each package, then its binaries indented.
fn tracked_by_cargo(root: &Path, executable: &Path) -> Result<bool> {
    if !root.join(".crates.toml").try_exists()? {
        return Ok(false);
    }
    // Outside any project, so its rust-toolchain file cannot select Cargo.
    let output = cargo()
        .args(["install", "--list", "--root"])
        .arg(root)
        .current_dir(root)
        .output()
        .map_err(|error| {
            Error::tooling(format!(
                "could not run Cargo to inspect {}: {error}",
                root.display()
            ))
            .remedy("install Rust from https://rustup.rs, then run `fusor upgrade`")
        })?;
    if !output.status.success() {
        return Err(Error::tooling(format!(
            "Cargo could not list the packages installed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let package = format!("{} v", env!("CARGO_PKG_NAME"));
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut lines = listing.lines();
    if !lines.any(|line| line.starts_with(&package)) {
        return Ok(false);
    }
    Ok(lines
        .take_while(|line| line.starts_with(' '))
        .any(|binary| executable.file_name() == Some(binary.trim().as_ref())))
}

/// Unset outside tests. A value that is not Unicode cannot form a URL, so it
/// counts as unset too.
fn release_base() -> Option<String> {
    env::var(RELEASE_BASE).ok()
}

fn latest_release() -> Result<Release> {
    let url = match release_base() {
        Some(base) => format!("{base}/latest.json"),
        None => format!(
            "https://api.github.com/repos/{}/releases/latest",
            repository()
        ),
    };
    serde_json::from_str(&get(&url)?).map_err(|error| {
        Error::tooling(format!("{url} returned an unreadable release: {error}"))
            .remedy("try `fusor upgrade` again later")
    })
}

fn get(url: &str) -> Result<String> {
    toolchain::download::agent()
        .get(url)
        .call()
        .map_err(|error| {
            Error::tooling(format!("could not download {url}: {error}"))
                .remedy("check your connection, then run `fusor upgrade` again")
        })?
        .into_string()
        .map_err(Error::from)
}

fn stable_version(release: &Release) -> Result<Version> {
    let version = Version::parse(release.tag_name.trim_start_matches('v')).map_err(|error| {
        Error::tooling(format!(
            "the latest release tag {:?} is not a version: {error}",
            release.tag_name
        ))
    })?;
    if release.draft || release.prerelease || !version.pre.is_empty() {
        return Err(Error::tooling(format!(
            "the latest release, {}, is not a stable release",
            release.tag_name
        ))
        .remedy("try `fusor upgrade` again once it is published as stable"));
    }
    Ok(version)
}

fn repository() -> &'static str {
    env!("CARGO_PKG_REPOSITORY")
        .strip_prefix("https://github.com/")
        .expect("the package repository is hosted on GitHub")
}

/// crates.io receives a release shortly after GitHub does.
fn install_with_cargo(root: &Path, version: &Version) -> Result {
    checked(
        cargo()
            .args(["install", env!("CARGO_PKG_NAME"), "--locked", "--version"])
            .arg(format!("={version}"))
            .arg("--root")
            .arg(root)
            .current_dir(root),
    )
    .map_err(|error| {
        error.remedy("if the release was published moments ago, run `fusor upgrade` again later")
    })
}

/// The installer comes from the release's own tag, so it always matches the
/// archives it installs.
fn install_with_script(root: &Path, release: &Release, version: &Version) -> Result {
    require_archive(release, version)?;
    let base = release_base();
    let script = if cfg!(windows) {
        "install.ps1"
    } else {
        "install.sh"
    };
    let tag = &release.tag_name;
    let url = match &base {
        Some(base) => format!("{base}/{tag}/{script}"),
        None => format!(
            "https://raw.githubusercontent.com/{}/{tag}/{script}",
            repository()
        ),
    };
    let staging = env::temp_dir().join(format!(
        "fusor-upgrade-{}",
        crate::pipeline::publish::generation()?
    ));
    fs::create_dir(&staging)?;
    let _cleanup = Staging(staging.clone());
    let installer = staging.join(script);
    fs::write(&installer, get(&url)?)?;
    let mut command = shell();
    command
        .arg(&installer)
        .env("FUSOR_VERSION", tag)
        .env("FUSOR_INSTALL", root);
    if let Some(base) = base {
        command.env("FUSOR_DOWNLOAD_BASE", base);
    }
    checked(&mut command)
}

/// The release is published before its archives are attached.
fn require_archive(release: &Release, version: &Version) -> Result {
    let archive = archive_name(version)?;
    for required in [archive.clone(), format!("{archive}.sha256")] {
        if !release.assets.iter().any(|asset| asset.name == required) {
            return Err(Error::tooling(format!(
                "fusor {version} is still being published; {required} is not attached yet"
            ))
            .remedy("try `fusor upgrade` again in a few minutes"));
        }
    }
    Ok(())
}

/// Windows PowerShell ships with every Windows release, and refuses script
/// files under its default execution policy.
fn shell() -> Command {
    if cfg!(windows) {
        let mut command = Command::new("powershell");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]);
        command
    } else {
        Command::new("sh")
    }
}

/// Named as the release workflow packages them. Windows on Arm runs the x64
/// build, as `install.ps1` installs it.
fn archive_name(version: &Version) -> Result<String> {
    let target = match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("windows", "x86_64" | "aarch64") => "x86_64-pc-windows-msvc",
        (os, architecture) => {
            return Err(Error::tooling(format!(
                "fusor publishes no prebuilt release for {os} on {architecture}"
            ))
            .remedy(format!(
                "upgrade with `cargo install {} --locked`",
                env!("CARGO_PKG_NAME")
            )));
        }
    };
    let extension = if cfg!(windows) { "zip" } else { "tar.gz" };
    Ok(format!("fusor-{version}-{target}.{extension}"))
}

/// Windows cannot replace a running executable, but it can rename one. The
/// renamed copy stays until the next upgrade, because it is still running.
#[cfg(windows)]
fn move_aside(executable: &Path) -> Result<Option<PathBuf>> {
    let moved = executable.with_extension("exe.old");
    match fs::remove_file(&moved) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        removed => removed.map_err(|error| {
            Error::tooling(format!(
                "could not remove {} left by the last upgrade: {error}",
                moved.display()
            ))
            .remedy("close every running fusor, then run `fusor upgrade` again")
        })?,
    }
    fs::rename(executable, &moved)?;
    Ok(Some(moved))
}

/// A Unix rename replaces the file while a running process keeps its copy.
#[cfg(not(windows))]
fn move_aside(_executable: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}

fn verify(executable: &Path, version: &Version) -> Result {
    let output = Command::new(executable).arg("--version").output()?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || reported.trim() != format!("fusor {version}") {
        return Err(Error::tooling(format!(
            "the upgraded {} reports {:?}, not fusor {version}",
            executable.display(),
            reported.trim()
        ))
        .remedy("reinstall fusor from https://fusor.build"));
    }
    Ok(())
}
