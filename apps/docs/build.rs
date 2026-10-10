use docs_base_build::{Config, Highlighter};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let highlighter = Highlighter::default();
    let guides = docs_base_build::compile(
        &Config {
            root: Path::new("."),
            content: Path::new("public/content"),
            navigation: Path::new("content/navigation.json"),
            references: Some(Path::new("content/references.json")),
            base_path: "/docs/",
            repository: None,
        },
        &highlighter,
    )?;
    guides.write_assets(Path::new("public"))?;
    let mut source = guides.source.to_string();
    compile_showcase(&mut source, &highlighter)?;
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("content.rs"),
        source,
    )?;
    fusor_build::compile_app()?;
    Ok(())
}

fn compile_showcase(
    source: &mut String,
    highlighter: &Highlighter,
) -> Result<(), Box<dyn std::error::Error>> {
    let text = |v: &serde_json::Value, key: &str| format!("{:?}", v[key].as_str().unwrap_or(""));
    println!("cargo:rerun-if-changed=content/showcase.json");
    let demos: serde_json::Value = serde_json::from_slice(&fs::read("content/showcase.json")?)?;
    source.push_str("pub static DEMOS: &[DemoData] = &[\n");
    for demo in demos.as_array().ok_or("showcase must be an array")? {
        source.push_str("DemoData {");
        for key in [
            "slug",
            "title",
            "category",
            "description",
            "try_it",
            "explanation",
            "guide",
            "guide_label",
            "preview_title",
            "preview_value",
            "preview_detail",
        ] {
            source.push_str(&format!("{key}: {},", text(demo, key)));
        }
        source.push_str(&format!(
            "workers: {},",
            demo["workers"].as_bool().unwrap_or(false)
        ));
        let slug = demo["slug"].as_str().ok_or("demo slug is required")?;
        for (language, extension, folder) in [
            ("rust", "rs", "src"),
            ("html", "html", "web"),
            ("javascript", "js", "web"),
        ] {
            let path = format!("{folder}/demos/{slug}.{extension}");
            println!("cargo:rerun-if-changed={path}");
            if language == "javascript" && !demo["javascript"].as_bool().unwrap_or(false) {
                source.push_str("javascript: None,");
                continue;
            }
            compile_demo_source(source, highlighter, language, &path)?;
        }
        source.push_str("},\n");
    }
    source.push_str("];\n");
    Ok(())
}

fn compile_demo_source(
    source: &mut String,
    highlighter: &Highlighter,
    language: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let code = fs::read_to_string(path)?;
    let tokens = highlighter.tokens(&code, path)?;
    let value = format!("CodeData {{ label: {path:?}, tokens: {tokens} }}");
    let value = if language == "javascript" {
        format!("Some({value})")
    } else {
        value
    };
    source.push_str(&format!("{language}: {value},"));
    Ok(())
}
