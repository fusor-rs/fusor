use std::{env, fs, path::PathBuf};
#[path = "build/highlight.rs"]
mod highlight;
#[path = "build/prose.rs"]
mod prose;
#[path = "build/reference.rs"]
mod reference;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build/highlight.rs");
    println!("cargo:rerun-if-changed=build/prose.rs");
    println!("cargo:rerun-if-changed=build/reference.rs");
    let highlighter = highlight::Highlighter::new();
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
        if fs::read(&output).ok().as_ref() != Some(&contents) {
            fs::write(output, contents)?;
        }
    }
    println!("cargo:rerun-if-changed=content/pages.json");
    let pages: serde_json::Value = serde_json::from_slice(&fs::read("content/pages.json")?)?;
    validate_pages(pages.as_array().ok_or("pages must be an array")?)?;
    println!("cargo:rerun-if-changed=content/references.json");
    let references: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read("content/references.json")?)?;
    for reference in &references {
        validate_reference(
            &pages,
            reference["href"]
                .as_str()
                .ok_or("reference href required")?,
        )?;
    }
    let text = |v: &serde_json::Value, key: &str| format!("{:?}", v[key].as_str().unwrap_or(""));
    let mut source = String::from("pub static PAGES: &[PageData] = &[\n");
    for page in pages.as_array().ok_or("pages must be an array")? {
        source.push_str(&format!(
            "PageData {{ parent: {:?}, reference: {}, slug: {}, title: {}, group: {}, lead: {}, lead_prose: {}, sections: &[",
            page["parent"].as_str(),
            page["reference"].as_bool().unwrap_or(false),
            text(page, "slug"),
            text(page, "title"),
            text(page, "group"),
            text(page, "lead"),
            prose::compile(page["lead"].as_str().unwrap_or(""))?
        ));
        let sections = page["sections"]
            .as_array()
            .ok_or("sections must be an array")?;
        let ids: std::collections::BTreeSet<&str> = sections
            .iter()
            .filter_map(|section| section["id"].as_str())
            .collect();
        let mut anchors = std::collections::BTreeSet::new();
        for section in sections {
            let structured = reference::section(section, &ids, &highlighter)?;
            for anchor in structured.anchors.iter() {
                if ids.contains(anchor.as_str()) || !anchors.insert(anchor.clone()) {
                    return Err(format!("member anchor repeats an id: {anchor}").into());
                }
            }
            let code = if let Some(path) = section["source"].as_str() {
                println!("cargo:rerun-if-changed={path}");
                fs::read_to_string(path)?
            } else {
                section["code"].as_str().unwrap_or("").to_owned()
            };
            let tokens = highlighter.tokens(&code, section["language"].as_str().unwrap_or(""))?;
            source.push_str(&format!(
                "SectionData {{ id: {}, title: {}, body: {}, body_prose: {}, code: {:?}, tokens: {}, language: {}, note: {}, note_prose: {}, links: &[",
                text(section, "id"),
                text(section, "title"),
                text(section, "body"),
                prose::compile(section["body"].as_str().unwrap_or(""))?,
                code,
                tokens,
                text(section, "language"),
                text(section, "note"),
                prose::compile(section["note"].as_str().unwrap_or(""))?,
            ));
            if let Some(links) = section["links"].as_array() {
                for link in links {
                    source.push_str(&format!(
                        "LinkData {{ href: {}, label: {} }},",
                        text(link, "href"),
                        text(link, "label")
                    ));
                }
            }
            source.push_str("], references: &[");
            if !page["reference"].as_bool().unwrap_or(false) && !code.is_empty() {
                let mut seen = std::collections::BTreeSet::new();
                for reference in &references {
                    let token = reference["token"]
                        .as_str()
                        .ok_or("reference token required")?;
                    let found = code.match_indices(token).any(|(start, _)| {
                        let continuation =
                            |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':');
                        !code[..start].chars().next_back().is_some_and(continuation)
                            && (token.ends_with(':')
                                || !code[start + token.len()..]
                                    .chars()
                                    .next()
                                    .is_some_and(continuation))
                    });
                    if found && seen.insert(reference["href"].as_str().unwrap()) {
                        source.push_str(&format!(
                            "LinkData {{ href: {}, label: {} }},",
                            text(reference, "href"),
                            text(reference, "label")
                        ));
                    }
                }
            }
            source.push_str(&format!("], {} }},\n", structured.source));
        }
        source.push_str("] },\n");
    }
    source.push_str("];\n");

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
            let code = fs::read_to_string(&path)?;
            let tokens = highlighter.tokens(&code, &path)?;
            let value = format!("CodeData {{ label: {path:?}, tokens: {tokens} }}");
            let value = if language == "javascript" {
                format!("Some({value})")
            } else {
                value
            };
            source.push_str(&format!("{language}: {value},"));
            let output = format!("public/source/showcase-{slug}.{extension}.txt");
            println!("cargo:rerun-if-changed={output}");
            if fs::read_to_string(&output).ok().as_ref() != Some(&code) {
                fs::write(output, code)?;
            }
        }
        source.push_str("},\n");
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("content.rs"),
        source,
    )?;
    fusor_build::compile_app()
}

fn validate_pages(pages: &[serde_json::Value]) -> Result<(), Box<dyn std::error::Error>> {
    let mut slugs = std::collections::BTreeMap::new();
    for page in pages {
        let slug = page["slug"].as_str().ok_or("page slug required")?;
        if slugs.insert(slug, page).is_some() {
            return Err(format!("duplicate page: {slug}").into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for section in page["sections"].as_array().ok_or("sections required")? {
            let id = section["id"].as_str().ok_or("section id required")?;
            if !ids.insert(id) {
                return Err(format!("duplicate section: {slug}#{id}").into());
            }
        }
    }
    for page in pages {
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(page["slug"].as_str().unwrap());
        let mut current = page;
        while let Some(parent) = current["parent"].as_str() {
            if !seen.insert(parent) {
                return Err(format!("page hierarchy cycle: {parent}").into());
            }
            let next = slugs
                .get(parent)
                .ok_or_else(|| format!("unknown parent: {parent}"))?;
            if next["group"] != page["group"] {
                return Err(format!("parent and child must share group: {parent}").into());
            }
            current = next;
        }
    }
    Ok(())
}
fn validate_reference(
    pages: &serde_json::Value,
    href: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = href
        .strip_prefix("/docs/")
        .ok_or("reference must point to docs")?;
    let (slug, id) = path
        .split_once('#')
        .ok_or("reference must point to a section")?;
    let page = pages
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["slug"] == slug)
        .ok_or_else(|| format!("unknown reference page: {href}"))?;
    if !page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .any(|section| section["id"] == id)
    {
        return Err(format!("unknown reference section: {href}").into());
    }
    Ok(())
}
