use crate::{
    context::Context,
    error::{Error, Result},
    process::{cargo, checked},
};
use std::{env, path::Path, process::Command};

pub(crate) fn check(cx: &Context) -> Result {
    if cx.package.is_some() || !cx.features.is_empty() || cx.manifest_path.is_some() {
        return Err(
            Error::usage("`repo check` verifies the whole framework workspace")
                .remedy("drop the application selection flags"),
        );
    }
    let output = cargo()
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()?;
    if !output.status.success() {
        return Err(Error::usage(
            "`repo check` must run inside the Fusor framework workspace",
        ));
    }
    let manifest = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let root = manifest
        .parent()
        .ok_or_else(|| Error::internal("workspace manifest has no parent directory"))?;
    if !root.join("crates/fusor-cli/Cargo.toml").is_file()
        || !root.join("crates/fusor-build/Cargo.toml").is_file()
        || !root.join("tests/fixtures/consumer/Cargo.toml").is_file()
    {
        return Err(Error::usage("this is not the Fusor framework repository")
            .remedy("use `fusor check` for an application"));
    }
    // The commands below use workspace-relative paths.
    env::set_current_dir(root)?;
    check_workspace(cx, root)
}

/// Workspace builds unify features, which hides a missing dependency until a
/// standalone consumer hits it.
const ISOLATED_STD_FEATURES: &[&str] = &[
    "",
    "forms",
    "actions",
    "forms,actions",
    "resources",
    "query",
    "routing",
    "browser",
    "forms,browser",
    "actions,browser",
    "query,browser",
    "routing,browser",
];

/// Built outside the workspace, to prove the crates work on their own.
const CONSUMER_FIXTURES: [(&str, &str); 2] = [
    (
        "tests/fixtures/consumer/Cargo.toml",
        "target/consumer-tests",
    ),
    (
        "tests/fixtures/application/Cargo.toml",
        "target/application-tests",
    ),
];

fn check_workspace(cx: &Context, root: &Path) -> Result {
    let run = |command: &mut Command| {
        if cx.offline {
            command.env("CARGO_NET_OFFLINE", "true");
        }
        command.env("CARGO_TERM_COLOR", cx.color.as_str());
        if cx.verbose {
            command.env("CARGO_TERM_VERBOSE", "true");
        }
        if cx.quiet {
            command.env("CARGO_TERM_QUIET", "true");
        }
        checked(command)
    };
    run(cargo().args(["fmt", "--all", "--check"]))?;
    run(cargo().args(["test", "--workspace", "--locked"]))?;
    check_feature_isolation(&run)?;
    run(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ]))?;
    check_consumers(&run, root)?;
    check_documentation(&run)
}

fn check_feature_isolation(run: &impl Fn(&mut Command) -> Result) -> Result {
    for features in ISOLATED_STD_FEATURES {
        run(cargo().args([
            "check",
            "-p",
            "fusor-std",
            "--no-default-features",
            "--features",
            features,
            "--locked",
        ]))?;
    }
    run(cargo().args([
        "test",
        "-p",
        "fusor-std",
        "--features",
        "forms,actions",
        "--locked",
    ]))
}

fn check_consumers(run: &impl Fn(&mut Command) -> Result, root: &Path) -> Result {
    for (manifest, target) in CONSUMER_FIXTURES {
        run(cargo()
            .args(["check", "--locked", "--manifest-path"])
            .arg(root.join(manifest))
            .arg("--target-dir")
            .arg(root.join(target)))
        .map_err(|error| {
            error.remedy("if its lockfile is stale, run `just lock` and commit the result")
        })?;
    }
    Ok(())
}

fn check_documentation(run: &impl Fn(&mut Command) -> Result) -> Result {
    let flags = format!(
        "{} -D warnings",
        env::var("RUSTDOCFLAGS").unwrap_or_default()
    );
    run(cargo()
        .args([
            "doc",
            "--workspace",
            "--all-features",
            "--no-deps",
            "--locked",
        ])
        .env("RUSTDOCFLAGS", flags))
}
