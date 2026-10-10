use fusor_build::BuildError;
use fusor_build::app::{AppConfig, ArtifactManifest, generate};
use std::{error::Error as _, fs};

fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("web")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        r#"
[package]
name = "consumer"
[package.metadata.fusor]
entry = "web/index.html"
base-path = "/tools/issues/"
[package.metadata.fusor.components]
counter = "web/counter.html"
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("web/index.html"),
        r#"<!doctype html><html><body>
<script type="text/rust">use crate::ui::counter::Counter; struct App;</script>
<main rust:component="App"><Counter></Counter><Counter></Counter></main>
<!-- literal </body> -->
</body></html>"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("web/counter.html"),
        r#"
<script type="text/rust">pub struct Counter;</script>
<template rust:component="Counter"><p>{{ 42 }}</p></template>
"#,
    )
    .unwrap();
    dir
}

const EXTERNAL_MODULE: &str =
    "//! Module documentation\n#![deny(unsafe_code)]\nstruct App;\nfusor::bindings!(app);\n";
const EXTERNAL_HTML: &str = r#"<script type="text/rust" src="../src/app.rs" rust:module="crate::app"></script><App state="{{ App }}"><main></main></App>"#;

fn external_application() -> tempfile::TempDir {
    let dir = setup();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/app.rs"), EXTERNAL_MODULE).unwrap();
    fs::write(dir.path().join("web/index.html"), EXTERNAL_HTML).unwrap();
    dir
}

#[test]
fn external_registration_tracks_source_and_keeps_native_module_ownership() {
    let dir = external_application();
    let native = dir.path().join("src/app.rs");
    let generate_app = || generate(&dir.path().join("Cargo.toml"), &dir.path().join("out"));
    let artifact = generate_app().unwrap();
    let source = &artifact.sources[0];
    assert_eq!(
        source.external.as_ref().unwrap().canonicalize().unwrap(),
        native.canonicalize().unwrap()
    );
    assert!(source.registration.is_some());
    assert_eq!(fs::read_to_string(&native).unwrap(), EXTERNAL_MODULE);
    assert!(
        !fs::read_to_string(&source.rust)
            .unwrap()
            .contains("struct App")
    );
    assert!(
        !fs::read_to_string(&artifact.module)
            .unwrap()
            .contains("pub mod app")
    );
}

#[test]
fn external_registration_rejects_missing_escaping_and_duplicate_modules() {
    let dir = external_application();
    let path = dir.path().join("web/index.html");
    let generate_app = || generate(&dir.path().join("Cargo.toml"), &dir.path().join("out"));
    fs::write(
        &path,
        EXTERNAL_HTML.replace("../src/app.rs", "../src/missing.rs"),
    )
    .unwrap();
    let error = generate_app().unwrap_err();
    let BuildError::Source(source_error) = &error else {
        panic!("{error}")
    };
    assert_eq!(source_error.path, path.canonicalize().unwrap());
    assert_eq!(source_error.location, Some((1, 1)));
    let cause = source_error
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
    let missing = error.to_string();
    assert!(
        missing.contains("index.html:1:1: external Rust source "),
        "{missing}"
    );
    fs::write(
        &path,
        EXTERNAL_HTML.replace("../src/app.rs", "../Cargo.toml"),
    )
    .unwrap();
    assert!(
        generate_app()
            .unwrap_err()
            .to_string()
            .contains("external Rust source must be a .rs file")
    );
    fs::create_dir(dir.path().join("dist")).unwrap();
    fs::write(dir.path().join("dist/generated.rs"), EXTERNAL_MODULE).unwrap();
    fs::write(
        &path,
        EXTERNAL_HTML.replace("../src/app.rs", "../dist/generated.rs"),
    )
    .unwrap();
    assert!(
        generate_app()
            .unwrap_err()
            .to_string()
            .contains("outside its output directory")
    );
    fs::write(&path, EXTERNAL_HTML).unwrap();
    fs::write(dir.path().join("web/counter.html"), r#"<script type="text/rust" src="../src/app.rs" rust:module="crate::app"></script><template rust:component="Counter"><p></p></template>"#).unwrap();
    assert!(
        generate_app()
            .unwrap_err()
            .to_string()
            .contains("registered more than once")
    );
}

#[test]
fn source_graph_links_unique_templates_once_and_emits_normal_rust_modules() {
    let dir = setup();
    let out = dir.path().join("out");
    let artifact = generate(&dir.path().join("Cargo.toml"), &out).unwrap();
    let html = fs::read_to_string(&artifact.html).unwrap();
    assert_eq!(html.matches("data-fusor-component=\"0\"").count(), 1);
    assert_eq!(html.matches("data-fusor-component=").count(), 2);
    assert!(html.find("<!-- literal </body> -->").unwrap() < html.find("<template").unwrap());
    assert!(html.find("<template").unwrap() < html.rfind("</body>").unwrap());
    assert!(!html.contains("text/rust"));
    assert!(!html.contains("boot.js")); // loader is inserted by the CLI
    assert!(html.is_char_boundary(artifact.loader_offset));
    let module = fs::read_to_string(&artifact.module).unwrap();
    assert!(module.contains("pub mod ui"));
    assert!(module.contains("pub mod app"));
    assert!(module.contains("pub mod counter"));
    assert_eq!(artifact.sources.len(), 2);
    assert_eq!(
        ArtifactManifest::read(&out.join("fusor_artifacts.json"))
            .unwrap()
            .sources
            .len(),
        2
    );
    let counter = fs::read_to_string(&artifact.sources[1].rust).unwrap();
    assert!(counter.contains("TemplateComponent"));
    assert!(
        counter
            .lines()
            .nth(1)
            .unwrap()
            .contains("pub struct Counter;")
    );
}

#[test]
fn duplicate_files_and_non_template_components_are_rejected_with_source_context() {
    let dir = setup();
    let path = dir.path().join("web/counter.html");
    for source in [
        "<script type=text/rust>pub struct Counter;</script><p rust:component=Counter>Not a template</p>",
        "<script type=text/rust>pub struct Counter;</script><style>p { color: red; }</style>",
        "<script src=app.js></script>",
        "<!doctype html><template rust:component=Counter><p></p></template>",
        "<template><p>No type</p></template>",
        "Lost text<template rust:component=Counter><p></p></template>",
    ] {
        fs::write(&path, source).unwrap();
        let error = generate(&dir.path().join("Cargo.toml"), &dir.path().join("out"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("counter.html:"), "{error}");
    }
    let manifest = dir.path().join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("web/counter.html", "web/index.html");
    fs::write(&manifest, text).unwrap();
    assert!(
        generate(&manifest, &dir.path().join("out"))
            .unwrap_err()
            .to_string()
            .contains("registered more than once")
    );
}

#[test]
fn configuration_rejects_typos_invalid_modules_and_overlapping_output() {
    let dir = setup();
    let path = dir.path().join("Cargo.toml");
    let valid = fs::read_to_string(&path).unwrap();
    fs::write(&path, "[package").unwrap();
    let error = AppConfig::load(&path).unwrap_err();
    assert!(matches!(error, BuildError::Manifest(_)), "{error}");
    for text in [
        valid.replace("base-path", "basepath"),
        valid.replace("counter =", "app ="),
        valid.replace("counter =", "self ="),
        valid.replace("counter =", "\"some-name\" ="),
        valid.replace("/tools/issues/", "/../outside/"),
        valid.replace("entry =", "output = \"web\"\nentry ="),
        valid.replace("entry =", "output = \"src/output\"\nentry ="),
        valid.replace("web/counter.html", "../counter.html"),
        valid.replace(
            "entry =",
            "assets-build = [\"node\", \"build.mjs\"]\nentry =",
        ),
        valid.replace("entry =", "assets-build = [\"\"]\nentry ="),
        valid.replace("entry =", "dev-refresh = \"false\"\nentry ="),
    ] {
        fs::write(&path, &text).unwrap();
        assert!(AppConfig::load(&path).is_err(), "accepted {text}");
    }
}

#[test]
fn optional_asset_command_and_refresh_policy_are_explicit_configuration() {
    let dir = setup();
    let path = dir.path().join("Cargo.toml");
    assert!(AppConfig::load(&path).unwrap().dev_refresh);
    fs::create_dir(dir.path().join("public")).unwrap();
    let source = fs::read_to_string(&path).unwrap().replace("entry =", "assets = \"public\"\nassets-build = [\"node\", \"build-assets.mjs\"]\ndev-refresh = false\nentry =");
    fs::write(&path, &source).unwrap();
    let config = AppConfig::load(&path).unwrap();
    assert!(!config.dev_refresh);
    assert_eq!(config.assets_build, ["node", "build-assets.mjs"]);
    fs::write(
        &path,
        source.replace(
            r#"["node", "build-assets.mjs"]"#,
            r#""npx some-tool build""#,
        ),
    )
    .unwrap();
    assert_eq!(
        AppConfig::load(&path).unwrap_err().to_string(),
        "assets-build expects an array of executable and arguments, not a string; \
         commands run without a shell. Use assets-build = [\"npx\", \"some-tool\", \"build\"]"
    );
}

fn native_template_application() -> (tempfile::TempDir, String) {
    let dir = setup();
    fs::write(dir.path().join("web/index.html"), r#"<!doctype html><html><body><App state="{{ App }}"><main><Counter count="{{ state.count.clone() }}"></Counter></main></App></body></html>"#).unwrap();
    fs::create_dir_all(dir.path().join("web/components/nested")).unwrap();
    fs::write(
        dir.path().join("web/components/nested/panel.html"),
        r#"<template rust:component="Panel"><section></section></template>"#,
    )
    .unwrap();
    // The explicit file remains inline/compatible; discovery must deduplicate it.
    fs::rename(
        dir.path().join("web/counter.html"),
        dir.path().join("web/components/counter.html"),
    )
    .unwrap();
    let manifest = dir.path().join("Cargo.toml");
    let original = fs::read_to_string(&manifest)
        .unwrap()
        .replace("web/counter.html", "web/components/counter.html");
    fs::write(&manifest, &original).unwrap();
    (dir, original)
}

#[test]
fn native_templates_are_discovered_once_and_mirrored_for_macro_expansion() {
    let (dir, original) = native_template_application();
    let manifest = dir.path().join("Cargo.toml");
    let out = dir.path().join("out");
    let artifact = generate(&manifest, &out).unwrap();
    assert_eq!(artifact.sources.len(), 3);
    assert_eq!(
        artifact.sources[0].rust,
        out.canonicalize()
            .unwrap()
            .join("fusor_templates/web/index.html.rs")
    );
    assert!(
        artifact.sources[2]
            .rust
            .ends_with("fusor_templates/web/components/nested/panel.html.rs")
    );
    let html = fs::read_to_string(&artifact.html).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(artifact.loader_offset > html.find("</main>").unwrap());
    assert!(artifact.loader_offset <= html.find("</body>").unwrap());
    let module = fs::read_to_string(&artifact.module).unwrap();
    assert!(!module.contains("pub mod app"));
    assert!(module.contains("pub mod counter"));
    let removed = artifact.sources[2].rust.clone();
    fs::remove_file(dir.path().join("web/components/nested/panel.html")).unwrap();
    assert_eq!(generate(&manifest, &out).unwrap().sources.len(), 2);
    assert!(
        !removed.exists(),
        "removed templates must not leave includable stale bindings"
    );
    fs::write(
        &manifest,
        original.replace("base-path =", "templates = []\nbase-path ="),
    )
    .unwrap();
    assert_eq!(
        AppConfig::load(&manifest)
            .unwrap()
            .discover_sources(dir.path())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn template_discovery_checks_configuration_and_new_directory_membership() {
    let dir = setup();
    let manifest = dir.path().join("Cargo.toml");
    let config = AppConfig::load(&manifest).unwrap();
    assert_eq!(config.discover_sources(dir.path()).unwrap().len(), 2);
    fs::create_dir_all(dir.path().join("web/components")).unwrap();
    fs::write(
        dir.path().join("web/components/added.html"),
        "<template rust:component=Added><p></p></template>",
    )
    .unwrap();
    assert_eq!(config.discover_sources(dir.path()).unwrap().len(), 3);
    let original = fs::read_to_string(&manifest).unwrap();
    for templates in ["../outside", "dist", "dist/nested", "."] {
        fs::write(
            &manifest,
            original.replace(
                "base-path =",
                &format!("templates = [\"{templates}\"]\nbase-path ="),
            ),
        )
        .unwrap();
        assert!(AppConfig::load(&manifest).is_err(), "{templates}");
    }
    #[cfg(unix)]
    {
        fs::write(&manifest, original).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("web/counter.html"),
            dir.path().join("web/components/linked.html"),
        )
        .unwrap();
        assert!(
            config
                .discover_sources(dir.path())
                .unwrap_err()
                .to_string()
                .contains("symlinks")
        );
    }
}

fn javascript_application() -> tempfile::TempDir {
    let dir = setup();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/counter.rs"),
        r#"
mod nested {
#[derive(fusor::JsInputs)]
struct Counter { #[js] count: fusor::Signal<Option<Vec<i32>>>, private: fusor::Signal<String> }
fusor::template!("web/counter.html");
}
"#,
    )
    .unwrap();
    fs::write(dir.path().join("web/counter.html"), r#"<template rust:component="Counter"><script type="module" src="counter.ts"></script><article></article></template>"#).unwrap();
    fs::write(
        dir.path().join("web/counter.ts"),
        "export function onMount({ inputs }) { inputs.count.subscribe(console.log); }",
    )
    .unwrap();
    fs::write(dir.path().join("web/index.html"), r#"<script type="text/rust">#[derive(JsInputs)] struct App { #[js] speed: Signal<f64> }</script><App state="{{ crate::app::App::new() }}"><script type="module">import './counter.ts';</script><main></main></App>"#).unwrap();
    dir
}

#[test]
fn native_javascript_metadata_paths_and_editor_types_need_no_node() {
    let dir = javascript_application();
    let artifact = generate(&dir.path().join("Cargo.toml"), &dir.path().join("out")).unwrap();
    assert_eq!(artifact.javascript.len(), 2);
    let app = &artifact.javascript[0];
    assert!(app.inline);
    assert_eq!(app.component, "App");
    assert!(
        fs::read_to_string(&app.path)
            .unwrap()
            .contains("import './counter.ts'")
    );
    let app_types = fs::read_to_string(&app.declaration).unwrap();
    assert!(app_types.contains("speed: ReadonlyInput<number>"));
    let counter = &artifact.javascript[1];
    assert!(!counter.inline);
    assert_eq!(
        counter.path,
        dir.path().join("web/counter.ts").canonicalize().unwrap()
    );
    let types = fs::read_to_string(&counter.declaration).unwrap();
    assert!(types.contains("count: ReadonlyInput<(Array<number> | null)>"));
    assert!(!types.contains("private"));
    assert_eq!(counter.declaration_name, "web-counter-html-Counter.d.ts");
    assert!(
        counter
            .rust_source
            .as_ref()
            .unwrap()
            .ends_with("src/counter.rs")
    );
    assert!(
        !dir.path().join(".fusor").exists(),
        "Cargo compiler writes only to OUT_DIR; CLI publishes editor types"
    );
}

#[test]
fn javascript_discovery_rejects_unresolved_sources_and_retains_causes() {
    let dir = javascript_application();
    let manifest = dir.path().join("Cargo.toml");
    let output = dir.path().join("out");
    fs::remove_file(dir.path().join("src/counter.rs")).unwrap();
    let error = generate(&manifest, &output).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot discover JavaScript inputs for Counter")
    );
    fs::write(dir.path().join("src/counter.rs"), "mod broken {").unwrap();
    let error = generate(&manifest, &output).unwrap_err();
    let BuildError::Source(error) = error else {
        panic!("expected a source diagnostic");
    };
    assert!(
        error
            .source()
            .unwrap()
            .downcast_ref::<syn::Error>()
            .is_some()
    );
    fs::write(dir.path().join("web/counter.html"), r#"<template rust:component="Counter"><script type="module" src="./missing.ts"></script><article></article></template>"#).unwrap();
    fs::write(
        dir.path().join("src/counter.rs"),
        "struct Counter; fusor::template!(\"web/counter.html\");",
    )
    .unwrap();
    let error = generate(&manifest, &output).unwrap_err();
    let BuildError::Source(error) = error else {
        panic!("expected a source diagnostic");
    };
    assert_eq!(
        error
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    let error = error.to_string();
    assert!(error.contains("counter.html:1:"));
    assert!(error.contains("JavaScript source"));
}
