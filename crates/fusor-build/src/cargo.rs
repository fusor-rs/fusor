//! Cargo I/O adapter. The compiler stages remain pure and usable without Cargo.
use crate::{
    BuildError, SourceMap,
    app::{MANIFEST_FILE, MODULE_FILE, SourceError},
    extract,
};
use quote::quote;
use std::{env, fs, path::Path};

fn worker_build_mode() {
    println!("cargo::rustc-check-cfg=cfg(fusor_worker)");
    println!("cargo::rerun-if-env-changed=FUSOR_WORKER_BUILD");
    if env::var_os("FUSOR_WORKER_BUILD").is_some_and(|value| value == "1") {
        println!("cargo::rustc-cfg=fusor_worker");
    }
}

/// Compile the entry and component files in `[package.metadata.fusor]`.
/// Include `FUSOR_MODULE` in lib.rs to expose inline `crate::ui` modules and
/// verify external native modules registered with [`fusor::bindings!`].
pub fn compile_app() -> Result<(), BuildError> {
    worker_build_mode();
    let root = env::var_os("CARGO_MANIFEST_DIR").ok_or("compile_app must run from build.rs")?;
    let manifest = Path::new(&root).join("Cargo.toml");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let config = crate::app::AppConfig::load(&manifest)?;
    println!("cargo::rustc-env=FUSOR_BASE_PATH={}", config.base_path);
    for source in config.discover_sources(Path::new(&root))? {
        println!(
            "cargo::rerun-if-changed={}",
            Path::new(&root).join(&source.path).display()
        );
    }
    // Cargo watches directory membership, including additions/removals. A
    // missing default directory is covered by its closest existing ancestor.
    for directory in &config.templates {
        let mut watch = Path::new(&root).join(directory);
        while !watch.exists() {
            if !watch.pop() {
                break;
            }
        }
        println!("cargo::rerun-if-changed={}", watch.display());
    }
    let out = env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?;
    let artifacts = crate::app::generate(&manifest, Path::new(&out))?;
    for source in &artifacts.sources {
        if let Some(path) = &source.external {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
    for module in &artifacts.javascript {
        if let Some(path) = &module.rust_source {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
    println!(
        "cargo::rustc-env=FUSOR_MODULE={}",
        artifacts.module.display()
    );
    println!(
        "cargo::rustc-env=FUSOR_ARTIFACT_MANIFEST={}",
        Path::new(&out).join(MANIFEST_FILE).display()
    );
    Ok(())
}

/// Call from a package's build.rs. Cargo then recompiles when the HTML changes.
/// The package's lib.rs only needs `include!(env!("FUSOR_MODULE"));`.
/// This helper writes exclusively to Cargo's OUT_DIR.
pub fn compile(path: impl AsRef<Path>) -> Result<(), BuildError> {
    worker_build_mode();
    let manifest = env::var_os("CARGO_MANIFEST_DIR").ok_or("compile must run from build.rs")?;
    let source_path = Path::new(&manifest).join(path).canonicalize()?;
    println!("cargo::rerun-if-changed={}", source_path.display());
    let source = fs::read_to_string(&source_path)?;
    let page = extract(&source).map_err(|error| SourceError::extracted(&source_path, error))?;
    if !page.javascript.is_empty() {
        return Err("component JavaScript requires compile_app() and cargo fusor build".into());
    }
    if page.blocks.iter().any(|block| block.external.is_some()) {
        return Err(
            "external Rust sources require compile_app() and [package.metadata.fusor]".into(),
        );
    }
    println!("cargo::rustc-env=FUSOR_BASE_PATH=/");
    if page.blocks.is_empty() {
        return Err(SourceError::new(
            &source_path,
            "expected at least one <script type=\"text/rust\"> block",
        )
        .into());
    }
    let out_dir = env::var_os("OUT_DIR").ok_or("Cargo did not provide OUT_DIR")?;
    let out_dir = Path::new(&out_dir);
    let rust_path = out_dir.join("fusor_page.rs");
    let html_path = out_dir.join("fusor_page.html");
    let module_path = out_dir.join(MODULE_FILE);
    let map_path = out_dir.join("fusor_bindings.map");
    fs::write(&rust_path, &page.rust)?;
    fs::write(&html_path, page.with_loader())?;
    let map = SourceMap::new(page.locations.clone())?;
    fs::write(&map_path, map.to_string())?;
    // A normal module file also supports inner docs/attributes. include! alone
    // would restrict those, so the shim includes this ordinary module declaration.
    let rust_module = rust_path
        .to_str()
        .ok_or("generated Rust module path must be UTF-8")?;
    let module = quote! {
        #[path = #rust_module]
        mod fusor_page;
        pub use fusor_page::*;
    };
    fs::write(&module_path, module.to_string())?;
    for (name, path) in [
        ("FUSOR_MODULE", module_path),
        ("FUSOR_HTML_SOURCE", source_path),
        ("FUSOR_RUST_SOURCE", rust_path),
        ("FUSOR_HTML_OUTPUT", html_path),
        ("FUSOR_BINDING_MAP", map_path),
    ] {
        println!("cargo::rustc-env={name}={}", path.display());
    }
    Ok(())
}
