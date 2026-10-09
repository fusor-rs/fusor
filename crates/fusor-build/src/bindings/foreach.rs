//! Structural list syntax and item-only forwarding-row analysis.
use super::{ir::*, tag_input::TagInput, tags::BuiltIn, tokens::Rust};
use crate::{ExtractError, error};
use html5gum::{DefaultEmitter, Token, Tokenizer};
use std::collections::BTreeSet;

/// A ForEach owns its native parent's children, without adding a wrapper node.
/// Validate this before lowering so static siblings cannot silently disappear.
pub(super) fn hosts(source: &str) -> Result<BTreeSet<usize>, ExtractError> {
    let mut emitter = DefaultEmitter::<usize>::new_with_span();
    emitter.naively_switch_states(true);
    let mut stack: Vec<Frame> = Vec::new();
    let mut hosts = BTreeSet::new();
    for token in Tokenizer::new_with_emitter(source, emitter) {
        match token.expect("in-memory HTML") {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).into_owned();
                let list = super::tags::name(source, tag.span.start) == BuiltIn::ForEach.spelling();
                if BuiltIn::classify(&name) == Some(BuiltIn::ForEach) && !list {
                    return Err(error(
                        source,
                        tag.span.start,
                        "the built-in component is spelled ForEach",
                    ));
                }
                if list
                    && stack.iter().any(|frame| {
                        super::tags::foreign_element(&frame.name)
                            || matches!(frame.name.as_str(), "select" | "option")
                    })
                {
                    return Err(error(
                        source,
                        tag.span.start,
                        "ForEach requires ordinary HTML outside SVG, MathML and select controls",
                    ));
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children += 1;
                    parent.list |= list;
                }
                if !tag.self_closing && !super::tags::void_element(&name) {
                    stack.push(Frame {
                        name,
                        start: tag.span.start,
                        children: 0,
                        list: false,
                        text: false,
                    });
                }
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name);
                if let Some(index) = stack.iter().rposition(|frame| frame.name == name) {
                    let frame = &stack[index];
                    hosts.extend(frame.list_host(source)?);
                    stack.truncate(index);
                }
            }
            Token::String(text) => {
                if let Some(frame) = stack.last_mut() {
                    frame.text |= !String::from_utf8_lossy(&text).trim().is_empty();
                }
            }
            _ => {}
        }
    }
    Ok(hosts)
}

pub(super) fn inputs(input: &TagInput) -> Result<(Rust, Rust, Rust, Rust), ExtractError> {
    input.closed()?;
    input.accepts(
        &["items", "key", "item", "index"],
        "items, key, and optional item and index names",
    )?;
    let item = input.binding_or("item", "item")?;
    let index = input.binding_or("index", "index")?;
    if item.same_tokens(&index) {
        return Err(input.error("ForEach item and index names must differ"));
    }
    Ok((
        input.expression("items")?,
        input.expression("key")?,
        item,
        index,
    ))
}

// This proof intentionally covers only the existing direct component-forwarding
// row lowering. Descendant content and nested lexical rows retain the full Row.
pub(super) fn mark_item_only_rows(components: &mut [Component]) {
    let rows: Vec<usize> = components
        .iter()
        .flat_map(|component| Binding::walk(&component.bindings))
        .filter_map(|binding| match binding {
            Binding::ForEach { body, .. } => Some(*body),
            _ => None,
        })
        .collect();
    for row in rows {
        let item_only = forwards_item_only(&components[row], components);
        components[row].item_only_row = item_only;
    }
}

fn forwards_item_only(component: &Component, components: &[Component]) -> bool {
    if !component.inline()
        || component.capture().is_some()
        || component.row_locals.len() != 1
        || !component.async_locals.is_empty()
        || !component.route_locals.is_empty()
        || !component.elements.is_empty()
        || !component.texts.is_empty()
        || !component.text_elements.is_empty()
    {
        return false;
    }
    let [
        binding @ Binding::Invocation {
            inputs,
            children,
            condition: None,
            key: None,
            ..
        },
    ] = component.bindings.as_slice()
    else {
        return false;
    };
    // The parser retains an empty Children fragment for explicit closing tags.
    // children_factory discards it. No descendant code is evaluated or captured.
    if children.iter().any(|slot| {
        let child = &components[slot.body];
        !child.empty || !child.bindings.is_empty()
    }) {
        return false;
    }
    if inputs
        .iter()
        .any(|input| matches!(input.value, InputValue::Content { .. }))
    {
        return false;
    }
    let index = component.row_locals[0].1.tokens.to_string();
    let index = index.strip_prefix("r#").unwrap_or(&index);
    fn independent(tokens: proc_macro2::TokenStream, index: &str) -> bool {
        tokens.into_iter().all(|token| match token {
            proc_macro2::TokenTree::Group(group) => independent(group.stream(), index),
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let name = name.strip_prefix("r#").unwrap_or(&name);
                // Raw names compare like ordinary identifiers. Conservatively
                // avoid Unicode normalization and compiler-context escapes.
                name.is_ascii() && name != index && !name.starts_with("__fusor")
            }
            // Opaque macros/attributes may introduce a use absent from tokens.
            // Rejecting unary ! and != too is an intentional false positive.
            proc_macro2::TokenTree::Punct(punct) => !matches!(punct.as_char(), '!' | '#'),
            proc_macro2::TokenTree::Literal(_) => true,
        })
    }
    index.is_ascii()
        && binding
            .fragments()
            .iter()
            .all(|fragment| independent(fragment.tokens.clone(), index))
}

struct Frame {
    name: String,
    start: usize,
    children: usize,
    list: bool,
    text: bool,
}

impl Frame {
    fn list_host(&self, source: &str) -> Result<Option<usize>, ExtractError> {
        if !self.list {
            return Ok(None);
        }
        if self.children != 1
            || self.text
            || super::tags::is_component(super::tags::name(source, self.start))
            || self.name == "template"
        {
            return Err(error(
                source,
                self.start,
                "ForEach requires its own native HTML container; put other content outside that container",
            ));
        }
        Ok(Some(self.start))
    }
}
