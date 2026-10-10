//! Exercises a built CLI against a fresh application. Preview runs with an
//! empty `PATH` to prove serving needs neither Cargo nor Rust. This uses
//! `--framework-path`, so it does not test a registry installation.
use crate::{Result, remove_directory, workspace_root};
use std::{
    fs,
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// Proves the served page is the starter, not an error document.
const STARTER_TEXT: &str = "Rust, inside HTML.";

const READY_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) fn run(binary: &Path) -> Result {
    let binary = binary.canonicalize()?;
    let root = scratch_directory()?;
    let result = exercise(&binary, &root);
    finish_cleanup(result, remove_directory(&root))?;
    println!("new, frozen check, frozen build and artifact-only preview all passed");
    Ok(())
}

fn exercise(binary: &Path, root: &Path) -> Result {
    let application = root.join("app");
    let environment = [
        ("FUSOR_CACHE_DIR", root.join("cache")),
        ("CARGO_TARGET_DIR", root.join("target")),
    ];
    let cli = |directory: &Path, args: &[&str]| -> Result {
        let mut command = Command::new(binary);
        command.args(args).current_dir(directory);
        command.envs(environment.iter().map(|(key, value)| (key, value)));
        if !command.status()?.success() {
            return Err(format!("fusor {} failed", args.join(" ")).into());
        }
        Ok(())
    };

    cli(
        root,
        &[
            "new",
            "app",
            "--yes",
            "--framework-path",
            &workspace_root().to_string_lossy(),
        ],
    )?;
    cli(&application, &["check", "--frozen"])?;
    cli(&application, &["build", "--frozen"])?;

    preview(binary, root, &application, &environment)
}

fn preview(
    binary: &Path,
    root: &Path,
    application: &Path,
    environment: &[(&str, PathBuf)],
) -> Result {
    let port = free_port()?;
    let mut preview = Command::new(binary);
    preview
        .args(["preview"])
        .arg(application.join("dist"))
        .args(["--port", &port.to_string()])
        .current_dir(root)
        .env_clear()
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .env("PATH", "")
        .stdout(Stdio::null());
    // Windows cannot initialize sockets without SystemRoot.
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        preview.env("SystemRoot", system_root);
    }
    let mut preview = preview.spawn()?;
    let served = wait_for_page(&mut preview, port);
    let stopped = match preview.try_wait() {
        Ok(Some(_)) => Ok(()),
        Ok(None) => preview.kill().map_err(Into::into),
        Err(error) => finish_cleanup(Err(error.into()), preview.kill().map_err(Into::into)),
    };
    let waited = preview.wait().map(|_| ()).map_err(Into::into);
    finish_cleanup(served, finish_cleanup(stopped, waited))
}

fn finish_cleanup(result: Result, cleanup: Result) -> Result {
    match (result, cleanup) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup failed: {cleanup}").into()),
    }
}

fn wait_for_page(preview: &mut Child, port: u16) -> Result {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = preview.try_wait()? {
            return Err(format!("the preview server exited early with {status}").into());
        }
        let response = ureq::get(&format!("http://127.0.0.1:{port}/"))
            .timeout(Duration::from_secs(1))
            .call();
        if let Ok(response) = response {
            let body = response.into_string()?;
            if body.contains(STARTER_TEXT) {
                return Ok(());
            }
            return Err(format!("the served page does not contain {STARTER_TEXT:?}").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("the preview server did not start within the timeout".into())
}

/// Racy between release and bind; the readiness loop turns a lost race into
/// a clear failure.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

fn scratch_directory() -> Result<PathBuf> {
    let root = std::env::temp_dir().join(format!("fusor-release-smoke-{}", std::process::id()));
    remove_directory(&root)?;
    fs::create_dir_all(&root)?;
    Ok(root)
}
