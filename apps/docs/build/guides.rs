use super::{highlight::Highlighter, markdown};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const CONTENT_DIRECTORY: &str = "content";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Navigation {
    slug: String,
    group: String,
    parent: Option<String>,
    #[serde(default)]
    reference: bool,
}

struct Guide {
    navigation: Navigation,
    document: markdown::Document,
    source: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    token: String,
    href: String,
    label: String,
}

pub(crate) fn compile(highlighter: &Highlighter) -> Result<String> {
    println!("cargo:rerun-if-changed=content/navigation.json");
    println!("cargo:rerun-if-changed=public/{CONTENT_DIRECTORY}");
    let navigation: Vec<Navigation> =
        serde_json::from_slice(&fs::read("content/navigation.json")?)?;
    let guides = navigation
        .into_iter()
        .map(|navigation| load(navigation, highlighter))
        .collect::<Result<Vec<_>>>()?;
    validate_navigation(&guides)?;
    println!("cargo:rerun-if-changed=content/references.json");
    let references: Vec<Reference> = serde_json::from_slice(&fs::read("content/references.json")?)?;
    for reference in &references {
        validate_reference(&guides, reference)?;
    }
    let mut source = String::from("pub static PAGES: &[PageData] = &[\n");
    for guide in &guides {
        source.push_str(&page_source(guide, &references));
    }
    source.push_str("];\n");
    Ok(source)
}

fn load(navigation: Navigation, highlighter: &Highlighter) -> Result<Guide> {
    let slug = &navigation.slug;
    if !slug.is_empty()
        && slug.split('/').any(|part| {
            part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return Err(format!("invalid guide slug: {slug}").into());
    }
    let name = if slug.is_empty() { "index" } else { slug };
    let path = format!("public/{CONTENT_DIRECTORY}/{name}.md");
    let markdown = fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))?;
    let document = markdown::parse(&markdown, Path::new("."), highlighter)
        .map_err(|error| format!("{path}: {error}"))?;
    Ok(Guide {
        source: format!("/docs/{CONTENT_DIRECTORY}/{name}.md"),
        navigation,
        document,
    })
}

fn validate_navigation(guides: &[Guide]) -> Result<()> {
    let mut slugs = BTreeMap::new();
    for guide in guides {
        if slugs
            .insert(guide.navigation.slug.as_str(), guide)
            .is_some()
        {
            return Err(format!("duplicate page: {}", guide.navigation.slug).into());
        }
    }
    for guide in guides {
        let mut seen = BTreeSet::from([guide.navigation.slug.as_str()]);
        let mut current = guide;
        while let Some(parent) = &current.navigation.parent {
            if !seen.insert(parent) {
                return Err(format!("page hierarchy cycle: {parent}").into());
            }
            let next = slugs
                .get(parent.as_str())
                .ok_or_else(|| format!("unknown parent: {parent}"))?;
            if next.navigation.group != guide.navigation.group {
                return Err(format!("parent and child must share group: {parent}").into());
            }
            current = next;
        }
    }
    Ok(())
}

fn validate_reference(guides: &[Guide], reference: &Reference) -> Result<()> {
    let href = &reference.href;
    let (slug, id) = href
        .strip_prefix("/docs/")
        .and_then(|path| path.split_once('#'))
        .ok_or_else(|| format!("reference must point to a docs section: {href}"))?;
    if !guides
        .iter()
        .any(|guide| guide.navigation.slug == slug && guide.document.anchors.contains(id))
    {
        return Err(format!("unknown reference target: {href}").into());
    }
    if reference.token.is_empty() {
        return Err(format!("reference token required: {href}").into());
    }
    Ok(())
}

fn page_source(guide: &Guide, references: &[Reference]) -> String {
    let navigation = &guide.navigation;
    let document = &guide.document;
    let references = if navigation.reference {
        &[]
    } else {
        references
    };
    let mut source = format!(
        "PageData {{ parent: {:?}, reference: {}, slug: {:?}, title: {:?}, group: {:?}, lead: {:?}, search: {:?}, source: {:?}, sections: &[",
        navigation.parent,
        navigation.reference,
        navigation.slug,
        document.title,
        navigation.group,
        document.lead,
        document.search,
        guide.source
    );
    for section in &document.sections {
        source.push_str(&format!(
            "SectionData {{ id: {:?}, title: {:?}, html: {:?}, references: &[",
            section.id, section.title, section.body.html
        ));
        let mut seen = BTreeSet::new();
        for reference in references {
            if contains_token(&section.body.code, &reference.token) && seen.insert(&reference.href)
            {
                source.push_str(&format!(
                    "LinkData {{ href: {:?}, label: {:?} }},",
                    reference.href, reference.label
                ));
            }
        }
        source.push_str("] },\n");
    }
    source.push_str("] },\n");
    source
}

fn contains_token(code: &str, token: &str) -> bool {
    code.match_indices(token).any(|(start, _)| {
        let continuation = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':');
        !code[..start].chars().next_back().is_some_and(continuation)
            && (token.ends_with(':')
                || !code[start + token.len()..]
                    .chars()
                    .next()
                    .is_some_and(continuation))
    })
}
