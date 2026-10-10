//! Sources are staged and renamed into place, so an interrupted run leaves
//! nothing half-created. A failed preparation keeps the sources and says how to
//! resume.
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    pipeline::cargo::{Mode, compile},
    toolchain,
    transaction::Staging,
    workspace::Project,
};
use std::{fs, path::Path};

pub(crate) fn run(
    cx: &Context,
    path: &Path,
    framework: Option<&Path>,
    javascript: bool,
    skip_install: bool,
) -> Result {
    scaffold(cx, path, framework, javascript)?;
    let path = path.canonicalize()?;
    if skip_install {
        cx.reporter.result(format!(
            "Sources created; preparation remains.\n\n  cd {}\n  fusor install\n  fusor dev",
            path.display()
        ))?;
        return Ok(());
    }
    prepare(cx, &path, javascript).map_err(|error| {
        error
            .context(format!("preparing {}", path.display()))
            .remedy(format!(
                "sources are preserved; resume with `fusor install --manifest-path {}`",
                path.join("Cargo.toml").display()
            ))
    })?;
    cx.reporter.result(format!(
        "Application checked\n\n  cd {}\n  fusor dev",
        path.display()
    ))?;
    Ok(())
}

/// `new` either hands back something that builds or says why it does not.
fn prepare(cx: &Context, path: &Path, javascript: bool) -> Result {
    let cx = cx.for_manifest(path.join("Cargo.toml"), None);
    toolchain::rust::preflight(&cx, path, true)?;
    let project = Project::discover(&cx)?;
    toolchain::prepare(&cx, &project, true, javascript)?;
    let mut checked = cx.clone();
    checked.locked = true;
    compile(&checked, &project, Mode::Check).map(drop)
}

fn scaffold(cx: &Context, path: &Path, framework: Option<&Path>, javascript: bool) -> Result {
    if path.exists() {
        return Err(Error::usage(format!(
            "destination already exists: {}",
            path.display()
        )));
    }
    let name = package_name(path)?;
    let manifest = manifest(&name, framework, javascript)?;
    // A malformed manifest should fail before anything is created.
    let _: toml::Value = manifest.parse()?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        "{}{name}-{}",
        layout::SCAFFOLD_PREFIX,
        std::process::id()
    ));
    fs::create_dir(&staging)?;
    let guard = Staging(staging.clone());
    write_sources(&staging, &name, &manifest)?;
    if javascript {
        write_javascript(&staging, &name)?;
    }
    if path.exists() {
        return Err(Error::usage(
            "the destination appeared while scaffolding; no files were overwritten",
        ));
    }
    fs::rename(&staging, path)?;
    drop(guard);
    cx.reporter.result(format!("Created {}", path.display()))?;
    Ok(())
}

/// The directory name is the package name and, with hyphens replaced, a Rust
/// module path. Both constrain it.
fn package_name(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::usage("the destination must end in a UTF-8 package name"))?;
    let module = name.replace('-', "_");
    let acceptable = name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
        && !["self", "super", "crate", "test", "core", "std", "alloc"].contains(&module.as_str())
        && fusor_build::app::valid_module_name(&module);
    if !acceptable {
        return Err(Error::usage(format!("{name:?} is not a usable package name")).remedy(
            "use a lowercase Cargo package name starting with a letter, containing letters, digits, '-' or '_'",
        ));
    }
    Ok(name.to_owned())
}

fn manifest(name: &str, framework: Option<&Path>, javascript: bool) -> Result<String> {
    let runtime = dependency("fusor-core", framework, javascript)?;
    let compiler = dependency("fusor-build", framework, javascript)?;
    let components = dependency("fusor-components", framework, javascript)?;
    let bindgen = layout::BINDGEN_VERSION;
    Ok(format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[workspace]

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
fusor-core = {runtime}
fusor-components = {components}
wasm-bindgen = "={bindgen}"

[build-dependencies]
fusor-build = {compiler}

[package.metadata.fusor]
assets = {{ "/" = "public" }}
output = "dist"
base-path = "/"

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
"#
    ))
}

/// Pinned to this CLI's exact version, optionally from a local checkout until
/// the registry release.
fn dependency(crate_name: &str, framework: Option<&Path>, javascript: bool) -> Result<String> {
    let mut fields = toml::map::Map::new();
    fields.insert(
        "version".into(),
        toml::Value::String(format!("={}", env!("CARGO_PKG_VERSION"))),
    );
    if let Some(framework) = framework {
        let crate_path = framework.join("crates").join(crate_name).canonicalize()?;
        if !crate_path.join("Cargo.toml").is_file() {
            return Err(Error::usage(format!(
                "--framework-path has no {crate_name} package"
            )));
        }
        fields.insert(
            "path".into(),
            toml::Value::String(
                crate_path
                    .to_str()
                    .ok_or_else(|| Error::usage("--framework-path must be UTF-8"))?
                    .into(),
            ),
        );
    }
    if matches!(crate_name, "fusor-core" | "fusor-components") {
        let mut features = vec![toml::Value::String(
            if crate_name == "fusor-core" {
                "dom"
            } else {
                "browser"
            }
            .into(),
        )];
        if javascript && crate_name == "fusor-core" {
            features.push(toml::Value::String("javascript".into()));
        }
        fields.insert("features".into(), toml::Value::Array(features));
    }
    // An inline table renders Windows paths with the escaping TOML requires.
    Ok(toml::Value::Table(fields).to_string())
}

fn write_sources(staging: &Path, name: &str, manifest: &str) -> Result {
    for directory in ["src", "web/components", "public"] {
        fs::create_dir_all(staging.join(directory))?;
    }
    let toolchain = format!(
        "[toolchain]\nchannel = \"{}\"\nprofile = \"minimal\"\ntargets = [\"{}\"]\n",
        layout::RUST_VERSION,
        layout::TARGET
    );
    for (file, content) in [
        ("Cargo.toml", manifest),
        ("rust-toolchain.toml", &toolchain),
        (
            "build.rs",
            "fn main() -> Result<(), fusor_build::BuildError> {\n    fusor_build::compile_app()\n}\n",
        ),
        ("src/lib.rs", "mod app;\nmod counter;\n"),
        ("src/app.rs", include_str!("../../template/app.rs")),
        ("src/counter.rs", include_str!("../../template/counter.rs")),
        ("web/index.html", include_str!("../../template/index.html")),
        (
            "web/components/counter.html",
            include_str!("../../template/counter.html"),
        ),
        ("public/app.css", include_str!("../../template/app.css")),
        (".gitignore", GITIGNORE),
        ("README.md", &readme(name)),
    ] {
        fs::write(staging.join(file), content)?;
    }
    Ok(())
}

fn write_javascript(staging: &Path, name: &str) -> Result {
    fs::create_dir_all(staging.join(layout::STATE))?;
    // Lets a later `fusor install` create the lockfile.
    fs::write(
        staging.join(layout::PENDING_JAVASCRIPT),
        env!("CARGO_PKG_VERSION"),
    )?;
    fs::write(
        staging.join("package.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": name, "private": true, "type": "module",
            "devDependencies": { "esbuild": layout::ESBUILD_VERSION }
        }))?,
    )?;
    fs::write(staging.join("web/app.js"), STARTER_JAVASCRIPT)?;
    fs::write(
        staging.join("web/index.html"),
        include_str!("../../template/index.html").replace(
            "  <App state=\"{{ App::new() }}\">",
            "  <App state=\"{{ App::new() }}\">\n    <script type=\"module\" src=\"./app.js\"></script>",
        ),
    )?;
    let readme = fs::read_to_string(staging.join("README.md"))?;
    fs::write(
        staging.join("README.md"),
        format!("{readme}\n{JAVASCRIPT_README}"),
    )?;
    Ok(())
}

const GITIGNORE: &str = "/target/
/dist/
/.fusor-*/
/.fusor/
/node_modules/
";

const STARTER_JAVASCRIPT: &str = r#"// Import browser libraries here using ordinary JavaScript imports.
export function onMount({ root, signal, onCleanup }) {
  root.addEventListener('pointerdown', () => {
    root.dataset.lastPointer = 'pressed';
  }, { signal });
  onCleanup(() => {
    delete root.dataset.lastPointer;
  });
}
"#;

const JAVASCRIPT_README: &str =
    "JavaScript modules are enabled. Creation prepares package-lock.json; commit it
and use `fusor install --locked` for later installations. Builds only use installed,
locked dependencies and never install packages. Add libraries with `npm install PACKAGE`
and import them from web/app.js. External .ts modules are transpiled; use a separate
`tsc --noEmit` step when type checking is wanted.
";

fn readme(name: &str) -> String {
    format!(
        r#"# {name}

Run `fusor install` to prepare a fresh clone, then `fusor dev`.
Use `fusor check` to type-check and `fusor build` for a static site in `dist/`.

Associate HTML with an ordinary Rust module using fusor::template!("web/index.html")
or a path under web/components/. The build discovers these templates automatically.
Derive FromInputs with #[input] for parent values and #[local(init = ...)] for
per-instance state. The derive generates the typed input contract; manual FromInputs
implementations support custom setup. The <App state=...> boundary constructs and
retains the application.
Pass signals to share reactive inputs; each component keeps its own local state.
"#
    )
}
