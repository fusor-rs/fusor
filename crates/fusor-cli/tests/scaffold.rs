//! Creating an application, and what the read-only commands do to one that has
//! not been prepared yet.
mod support;

use std::fs;
use support::{Fixture, failure, success};

#[test]
fn scaffolding_and_read_only_commands_run_without_rust() {
    let fixture = Fixture::new();
    let help = success(fixture.cli(&fixture.root, &[]).env("PATH", ""));
    assert!(String::from_utf8_lossy(&help.stdout).contains("fusor new my-app"));

    success(
        fixture
            .cli(
                &fixture.root,
                &["new", "starter", "--skip-install", "--framework-path"],
            )
            .arg(&fixture.framework)
            .env("PATH", ""),
    );
    let application = fixture.root.join("starter");
    let before = fs::read(application.join("Cargo.toml")).unwrap();
    // Doctor reports problems; it never resolves a lockfile or edits anything.
    let error = failure(fixture.cli(&application, &["doctor"]).env("PATH", ""));
    assert!(error.contains("Cargo.lock is missing"), "{error}");
    assert_eq!(before, fs::read(application.join("Cargo.toml")).unwrap());
    assert!(!application.join("Cargo.lock").exists());
}

#[test]
fn doctor_reports_broken_ancestor_manifests_without_rust() {
    let fixture = Fixture::new();
    let malformed = fixture.root.join("malformed");
    let unreadable = fixture.root.join("unreadable");
    fs::create_dir(&malformed).unwrap();
    fs::write(malformed.join("Cargo.toml"), "[broken").unwrap();
    fs::create_dir_all(unreadable.join("Cargo.toml")).unwrap();
    for root in [malformed, unreadable] {
        let application = root.join("app");
        fs::create_dir(&application).unwrap();
        fs::write(
            application.join("Cargo.toml"),
            "[package]\nname = 'app'\n[package.metadata.fusor]\n",
        )
        .unwrap();
        let error = failure(fixture.cli(&application, &["doctor"]).env("PATH", ""));
        assert!(
            error.contains(&root.join("Cargo.toml").display().to_string()),
            "{error}"
        );
    }
}

#[test]
fn a_frozen_check_refuses_to_resolve_a_missing_lockfile() {
    let fixture = Fixture::new();
    let application = fixture.scaffold("unlocked");
    let error = failure(&mut fixture.cli(&application, &["check", "--frozen"]));
    assert!(error.contains("fusor install"), "{error}");
    assert!(!application.join("Cargo.lock").exists());
}

#[test]
fn new_refuses_an_occupied_destination() {
    let fixture = Fixture::new();
    fixture.scaffold("taken");
    let error = failure(&mut fixture.cli(&fixture.root, &["new", "taken", "--skip-install"]));
    assert!(error.contains("destination already exists"), "{error}");
}
