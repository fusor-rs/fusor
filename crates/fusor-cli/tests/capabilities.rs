//! `fusor add`, which edits dependencies and nothing else.
//!
//! Ignored by default: these compile real applications, so they need the Wasm
//! target and a matching wasm-bindgen. CI runs them explicitly.
mod support;

use std::fs;
use support::{Fixture, failure, success};

/// A snippet that uses the capability's API, so `check` proves the added
/// dependency and features are actually sufficient.
fn sample(capability: &str) -> &'static str {
    match capability {
        "router" => {
            "pub fn route() -> String { let url = fusor_router::AppUrl::parse(\"/issues/42\").unwrap(); let _options = fusor_router::browser::NavigateOptions::default(); url.path().to_owned() }"
        }
        "async" => {
            "pub fn read(owner: &fusor::OwnerHandle) { let _ = fusor_async::browser::read(owner, || 1_u32, |key, _| async move { Ok::<_, String>(key) }); }"
        }
        "query" => {
            "pub fn query(owner: &fusor::OwnerHandle) { let _ = fusor_query::browser::client(owner, fusor_query::QueryOptions { freshness: fusor_query::Freshness::For(std::time::Duration::from_secs(60)), retention: std::time::Duration::from_secs(120), capacity: std::num::NonZeroUsize::new(4).unwrap() }, |key: u32, _: fusor_async::CancellationToken| async move { Ok::<_, String>(key) }); }"
        }
        "forms" => {
            "pub fn field() -> fusor_std::forms::TextField<String> { fusor_std::forms::TextField::new(String::from(\"draft\")) }"
        }
        "actions" => {
            "pub fn action(owner: &fusor::OwnerHandle) { let _ = fusor_std::actions::browser::action(owner, |_: std::rc::Rc<String>, _| async { fusor_std::actions::Outcome::<(), String>::Accepted(()) }); }"
        }
        other => panic!("no sample for {other}"),
    }
}

#[test]
#[ignore = "requires the installed Wasm target and a matching wasm-bindgen"]
fn each_capability_is_idempotent_preserves_comments_and_compiles() {
    let fixture = Fixture::new();
    for capability in ["router", "async", "query", "forms", "actions"] {
        let application = fixture.scaffold(&format!("with-{capability}"));
        fixture.lock(&application);
        let manifest = application.join("Cargo.toml");
        let commented = format!(
            "# Preserve my application comment\n{}",
            fs::read_to_string(&manifest).unwrap()
        );
        fs::write(&manifest, &commented).unwrap();
        let lock = fs::read(application.join("Cargo.lock")).unwrap();

        success(&mut fixture.cli(&application, &["add", capability, "--dry-run", "--offline"]));
        assert_eq!(commented, fs::read_to_string(&manifest).unwrap());
        assert_eq!(lock, fs::read(application.join("Cargo.lock")).unwrap());

        success(&mut fixture.cli(&application, &["add", capability, "--offline"]));
        let once = fs::read(&manifest).unwrap();
        success(&mut fixture.cli(&application, &["add", capability, "--offline"]));
        assert_eq!(once, fs::read(&manifest).unwrap(), "add must be idempotent");
        assert!(String::from_utf8_lossy(&once).starts_with("# Preserve my application comment"));

        fs::write(application.join("src/capability.rs"), sample(capability)).unwrap();
        fs::write(
            application.join("src/lib.rs"),
            "mod app;\nmod counter;\npub mod capability;\n",
        )
        .unwrap();
        success(&mut fixture.cli(&application, &["check", "--frozen"]));
    }
}

#[test]
#[ignore = "requires the installed Wasm target and a matching wasm-bindgen"]
fn a_workspace_addition_touches_only_the_selected_member() {
    let fixture = Fixture::new();
    let workspace = fixture.root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let mut members = Vec::new();
    for member in ["one", "two"] {
        let application = fixture.scaffold(member);
        let destination = workspace.join(member);
        fs::rename(&application, &destination).unwrap();
        let manifest = destination.join("Cargo.toml");
        let source = fs::read_to_string(&manifest)
            .unwrap()
            .replace("[workspace]\n", "")
            .lines()
            .map(|line| {
                if line.starts_with("fusor-core = ") {
                    "fusor-core = { workspace = true }"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&manifest, source).unwrap();
        members.push(manifest);
    }
    let workspace_source = format!(
        "[workspace]\nmembers = [\"one\", \"two\"]\nresolver = \"2\"\n[workspace.dependencies]\nfusor-core = {{ path = {:?}, features = [\"dom\"] }}\nfusor-std = {{ path = {:?} }}\n",
        fixture.framework.join("crates/fusor-core"),
        fixture.framework.join("crates/fusor-std")
    );
    fs::write(workspace.join("Cargo.toml"), &workspace_source).unwrap();
    fixture.lock(&workspace);

    let sibling = fs::read(&members[1]).unwrap();
    success(&mut fixture.cli(&workspace, &["add", "forms", "-p", "one", "--offline"]));
    assert_eq!(
        workspace_source,
        fs::read_to_string(workspace.join("Cargo.toml")).unwrap(),
        "shared definitions are not edited"
    );
    assert_eq!(
        sibling,
        fs::read(&members[1]).unwrap(),
        "siblings are not edited"
    );
    success(&mut fixture.cli(&workspace, &["check", "-p", "one", "--frozen"]));
}

#[test]
#[ignore = "requires the installed Wasm target and a matching wasm-bindgen"]
fn a_failure_after_cargos_edit_restores_every_file_the_operation_wrote() {
    let fixture = Fixture::new();
    let application = fixture.scaffold("failed-add");
    fixture.lock(&application);
    // An incompatible esbuild pin makes the JavaScript step fail after Cargo
    // has already edited the manifest and lock.
    fs::write(
        application.join("package.json"),
        "{\"private\":true,\"devDependencies\":{\"esbuild\":\"0.0.0\"}}\n",
    )
    .unwrap();
    let manifest = fs::read(application.join("Cargo.toml")).unwrap();
    let lock = fs::read(application.join("Cargo.lock")).unwrap();
    let npm = fs::read(application.join("package.json")).unwrap();

    let error = failure(&mut fixture.cli(&application, &["add", "javascript", "--offline"]));
    assert!(error.contains("esbuild"), "{error}");
    assert_eq!(manifest, fs::read(application.join("Cargo.toml")).unwrap());
    assert_eq!(lock, fs::read(application.join("Cargo.lock")).unwrap());
    assert_eq!(npm, fs::read(application.join("package.json")).unwrap());
}
