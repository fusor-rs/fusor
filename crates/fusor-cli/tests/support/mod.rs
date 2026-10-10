//! A temporary application outside this workspace, and the CLI that operates
//! on it.
//!
//! These tests exercise the CLI the way a user does: as a subprocess, against
//! a real Cargo project.
#![expect(
    dead_code,
    reason = "Integration suites share this fixture but exercise different subprocess operations"
)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU32, Ordering},
};

pub struct Fixture {
    /// A scratch directory holding every application this test creates.
    pub root: PathBuf,
    /// This framework checkout, which scaffolded applications depend on.
    pub framework: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        // Tests in one binary run in parallel, so the process id alone is not
        // unique enough.
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "fusor-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("scratch directory");
        Self {
            root,
            framework: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("workspace root")
                .to_owned(),
        }
    }

    /// The CLI under test, pinned offline and pointed at a shared target
    /// directory so repeated runs do not recompile the world.
    pub fn cli(&self, directory: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fusor"));
        command
            .args(args)
            .current_dir(directory)
            .env("CARGO_NET_OFFLINE", "true")
            .env(
                "RUSTUP_TOOLCHAIN",
                std::env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| "stable".into()),
            )
            .env(
                "CARGO_TARGET_DIR",
                self.framework.join("target/cli-consumer-tests"),
            );
        let tool = self
            .framework
            .join("target/tools/bin")
            .join(format!("wasm-bindgen{}", std::env::consts::EXE_SUFFIX));
        if std::env::var_os("FUSOR_WASM_BINDGEN").is_none() && tool.is_file() {
            command.env("FUSOR_WASM_BINDGEN", tool);
        }
        command
    }

    /// A new application, sources only. Preparation is the part under test in
    /// the suites that want it.
    pub fn scaffold(&self, name: &str) -> PathBuf {
        success(
            self.cli(
                &self.root,
                &["new", name, "--skip-install", "--framework-path"],
            )
            .arg(&self.framework),
        );
        self.root.join(name)
    }

    /// Resolve a lockfile with Cargo directly, so a test can start from a
    /// locked project without depending on `fusor install`.
    pub fn lock(&self, application: &Path) {
        success(
            Command::new("cargo")
                .args(["generate-lockfile", "--offline"])
                .current_dir(application)
                .env("RUSTUP_TOOLCHAIN", "stable"),
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn success(command: &mut Command) -> Output {
    let output = command.output().expect("the CLI runs");
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Run a command expected to fail, returning everything it printed.
pub fn failure(command: &mut Command) -> String {
    let output = command.output().expect("the CLI runs");
    assert!(
        !output.status.success(),
        "{command:?} unexpectedly succeeded"
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Exit codes are part of the CLI's contract.
pub fn exit_code(command: &mut Command) -> i32 {
    command
        .output()
        .expect("the CLI runs")
        .status
        .code()
        .expect("the CLI exits normally")
}
