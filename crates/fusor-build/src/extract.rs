//! Split an HTML page into its line-preserved Rust module and its runtime HTML.
use crate::{ExternalRust, ExtractError, Page, RustBlock, bindings, error, html};
use html5gum::{StartTag, Token};

/// Extract every `text/rust` script body, in document order, without interpreting
/// Rust, decoding HTML entities in code, or serializing it through an HTML DOM.
/// Standard HTML script-content rules apply, including the `</script>` delimiter.
pub fn extract(source: &str) -> Result<Page, ExtractError> {
    let page = extract_from(source, 0, false)?;
    // Without a script there is nowhere to declare state; compile_app() allows it
    // for templates included from ordinary Rust modules.
    if page.component_count > 0 && page.blocks.is_empty() {
        return Err(error(
            source,
            0,
            "declare component state in a <script type=\"text/rust\"> block, or use compile_app() with fusor::template! in an ordinary Rust module",
        ));
    }
    Ok(page)
}

/// `first_component` continues the numbering of components from earlier sources.
pub(crate) fn extract_from(
    source: &str,
    first_component: usize,
    embedded_templates: bool,
) -> Result<Page, ExtractError> {
    let scripts = scan_scripts(source)?;
    let mut rust = line_preserved_rust(source, &scripts.blocks);
    let compiled = bindings::compile(
        source,
        &scripts.blocks,
        &mut rust,
        first_component,
        embedded_templates,
    )?;
    let mut edits = compiled.edits;
    edits.extend(scripts.blocks.iter().map(|block| bindings::Edit {
        range: block.element.clone(),
        replacement: blank_preserving_lines(&source[block.element.clone()]),
    }));
    let (mut output, mut loader_offset) = apply_edits(source, edits, scripts.loader)?;
    if !compiled.templates.is_empty() {
        html::insert_before_body_end(&mut output, &compiled.templates, loader_offset.as_mut());
    }
    Ok(Page {
        rust,
        fingerprint: compiled.fingerprint,
        html: output,
        blocks: scripts.blocks,
        locations: compiled.locations,
        loader_offset,
        component_count: compiled.component_count,
        app_offset: compiled.app_offset,
        javascript: compiled.javascript,
    })
}

struct Scripts {
    blocks: Vec<RustBlock>,
    /// Before the first Rust script, or before the outermost template holding it:
    /// a script inside template.content is inert, so it must load earlier.
    loader: Option<usize>,
}

struct UnclosedScript {
    element_start: usize,
    content_start: usize,
    external: Option<ExternalRust>,
}

fn scan_scripts(source: &str) -> Result<Scripts, ExtractError> {
    let mut blocks = Vec::new();
    let mut unclosed: Option<UnclosedScript> = None;
    let mut template_depth = 0_usize;
    let mut outermost_template = None;
    let mut loader = None;
    for token in html::tokens(source) {
        match token {
            Token::StartTag(tag) if &*tag.name == b"template" => {
                if template_depth == 0 {
                    outermost_template = Some(tag.span.start);
                }
                template_depth += 1;
            }
            Token::EndTag(tag) if &*tag.name == b"template" => {
                template_depth = template_depth.saturating_sub(1);
                if template_depth == 0 {
                    outermost_template = None;
                }
            }
            Token::StartTag(tag) if &*tag.name == b"script" && html::is_rust_script(&tag) => {
                if tag.self_closing {
                    return Err(error(
                        source,
                        tag.span.start,
                        "Rust scripts need an explicit closing </script> tag",
                    ));
                }
                unclosed = Some(UnclosedScript {
                    element_start: tag.span.start,
                    content_start: tag.span.end,
                    external: parse_external(source, &tag)?,
                });
                loader.get_or_insert(outermost_template.unwrap_or(tag.span.start));
            }
            Token::EndTag(tag) if &*tag.name == b"script" => {
                if let Some(script) = unclosed.take() {
                    let content = script.content_start..tag.span.start;
                    if script.external.is_some() && !source[content.clone()].trim().is_empty() {
                        return Err(error(
                            source,
                            script.content_start,
                            "a Rust script with src must have an empty body",
                        ));
                    }
                    blocks.push(RustBlock {
                        element: script.element_start..tag.span.end,
                        content,
                        external: script.external,
                    });
                }
            }
            Token::Error(problem) => {
                return Err(error(
                    source,
                    problem.span.start,
                    format!("invalid HTML: {}", problem.value),
                ));
            }
            _ => {}
        }
    }
    if let Some(script) = unclosed {
        return Err(error(
            source,
            script.element_start,
            "Rust script is missing its closing </script> tag",
        ));
    }
    if blocks.len() > 1 && blocks.iter().any(|block| block.external.is_some()) {
        return Err(error(
            source,
            blocks[1].element.start,
            "use one external Rust source or inline Rust blocks per HTML module; compose external code with ordinary mod and use declarations",
        ));
    }
    Ok(Scripts { blocks, loader })
}

fn parse_external(
    source: &str,
    tag: &StartTag<usize>,
) -> Result<Option<ExternalRust>, ExtractError> {
    let (src, module) = match (
        html::attribute(tag, b"src"),
        html::attribute(tag, b"rust:module"),
    ) {
        (None, None) => return Ok(None),
        (Some(src), Some(module)) => (src, module),
        _ => {
            return Err(error(
                source,
                tag.span.start,
                "external Rust scripts require both src and rust:module; declare the source through an ordinary Rust mod and add fusor::bindings!(name) inside it",
            ));
        }
    };
    if !is_relative_rust_path(&src) {
        return Err(error(
            source,
            tag.span.start,
            "src must be a nonempty local Rust file path relative to this HTML file",
        ));
    }
    if !syn::parse_str::<syn::Path>(&module).is_ok_and(|path| is_crate_path(&path)) {
        return Err(error(
            source,
            tag.span.start,
            "rust:module requires an absolute Rust module path such as crate::app",
        ));
    }
    Ok(Some(ExternalRust { src, module }))
}

fn is_relative_rust_path(src: &str) -> bool {
    !src.is_empty() && !src.starts_with('/') && !src.contains([':', '?', '#', '\\'])
}

fn is_crate_path(path: &syn::Path) -> bool {
    path.leading_colon.is_none()
        && path
            .segments
            .first()
            .is_some_and(|part| part.ident == "crate")
        && path
            .segments
            .iter()
            .all(|part| matches!(part.arguments, syn::PathArguments::None))
}

fn line_preserved_rust(source: &str, blocks: &[RustBlock]) -> String {
    let mut rust = String::new();
    let mut cursor = 0;
    for block in blocks {
        rust.push_str(&blank_preserving_lines(
            &source[cursor..block.content.start],
        ));
        rust.push_str(&source[block.content.clone()]);
        cursor = block.content.end;
    }
    rust.push_str(&blank_preserving_lines(&source[cursor..]));
    rust
}

/// Keeps line breaks and tabs, so rustc's lines and columns still point into the HTML.
fn blank_preserving_lines(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            '\n' | '\r' | '\t' => character,
            _ => ' ',
        })
        .collect()
}

fn apply_edits(
    source: &str,
    mut edits: Vec<bindings::Edit>,
    loader: Option<usize>,
) -> Result<(String, Option<usize>), ExtractError> {
    edits.sort_by_key(|edit| edit.range.start);
    let mut output = String::new();
    let mut cursor = 0;
    let mut loader_offset = None;
    for edit in edits {
        if edit.range.start < cursor {
            return Err(error(source, edit.range.start, "overlapping HTML bindings"));
        }
        if let Some(offset) =
            loader.filter(|&offset| cursor <= offset && offset <= edit.range.start)
        {
            loader_offset = Some(output.len() + offset - cursor);
        }
        output.push_str(&source[cursor..edit.range.start]);
        output.push_str(&edit.replacement);
        cursor = edit.range.end;
    }
    if let Some(offset) = loader.filter(|&offset| offset >= cursor) {
        loader_offset = Some(output.len() + offset - cursor);
    }
    output.push_str(&source[cursor..]);
    Ok((output, loader_offset))
}
