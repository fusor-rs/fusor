//! A source that includes a file cannot be reused across a development refresh,
//! except for the includes this compiler arranges itself: `fusor::template!`,
//! `fusor::bindings!` and the generated module. Those shapes are this crate's
//! contract; changing a constant below means changing the matching macro in
//! `crates/fusor-core/src/authoring.rs`.
use proc_macro2::{TokenStream, TokenTree};

pub(crate) const TEMPLATE_DIRECTORY: &str = "fusor_templates";

pub(crate) const BINDINGS_PREFIX: &str = "fusor_";

pub(crate) const MODULE_VARIABLE: &str = "FUSOR_MODULE";

/// `None` when the source does not tokenize; callers should assume it does.
pub fn includes_foreign_file(rust: &str) -> Option<bool> {
    includes_foreign_file_with(rust, |_| false)
}

/// Internal CLI refresh protocol: only direct, literal data includes can be
/// cleared by a caller that proves their resolved files cannot be refreshed.
#[doc(hidden)]
pub fn includes_foreign_file_with(
    rust: &str,
    unchanged_data: impl Fn(&str) -> bool,
) -> Option<bool> {
    rust.parse()
        .ok()
        .map(|stream| walk(&stream, true, &unchanged_data))
}

/// Matched as token streams, so formatting does not matter. The metavariables
/// appear because a path dependency on `fusor` puts the macro definitions
/// themselves in the scanned sources.
fn generated_contracts() -> [TokenStream; 3] {
    [
        format!(r#"concat!(env!("OUT_DIR"), "/{TEMPLATE_DIRECTORY}/", $path, ".rs")"#),
        format!(r#"concat!(env!("OUT_DIR"), "/{BINDINGS_PREFIX}", stringify!($name), ".rs")"#),
        format!(r#"env!("{MODULE_VARIABLE}")"#),
    ]
    .map(|contract| contract.parse().expect("generated include contract"))
}

fn walk(stream: &TokenStream, direct: bool, unchanged_data: &impl Fn(&str) -> bool) -> bool {
    let tokens: Vec<_> = stream.clone().into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        let TokenTree::Group(group) = token else {
            continue;
        };
        let macro_name = macro_name(&tokens[..index]);
        if let Some(name) = macro_name
            .as_deref()
            .filter(|name| matches!(*name, "include" | "include_str" | "include_bytes"))
        {
            if is_generated(name, &group.stream()) {
                continue;
            }
            if direct && name != "include" {
                if let Ok(path) = syn::parse2::<syn::LitStr>(group.stream()) {
                    if unchanged_data(&path.value()) {
                        continue;
                    }
                }
            }
            return true;
        }
        // Other macros can transform or relocate their input. Do not infer
        // include origins inside their opaque arguments or definitions.
        let direct = direct && macro_name.is_none() && !is_attribute(&tokens[..index]);
        if walk(&group.stream(), direct, unchanged_data) {
            return true;
        }
    }
    false
}

fn macro_name(before: &[TokenTree]) -> Option<String> {
    let name = match before {
        [.., TokenTree::Ident(name), TokenTree::Punct(bang)] if bang.as_char() == '!' => name,
        [
            ..,
            TokenTree::Ident(name),
            TokenTree::Punct(bang),
            TokenTree::Ident(_),
        ] if name.to_string().trim_start_matches("r#") == "macro_rules"
            && bang.as_char() == '!' =>
        {
            name
        }
        _ => return None,
    };
    Some(name.to_string().trim_start_matches("r#").to_owned())
}

fn is_attribute(before: &[TokenTree]) -> bool {
    matches!(before, [.., TokenTree::Punct(hash)] if hash.as_char() == '#')
        || matches!(before, [.., TokenTree::Punct(hash), TokenTree::Punct(bang)]
            if hash.as_char() == '#' && bang.as_char() == '!')
}

fn is_generated(macro_name: &str, arguments: &TokenStream) -> bool {
    // This compiler emits Rust to include, never data to embed.
    if macro_name != "include" {
        return false;
    }
    let arguments = arguments.to_string();
    generated_contracts()
        .iter()
        .any(|contract| contract.to_string() == arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_include_outside_the_generated_contract_prevents_reuse() {
        for source in [
            r#"const PAGE: &str = include_str!("index.html");"#,
            r#"fn f() { let _ = std::include_bytes!("public/data.bin"); }"#,
            r#"include!(concat!(env!("OUT_DIR"), "/custom.rs"));"#,
            r#"macro_rules! content { () => { include_str!("file") } }"#,
            r#"include!(concat!(env!("OUT_DIR"), "/custom_", stringify!($name), ".rs"));"#,
        ] {
            assert_eq!(includes_foreign_file(source), Some(true), "{source}");
        }
    }

    #[test]
    fn this_compilers_own_includes_do_not_prevent_reuse() {
        for source in [
            r#"include!(env!("FUSOR_MODULE"));"#,
            r#"macro_rules! bindings { ($name:ident) => { include!(concat!(env!("OUT_DIR"), "/fusor_", stringify!($name), ".rs")); }; }"#,
            r#"macro_rules! template { ($path:literal) => { include!(concat!(env!("OUT_DIR"), "/fusor_templates/", $path, ".rs")); }; }"#,
            "// include_str!(\"file\")\nfn f() {}",
            r#"const EXAMPLE: &str = "include_str!(file)";"#,
        ] {
            assert_eq!(includes_foreign_file(source), Some(false), "{source}");
        }
    }

    #[test]
    fn source_that_does_not_tokenize_has_no_answer() {
        assert_eq!(includes_foreign_file("fn f() { '"), None);
    }

    #[test]
    fn only_direct_literal_data_includes_can_be_cleared_by_the_cli() {
        for source in [
            r#"const DATA: &str = include_str!("data.txt");"#,
            r#"fn data() { std::include_bytes!(r"data.txt"); }"#,
            r#"mod nested { const DATA: &str = include_str!("data.txt"); }"#,
        ] {
            assert_eq!(
                includes_foreign_file_with(source, |path| path == "data.txt"),
                Some(false),
                "{source}"
            );
            assert_eq!(includes_foreign_file(source), Some(true), "{source}");
        }
    }

    #[test]
    fn uncertain_include_origins_cannot_be_cleared_by_the_cli() {
        for source in [
            r#"include!("data.txt");"#,
            r#"include_str!(concat!("data", ".txt"));"#,
            r#"include_bytes!($path);"#,
            r#"macro_rules! data { () => { include_str!("data.txt") } }"#,
            r#"r#macro_rules! data { () => { r#include_str!("data.txt") } }"#,
            r#"opaque! { include_str!("data.txt") }"#,
            r#"#[opaque(include_str!("data.txt"))] fn data() {}"#,
            r#"#![opaque(include_str!("data.txt"))]"#,
        ] {
            assert_eq!(
                includes_foreign_file_with(source, |_| true),
                Some(true),
                "{source}"
            );
        }
    }
}
