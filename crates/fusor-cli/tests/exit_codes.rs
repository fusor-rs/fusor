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
