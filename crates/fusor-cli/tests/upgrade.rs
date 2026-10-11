//! `fusor upgrade`, run from an installed copy against a local server laid out
//! like a GitHub release.
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const FUTURE: &str = "999.0.0";
const TOOLING: i32 = 4;

type Files = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;

struct Release {
    base: String,
    files: Files,
    root: PathBuf,
}

impl Release {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "fusor-upgrade-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("installation/bin")).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fusor"), executable(&root)).unwrap();
        let files = Files::default();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = files.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut request = String::new();
                BufReader::new(&stream).read_line(&mut request).unwrap();
                let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
                let body = served.lock().unwrap().get(&path).cloned();
                let (status, body) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                // A client that stops reading early is not this server's failure.
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        Self { base, files, root }
    }

    fn serve(&self, path: &str, body: impl Into<Vec<u8>>) {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_owned(), body.into());
    }

    fn latest(&self, release: serde_json::Value) {
        self.serve("/latest.json", release.to_string());
    }

    fn upgrade(&self) -> Command {
        let mut command = Command::new(executable(&self.root));
        command
            .arg("upgrade")
            .env("FUSOR_RELEASE_BASE", &self.base)
            .env("CARGO_HOME", self.root.join("cargo home"))
            .env_remove("CARGO_NET_OFFLINE");
        command
    }

    fn installed_version(&self) -> String {
        let output = Command::new(executable(&self.root))
            .arg("--version")
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        // Windows may still hold the replaced executable briefly.
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn executable(root: &Path) -> PathBuf {
    root.join("installation/bin")
        .join(format!("fusor{}", std::env::consts::EXE_SUFFIX))
}

fn release(tag: &str, assets: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag,
        "draft": false,
        "prerelease": false,
        "assets": assets.iter().map(|name| serde_json::json!({ "name": name })).collect::<Vec<_>>(),
    })
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn an_executable_outside_an_installation_is_not_replaced() {
    let output = Command::new(env!("CARGO_BIN_EXE_fusor"))
        .arg("upgrade")
        .env("FUSOR_RELEASE_BASE", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(TOOLING));
    assert!(
        stderr(&output).contains("was not installed by the fusor installer or `cargo install`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_current_or_an_older_release_is_up_to_date() {
    let server = Release::new();
    for tag in [
        format!("v{CURRENT}"),
        "v0.0.1".to_owned(),
        format!("v{CURRENT}+build"),
    ] {
        server.latest(release(&tag, &[]));
        let output = server.upgrade().output().unwrap();
        assert!(output.status.success(), "{tag}: {}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!("fusor {CURRENT} is the latest release")),
            "{tag}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn unstable_unfinished_or_unreadable_releases_leave_the_installation_alone() {
    let server = Release::new();
    let original = fs::read(executable(&server.root)).unwrap();
    let future = format!("v{FUTURE}");
    let mut draft = release(&future, &[]);
    draft["draft"] = true.into();
    let mut prerelease = release(&future, &[]);
    prerelease["prerelease"] = true.into();
    for (latest, message) in [
        (Some(draft.to_string()), "is not a stable release"),
        (Some(prerelease.to_string()), "is not a stable release"),
        (
            Some(release(&format!("v{FUTURE}-rc.1"), &[]).to_string()),
            "is not a stable release",
        ),
        (
            Some(release("nightly", &[]).to_string()),
            "is not a version",
        ),
        (
            Some(release(&future, &[]).to_string()),
            "is still being published",
        ),
        (Some("not json".to_owned()), "unreadable release"),
        (None, "could not download"),
    ] {
        match &latest {
            Some(body) => server.serve("/latest.json", body.clone()),
            None => {
                server.files.lock().unwrap().remove("/latest.json");
            }
        }
        let output = server.upgrade().output().unwrap();
        assert_eq!(output.status.code(), Some(TOOLING), "{latest:?}");
        assert!(
            stderr(&output).contains(message),
            "{latest:?}: {}",
            stderr(&output)
        );
        assert_eq!(fs::read(executable(&server.root)).unwrap(), original);
    }
}

/// Cargo is offline with an empty home, so it fails; reaching it proves the
/// upgrade chose Cargo, and the executable must still be in place.
#[test]
fn a_cargo_installation_upgrades_through_cargo() {
    let server = Release::new();
    let cargo_home = server.root.join("cargo home");
    fs::create_dir(&cargo_home).unwrap();
    fs::write(cargo_home.join("config.toml"), "[net]\noffline = true\n").unwrap();
    fs::write(
        server.root.join("installation/.crates.toml"),
        format!(
            "[v1]\n\"fusor-cli {CURRENT} (registry+https://github.com/rust-lang/crates.io-index)\" = [\"fusor{}\"]\n",
            std::env::consts::EXE_SUFFIX
        ),
    )
    .unwrap();
    server.latest(release(&format!("v{FUTURE}"), &[]));
    let output = server.upgrade().output().unwrap();
    assert_eq!(output.status.code(), Some(TOOLING));
    assert!(
        stderr(&output).contains("failed with exit status"),
        "{}",
        stderr(&output)
    );
    assert!(!stderr(&output).contains("still being published"));
    assert_eq!(server.installed_version(), format!("fusor {CURRENT}"));
}

#[cfg(unix)]
mod installer {
    use super::*;
    use sha2::{Digest, Sha256};

    fn target() -> &'static str {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "aarch64-apple-darwin",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("linux", "aarch64") => "aarch64-unknown-linux-musl",
            ("linux", "x86_64") => "x86_64-unknown-linux-musl",
            host => panic!("no release target for {host:?}"),
        }
    }

    /// A release whose `fusor` is a script reporting `reported`.
    fn publish(server: &Release, reported: &str) {
        let name = format!("fusor-{FUTURE}-{}", target());
        let archive = format!("{name}.tar.gz");
        let script = format!("#!/bin/sh\necho 'fusor {reported}'\n");
        let mut header = tar::Header::new_gnu();
        header.set_size(script.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        builder
            .append_data(&mut header, format!("{name}/fusor"), script.as_bytes())
            .unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        let checksum = format!("{:x}  {archive}\n", Sha256::digest(&bytes));
        let tag = format!("v{FUTURE}");
        server.serve(
            &format!("/{tag}/install.sh"),
            fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../install.sh")).unwrap(),
        );
        server.serve(&format!("/{tag}/{archive}"), bytes);
        server.serve(&format!("/{tag}/{archive}.sha256"), checksum);
        server.latest(release(&tag, &[&archive, &format!("{archive}.sha256")]));
    }

    #[test]
    fn an_installer_installation_upgrades_with_the_release_installer() {
        let server = Release::new();
        publish(&server, FUTURE);
        let output = server.upgrade().output().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stderr(&output).contains(&format!("Upgraded fusor {CURRENT} to {FUTURE}")),
            "{}",
            stderr(&output)
        );
        assert_eq!(server.installed_version(), format!("fusor {FUTURE}"));
    }

    #[test]
    fn an_installed_binary_reporting_another_version_fails_the_upgrade() {
        let server = Release::new();
        publish(&server, "1.0.0");
        let output = server.upgrade().output().unwrap();
        assert_eq!(output.status.code(), Some(TOOLING));
        assert!(
            stderr(&output).contains(&format!("reports \"fusor 1.0.0\", not fusor {FUTURE}")),
            "{}",
            stderr(&output)
        );
    }
}
