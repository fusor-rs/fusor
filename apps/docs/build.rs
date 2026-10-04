use std::{env, fs, path::PathBuf};
#[path = "build/guides.rs"]
mod guides;
#[path = "build/highlight.rs"]
mod highlight;
#[path = "build/html.rs"]
mod html;
#[path = "build/markdown.rs"]
mod markdown;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build");
    let highlighter = highlight::Highlighter::new();
    copy_resources()?;
    let mut source = guides::compile(&highlighter)?;
    compile_showcase(&mut source, &highlighter)?;
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("content.rs"),
        source,
    )?;
    fusor_build::compile_app()?;
    Ok(())
}

fn copy_resources() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=content/resources.json");
    let resources: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&fs::read("content/resources.json")?)?;
    fs::create_dir_all("public/source")?;
    for (name, path) in resources {
        println!("cargo:rerun-if-changed={path}");
        let contents = fs::read(path)?;
        let output = PathBuf::from("public/source").join(name);
        // Watched too: a fresh checkout with a cached target/ lacks the copy.
        println!("cargo:rerun-if-changed={}", output.display());
        write_changed(&output, &contents)?;
    }
    Ok(())
}

fn compile_showcase(
    source: &mut String,
    highlighter: &highlight::Highlighter,
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
            compile_demo_source(source, highlighter, slug, language, &path)?;
        }
        source.push_str("},\n");
    }
    source.push_str("];\n");
    Ok(())
}

fn compile_demo_source(
    source: &mut String,
    highlighter: &highlight::Highlighter,
    slug: &str,
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
    let extension = std::path::Path::new(path)
        .extension()
        .ok_or("demo source requires an extension")?
        .to_string_lossy();
    let output = format!("public/source/showcase-{slug}.{extension}.txt");
    println!("cargo:rerun-if-changed={output}");
    write_changed(std::path::Path::new(&output), code.as_bytes())?;
    Ok(())
}

fn write_changed(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    // Missing or changed generated assets must be written before compilation.
    match fs::read(path) {
        Ok(previous) if previous == contents => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::write(path, contents)
}
