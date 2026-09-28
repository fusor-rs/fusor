//! The compiler records the loader's byte offset; nothing here parses HTML.
use crate::error::{Error, Result};
use fusor_build::app::ArtifactManifest;
use std::{fs, path::Path};

/// The generated entry module and the modules it statically imports, relative
/// to the package. Without these hints the browser discovers them one import
/// level at a time, after the boot module runs.
pub(crate) fn entry_modules(package: &Path) -> Result<Vec<String>> {
    let entry = fs::read_to_string(package.join("app.js"))?;
    let mut modules = vec!["app.js".to_owned()];
    for specifier in entry.lines().filter_map(static_import) {
        let module = specifier.trim_start_matches("./").to_owned();
        if !modules.contains(&module) {
            modules.push(module);
        }
    }
    Ok(modules)
}

/// A top-level `import … from './file.js'` of a module beside or below the entry.
fn static_import(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("import ")?;
    let source = rest[rest.rfind(" from ")? + 6..]
        .trim()
        .trim_end_matches(';');
    let specifier = ['"', '\'']
        .into_iter()
        .find_map(|quote| source.strip_prefix(quote)?.strip_suffix(quote))?;
    let local = specifier.starts_with("./") && !specifier.contains("..");
    (local && specifier.ends_with(".js")).then_some(specifier)
}

/// Production preloads: the entry module graph and the Wasm the entry fetches.
fn preloads(url_prefix: &str, modules: &[String]) -> String {
    if modules.is_empty() {
        return String::new();
    }
    let mut links: String = modules
        .iter()
        .map(|module| format!("<link rel=\"modulepreload\" href=\"{url_prefix}/pkg/{module}\">"))
        .collect();
    // wasm-bindgen fetches the module in CORS mode with same-origin credentials.
    links.push_str(&format!(
        "<link rel=\"preload\" href=\"{url_prefix}/pkg/app_bg.wasm\" as=\"fetch\" type=\"application/wasm\" crossorigin>"
    ));
    links
}

/// `revision` is set only in development, where the refresh client uses it to
/// tell a patched document from a stale one. `modules` are preloaded.
pub(crate) fn render(
    artifact: &ArtifactManifest,
    url_prefix: &str,
    revision: Option<u64>,
    styles: &[String],
    modules: &[String],
) -> Result<String> {
    let mut html = fs::read_to_string(&artifact.html)?;
    let offset = artifact.loader_offset;
    if !html.is_char_boundary(offset) {
        return Err(Error::internal(
            "the compiler recorded a loader offset inside a character",
        ));
    }
    let marker = revision
        .map(|revision| format!(" data-fusor-revision=\"{revision}\""))
        .unwrap_or_default();
    let stylesheets = styles
        .iter()
        .map(|style| {
            let style = style.replace('&', "&amp;").replace('"', "&quot;");
            format!("<link rel=\"stylesheet\" href=\"{url_prefix}/pkg/{style}\">")
        })
        .collect::<String>();
    let preloads = preloads(url_prefix, modules);
    html.insert_str(
        offset,
        &format!(
            "{stylesheets}{preloads}<script type=\"module\"{marker} src=\"{url_prefix}/boot.js\"></script>"
        ),
    );
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_imports_are_local_module_files_only() {
        for (line, expected) in [
            (
                "import { a } from './snippets/x/inline0.js';",
                Some("./snippets/x/inline0.js"),
            ),
            (
                "import * as import1 from \"./snippets/x/src/flat.js\"",
                Some("./snippets/x/src/flat.js"),
            ),
            ("import { from } from './chunk.js';", Some("./chunk.js")),
            ("import value from 'package';", None),
            ("import { a } from '../outside.js';", None),
            ("import './side-effect.js';", None),
            ("import { a } from './style.css';", None),
            ("  import { a } from './nested.js';", None),
            ("const lazy = import('./lazy.js');", None),
            ("export { default } from './entry.js';", None),
        ] {
            assert_eq!(static_import(line), expected, "{line}");
        }
    }

    #[test]
    fn preloads_the_module_graph_and_wasm_or_nothing() {
        assert_eq!(preloads("/base/__fusor/g", &[]), "");
        let links = preloads(
            "/base/__fusor/g",
            &["app.js".into(), "snippets/a.js".into()],
        );
        assert_eq!(
            links,
            "<link rel=\"modulepreload\" href=\"/base/__fusor/g/pkg/app.js\"><link rel=\"modulepreload\" href=\"/base/__fusor/g/pkg/snippets/a.js\"><link rel=\"preload\" href=\"/base/__fusor/g/pkg/app_bg.wasm\" as=\"fetch\" type=\"application/wasm\" crossorigin>"
        );
    }
}
