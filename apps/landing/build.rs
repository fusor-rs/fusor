#[path = "build/highlight.rs"]
mod highlight;

use std::{env, fmt::Write, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build/highlight.rs");
    let highlighter = highlight::Highlighter::new();
    let mut generated = String::new();
    let mut files = Vec::new();
    for example in ["counter", "search", "keyed_list", "async_data"] {
        for (language, source) in [
            ("rs", format!("src/{example}.rs")),
            ("html", format!("web/components/{example}.html")),
        ] {
            let constant = format!("{}_{}", example.to_uppercase(), language.to_uppercase());
            files.push((constant, format!("{example}.{language}"), language, source));
        }
        // The page that hosts this example is displayed, not compiled into this app.
        for (language, name, source) in [
            ("rs", "app.rs", format!("host/{example}/src/app.rs")),
            (
                "html",
                "index.html",
                format!("host/{example}/web/index.html"),
            ),
        ] {
            let constant = format!(
                "{}_PAGE_{}",
                example.to_uppercase(),
                language.to_uppercase()
            );
            files.push((constant, name.to_owned(), language, source));
        }
    }
    for (constant, name, language, source) in files {
        println!("cargo:rerun-if-changed={source}");
        let code = fs::read_to_string(&source)?;
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
