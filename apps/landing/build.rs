#[path = "build/highlight.rs"]
mod highlight;

use std::{env, fmt::Write, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build/highlight.rs");
    let highlighter = highlight::Highlighter::new();
    let mut generated = String::new();
    // The back of each frame on the stage.
    for (constant, name, language, source) in [
        (
            "GREETING_HTML",
            "greeting.html",
            "html",
            "web/components/greeting.html",
        ),
        ("GREETING_RS", "greeting.rs", "rs", "src/greeting.rs"),
        (
            "HYPERCMD_BROWSER_HTML",
            "browser.html",
            "html",
            "shown/hypercmd/browser.html",
        ),
        ("HYPERPANE_RUN", "Terminal", "sh", "shown/hyperpane/run.sh"),
    ] {
        println!("cargo:rerun-if-changed={source}");
        let code = fs::read_to_string(source)?;
        let href = format!("./source/{source}.txt");
        let tokens = highlighter.tokens(&code, language)?;
        writeln!(
            generated,
            "pub static {constant}: CodeFile = CodeFile {{ name: {name:?}, href: {href:?}, tokens: {tokens} }};"
        )?;
    }
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("highlighted.rs"),
        generated,
    )?;
    fusor_build::compile_app()?;
    Ok(())
}
