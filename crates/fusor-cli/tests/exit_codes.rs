//! Exit codes, which scripts and CI branch on.
//!
//! The contract is documented on the CLI reference page in `apps/docs`.
mod support;

use support::{Fixture, exit_code};

const USAGE: i32 = 2;
const PROJECT: i32 = 3;

#[test]
fn a_bad_flag_is_a_usage_error() {
    let fixture = Fixture::new();
    assert_eq!(
        exit_code(&mut fixture.cli(&fixture.root, &["build", "--no-such-flag"])),
        USAGE
    );
}

#[test]
fn invalid_proxy_rules_are_rejected_before_project_discovery() {
    let fixture = Fixture::new();
    for rule in [
        "/api",
        "api=http://localhost:8080",
        "/api?x=1=http://localhost:8080",
        "/api/../private=http://localhost:8080",
        "/__fusor=http://localhost:8080",
        "/api=https://localhost:8080",
        "/api=http://user:password@localhost:8080",
        "/api=http://localhost:8080/path",
        "/api=http://localhost:8080?query=1",
        "/api=http://localhost:bad",
        "/api=http://localhost:0",
        "/api=http://:8080",
    ] {
        let output = fixture
            .cli(&fixture.root, &["dev", "--proxy", rule])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(USAGE), "{rule}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("invalid value"), "{diagnostic}");
        assert!(
            diagnostic.contains("--proxy /api=http://127.0.0.1:8080"),
            "{diagnostic}"
        );
    }
}

#[test]
fn an_unknown_package_is_a_usage_error() {
    let fixture = Fixture::new();
    fixture.scaffold("one");
    assert_eq!(
        exit_code(&mut fixture.cli(&fixture.root.join("one"), &["check", "-p", "absent"])),
        USAGE
    );
}

#[test]
fn a_missing_lockfile_is_a_project_error() {
    let fixture = Fixture::new();
    let application = fixture.scaffold("unprepared");
    assert_eq!(
        exit_code(&mut fixture.cli(&application, &["check", "--frozen"])),
        PROJECT
    );
}

#[test]
fn help_and_version_succeed() {
    let fixture = Fixture::new();
    for args in [["--help"], ["--version"]] {
        assert_eq!(exit_code(&mut fixture.cli(&fixture.root, &args)), 0);
    }
}

#[test]
#[cfg(unix)]
fn a_closed_stdout_pipe_fails_with_a_project_error() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::Stdio,
    };
    let fixture = Fixture::new();
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let output = fixture
        .cli(
            &fixture.root,
            &["new", "closed-pipe", "--skip-install", "--framework-path"],
        )
        .arg(&fixture.framework)
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(PROJECT));
    assert!(String::from_utf8_lossy(&output.stderr).contains("writing command output"));
}
