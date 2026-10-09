//! Component invocations use Rust paths and ordinary named-field input structs.
use super::{
    interpolation::{exact_expression, interpolations},
    ir::*,
    tokens::Rust,
};
use crate::{ExtractError, error};
use fusor::template::MountId;
use html5gum::StartTag;
use quote::quote;

// html5gum intentionally folds HTML tag names. Read only the original name
// from the token's exact source range; Rust paths still go through syn/rustc.
pub(super) fn name(source: &str, offset: usize) -> &str {
    source[offset..]
        .trim_start_matches('<')
        .trim_start_matches('/')
        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '>' | '/'))
        .next()
        .unwrap_or("")
}

/// Declare the built-in tags once; each variant's name is its exact spelling.
macro_rules! built_ins {
    ($($tag:ident),* $(,)?) => {
        /// A tag the compiler implements itself. HTML folds tag names to lowercase, so
        /// a built-in is recognized by that name and must be written with its spelling.
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub(super) enum BuiltIn {
            $($tag),*
        }

        impl BuiltIn {
            const SPELLINGS: &[(Self, &'static str)] = &[$((Self::$tag, stringify!($tag))),*];
        }
    };
}

built_ins!(
    App, If, Else, Match, Case, ForEach, Async, Await, Children, Router, Route
);

impl BuiltIn {
    pub fn classify(name: &str) -> Option<Self> {
        Self::SPELLINGS
            .iter()
            .find(|(_, spelling)| spelling.eq_ignore_ascii_case(name))
            .map(|&(builtin, _)| builtin)
    }

    pub fn spelling(self) -> &'static str {
        Self::SPELLINGS
            .iter()
            .find(|(builtin, _)| *builtin == self)
            .map(|&(_, spelling)| spelling)
            .expect("every built-in has a spelling")
    }
}

const NATIVE_TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "basefont",
    "bdi",
    "bdo",
    "bgsound",
    "big",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "listing",
    "main",
    "map",
    "mark",
    "marquee",
    "math",
    "menu",
    "meta",
    "meter",
    "nav",
    "nobr",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "plaintext",
    "pre",
    "progress",
    "q",
    "rb",
    "rp",
    "rt",
    "rtc",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
    "xmp",
];

pub(super) fn is_component(name: &str) -> bool {
    if name.contains("::") {
        return true;
    }
    if name.contains('-') || !name.chars().next().is_some_and(char::is_uppercase) {
        return false;
    }
    // Preserve conventional ALL-UPPERCASE native HTML. PascalCase, including
    // Button and Input, is always a Rust component; lowercase tags remain HTML.
    if name.chars().any(char::is_lowercase) {
        return true;
    }
    !NATIVE_TAGS.contains(&name.to_ascii_lowercase().as_str())
}

pub(super) fn snake_case_ident(name: &str) -> bool {
    name.bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && syn::parse_str::<syn::Ident>(name).is_ok()
}

pub(super) fn field(source: &str, name: &str, offset: usize) -> Result<Rust, ExtractError> {
    if !snake_case_ident(name) {
        return Err(error(
            source,
            offset,
            "component input names must be snake_case Rust field identifiers",
        ));
    }
    Rust::parse(source, name, offset)
}

pub(super) fn invocation(
    source: &str,
    tag: &StartTag<usize>,
    point: MountId,
    hydrated: bool,
) -> Result<Binding, ExtractError> {
    let ty = component_type(source, tag)?;
    let mut inputs = Vec::new();
    let mut condition = None;
    let mut key = None;
    for (name, value) in &tag.attributes {
        let name = String::from_utf8_lossy(name);
        let value_text = String::from_utf8_lossy(value);
        let offset = value.span.start;
        let authored = source[offset..]
            .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '=' | '/' | '>'))
            .next()
            .unwrap_or("");
        if authored != name.as_ref() {
            return Err(error(
                source,
                offset,
                "component input attributes must use their exact snake_case Rust spelling",
            ));
        }
        let value_offset = crate::html::value_start(source, offset);
        if name == "hydrate" || name.starts_with("hydrate:") {
            if hydrated && super::hydration::ATTRIBUTES.contains(&name.as_ref()) {
                continue;
            }
            return Err(error(source, offset, super::hydration::misplaced(&name)));
        }
        match name.as_ref() {
            "rust:if" => condition = Some(Rust::parse(source, &value_text, value_offset)?),
            "rust:key" => key = Some(Rust::parse(source, &value_text, value_offset)?),
            _ => {
                let name = field(source, &name, offset)?;
                let value = input_value(source, &value_text, value_offset)?;
                inputs.push(Input { name, value });
            }
        }
    }
    Ok(Binding::Invocation {
        children: Vec::new(),
        point,
        ty,
        inputs,
        condition,
        key,
    })
}

fn component_type(source: &str, tag: &StartTag<usize>) -> Result<Rust, ExtractError> {
    let name = name(source, tag.span.start);
    let path = syn::parse_str::<syn::Path>(name).map_err(|_| {
        error(
            source,
            tag.span.start,
            "component tags require a Rust type path or imported alias",
        )
    })?;
    if path
        .segments
        .iter()
        .any(|part| !matches!(part.arguments, syn::PathArguments::None))
    {
        return Err(error(
            source,
            tag.span.start,
            "use a Rust type alias for a generic component",
        ));
    }
    Rust::parse(source, name, tag.span.start)
}

fn input_value(
    source: &str,
    value_text: &str,
    value_offset: usize,
) -> Result<InputValue, ExtractError> {
    let parts = interpolations(source, value_text, value_offset, false)?;
    Ok(if parts.is_empty() {
        InputValue::Literal(Rust::synthetic(quote! { #value_text }, value_offset))
    } else {
        literal_or_expression(exact_expression(
            source,
            value_text,
            parts,
            value_offset,
            "component inputs require a literal string or exactly one {{ Rust value }}; use format! explicitly for formatted strings",
        )?)
    })
}

/// `{{ "text" }}` is written like an expression but is a string literal input.
fn literal_or_expression(value: Rust) -> InputValue {
    if syn::parse2::<syn::LitStr>(value.tokens.clone()).is_ok() {
        InputValue::Literal(value)
    } else {
        InputValue::Expression(value)
    }
}

pub(super) fn void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// The parser reads these elements' contents as text, never as markup (the
/// RAWTEXT, RCDATA and PLAINTEXT states), so bindings cannot live inside them.
pub(super) fn text_only_element(name: &str) -> bool {
    matches!(
        name,
        "script"
            | "style"
            | "textarea"
            | "title"
            | "xmp"
            | "iframe"
            | "noembed"
            | "noframes"
            | "plaintext"
    )
}

/// SVG and MathML switch the parser into foreign content.
pub(super) fn foreign_element(name: &str) -> bool {
    matches!(name, "svg" | "math")
}

/// Table structure only accepts table content; the parser relocates anything else.
pub(super) fn table_structure(name: &str) -> bool {
    matches!(name, "table" | "tbody" | "thead" | "tfoot" | "tr")
}

pub(super) fn reserved_scope_name(name: &str) -> bool {
    matches!(name, "state" | "owner" | "event" | "ready") || name.starts_with("__fusor")
}
