//! Lower a native element's attributes: directives become bindings, and the
//! opening tag is rewritten with the element's ID and managed-content marker.
use crate::bindings::bind;
use crate::bindings::interpolation::{exact_expression, interpolations, string};
use crate::bindings::ir::{Binding, Component, Element, RenderTarget};
use crate::bindings::markup;
use crate::bindings::tags::{text_only_element, void_element};
use crate::bindings::tokens::Rust;
use crate::{ExtractError, error};
use fusor::template::{self, ChildPolicy, ComponentId, ElementId, InputKind};
use html5gum::StartTag;
use std::collections::BTreeMap;

fn boolean_attribute(name: &str) -> bool {
    matches!(
        name,
        "allowfullscreen"
            | "async"
            | "autofocus"
            | "autoplay"
            | "controls"
            | "default"
            | "defer"
            | "disabled"
            | "formnovalidate"
            | "hidden"
            | "inert"
            | "ismap"
            | "itemscope"
            | "loop"
            | "multiple"
            | "muted"
            | "nomodule"
            | "novalidate"
            | "open"
            | "playsinline"
            | "readonly"
            | "required"
            | "reversed"
            | "selected"
    )
}

/// What an attribute name means to the compiler.
#[derive(Clone, Copy)]
enum Directive<'a> {
    /// `rust:component`; the traversal reads its type.
    Component,
    Render,
    ActivationTarget,
    /// `hydrate` and every `hydrate:*` except `hydrate:target` belong on a component tag.
    Hydrate,
    /// `rust:async` and `rust:await` open a coherent region.
    Region,
    /// `rust:slot`, `rust:if` and `rust:key` are lowered together after the other attributes.
    Slot,
    Property(&'a str),
    Event(&'a str),
    Bind,
    Class(&'a str),
    UnknownRust,
    Plain,
}

impl<'a> Directive<'a> {
    fn classify(name: &'a str) -> Self {
        match name {
            "rust:component" => Self::Component,
            "rust:render" => Self::Render,
            "hydrate:target" => Self::ActivationTarget,
            "rust:async" | "rust:await" => Self::Region,
            "rust:key" | "rust:slot" | "rust:if" => Self::Slot,
            "bind" => Self::Bind,
            _ if name == "hydrate" || name.starts_with("hydrate:") => Self::Hydrate,
            _ => {
                if let Some(property) = name.strip_prefix("prop:") {
                    Self::Property(property)
                } else if let Some(event) = name.strip_prefix("on:") {
                    Self::Event(event)
                } else if let Some(class) = name.strip_prefix("class:") {
                    Self::Class(class)
                } else if name.starts_with("rust:") {
                    Self::UnknownRust
                } else {
                    Self::Plain
                }
            }
        }
    }
}

pub(super) fn is_directive(name: &str) -> bool {
    !matches!(Directive::classify(name), Directive::Plain)
}

/// Attribute values keyed by name, each with where the value starts in the source.
type Attributes = BTreeMap<String, (String, usize)>;

/// html5gum lowercases attribute names. Property and event names are case
/// sensitive, so read their authored spelling back from the source.
fn authored_attributes(source: &str, tag: &StartTag<usize>) -> Attributes {
    tag.attributes
        .iter()
        .map(|(key, value)| {
            (
                if key.starts_with(b"prop:") || key.starts_with(b"on:") {
                    let authored = source[value.span.start..]
                        .split(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '=' | '/' | '>'))
                        .next()
                        .unwrap_or("");
                    let prefix = if key.starts_with(b"prop:") {
                        "prop"
                    } else {
                        "on"
                    };
                    format!(
                        "{prefix}:{}",
                        authored.split_once(':').map_or("", |(_, suffix)| suffix)
                    )
                } else {
                    String::from_utf8_lossy(key).into_owned()
                },
                (
                    String::from_utf8_lossy(value).into_owned(),
                    crate::html::value_start(source, value.span.start),
                ),
            )
        })
        .collect()
}

/// The element being lowered.
struct Native<'a> {
    source: &'a str,
    name: &'a str,
    attrs: &'a Attributes,
    node: ElementId,
}

/// The rewritten opening tag being built.
struct Opening {
    rendered: String,
    /// The authored opening tag must be replaced.
    changed: bool,
    /// The element carries bindings, so it needs an element ID and descriptor.
    bound: bool,
}

// Appends at most one element descriptor. Traversal records the returned edit
// before selecting a TextHost, whose opening_edit indexes that edit.
pub(super) fn lower(
    source: &str,
    tag: &StartTag<usize>,
    component: &mut Component,
    node: ElementId,
    component_id: Option<ComponentId>,
    foreach_host: bool,
    async_root: bool,
) -> Result<Option<String>, ExtractError> {
    let name = String::from_utf8_lossy(&tag.name);
    let attrs = authored_attributes(source, tag);
    let native = Native {
        source,
        name: &name,
        attrs: &attrs,
        node,
    };
    let mut opening = Opening {
        rendered: String::new(),
        changed: component_id.is_some() || foreach_host || async_root,
        bound: foreach_host || async_root,
    };
    for (attr, (value, offset)) in &attrs {
        lower_attribute(
            &native,
            attr,
            value,
            *offset,
            component,
            component_id,
            &mut opening,
        )?;
    }
    lower_slot(&native, tag, component, &mut opening)?;
    if !opening.changed {
        return Ok(None);
    }
    Ok(Some(render_opening(
        &native,
        tag,
        component,
        component_id,
        foreach_host,
        opening,
    )))
}

fn lower_attribute(
    native: &Native,
    attr: &str,
    value: &str,
    offset: usize,
    component: &mut Component,
    component_id: Option<ComponentId>,
    opening: &mut Opening,
) -> Result<(), ExtractError> {
    let source = native.source;
    let binding = match Directive::classify(attr) {
        Directive::Component => return Ok(()),
        Directive::Render => {
            return render_target(native, value, offset, component, component_id, opening);
        }
        Directive::ActivationTarget => return activation_target(native, value, offset, opening),
        Directive::Hydrate => {
            return Err(error(
                source,
                offset,
                "hydrate belongs on a component tag; only hydrate:target belongs on a native button",
            ));
        }
        Directive::Region => return region(native, value, offset, opening),
        Directive::Slot => {
            opening.changed = true;
            return Ok(());
        }
        Directive::Property(property) => Some(property_binding(native, property, value, offset)?),
        Directive::Event(event) => {
            if event.is_empty() || value.trim().is_empty() {
                return Err(error(source, offset, "on:event requires a Rust handler"));
            }
            Some(Binding::Event {
                node: native.node,
                name: event.to_owned(),
                handler: Rust::parse(source, value, offset)?,
            })
        }
        Directive::Bind => Some(bind::parse(
            source,
            native.name,
            native.attrs,
            native.node,
            value,
            offset,
        )?),
        Directive::Class(class) => Some(class_binding(native, class, value, offset)?),
        Directive::UnknownRust => {
            return Err(error(
                source,
                offset,
                format!("unknown Rust directive {attr:?}"),
            ));
        }
        Directive::Plain => plain_attribute(native, attr, value, offset, &mut opening.rendered)?,
    };
    if let Some(binding) = binding {
        component.bindings.push(binding);
        opening.changed = true;
        opening.bound = true;
    }
    Ok(())
}

fn render_target(
    native: &Native,
    value: &str,
    offset: usize,
    component: &mut Component,
    component_id: Option<ComponentId>,
    opening: &mut Opening,
) -> Result<(), ExtractError> {
    let source = native.source;
    if component.app().is_some() {
        return Err(error(
            source,
            offset,
            "App is a browser startup boundary; use rust:component templates for native or shared rendering",
        ));
    }
    if component_id.is_none() {
        return Err(error(
            source,
            offset,
            "rust:render belongs on a component declaration",
        ));
    }
    component.render = match value {
        "server" => RenderTarget::Server,
        "shared" => RenderTarget::Shared,
        _ => {
            return Err(error(
                source,
                offset,
                "rust:render must be server or shared",
            ));
        }
    };
    opening.changed = true;
    Ok(())
}

fn activation_target(
    native: &Native,
    value: &str,
    offset: usize,
    opening: &mut Opening,
) -> Result<(), ExtractError> {
    if native.name != "button"
        || value.trim().is_empty()
        || value.contains("{{")
        || native
            .attrs
            .get("type")
            .is_none_or(|(value, _)| !value.eq_ignore_ascii_case("button"))
    {
        return Err(error(
            native.source,
            offset,
            "hydrate:target requires an explicit instance ID on a native type=button button",
        ));
    }
    opening.rendered.push_str(&markup::activation_target(value));
    opening.changed = true;
    Ok(())
}

fn region(
    native: &Native,
    value: &str,
    offset: usize,
    opening: &mut Opening,
) -> Result<(), ExtractError> {
    if value.trim().is_empty() || void_element(native.name) || native.name == "template" {
        return Err(error(
            native.source,
            offset,
            "rust:async and rust:await require an expression and an ordinary HTML region",
        ));
    }
    opening.changed = true;
    opening.bound = true;
    Ok(())
}

fn property_binding(
    native: &Native,
    property: &str,
    value: &str,
    offset: usize,
) -> Result<Binding, ExtractError> {
    let source = native.source;
    if !native.name.contains('-') || property.is_empty() || value.trim().is_empty() {
        return Err(error(
            source,
            offset,
            "prop:name requires a custom HTML element and a Rust value",
        ));
    }
    if matches!(
        property,
        "innerHTML" | "outerHTML" | "textContent" | "innerText" | "outerText"
    ) || property.starts_with("on")
    {
        return Err(error(
            source,
            offset,
            "prop:name cannot replace owned HTML or event handlers; use HTML children and on:event",
        ));
    }
    Ok(Binding::Property {
        node: native.node,
        name: property.to_owned(),
        value: Rust::parse(source, value, offset)?,
    })
}

fn class_binding(
    native: &Native,
    class: &str,
    value: &str,
    offset: usize,
) -> Result<Binding, ExtractError> {
    let source = native.source;
    if class.is_empty() || class.chars().any(char::is_whitespace) {
        return Err(error(
            source,
            offset,
            "class:name requires a single CSS class name",
        ));
    }
    if native
        .attrs
        .get("class")
        .is_some_and(|(value, _)| value.contains("{{"))
    {
        return Err(error(
            source,
            offset,
            "use a static class attribute with class:name bindings",
        ));
    }
    Ok(Binding::Class {
        node: native.node,
        name: class.to_owned(),
        value: Rust::parse(source, value, offset)?,
    })
}

/// A static attribute is rendered as written; an interpolated one becomes a binding.
fn plain_attribute(
    native: &Native,
    attr: &str,
    value: &str,
    offset: usize,
    rendered: &mut String,
) -> Result<Option<Binding>, ExtractError> {
    let source = native.source;
    let node = native.node;
    let parts = interpolations(source, value, offset, false)?;
    if parts.is_empty() {
        rendered.push_str(&format!(" {attr}=\"{}\"", markup::escape_attribute(value)));
        return Ok(None);
    }
    if attr.starts_with("on") || attr == "srcdoc" {
        return Err(error(
            source,
            offset,
            "interpolation cannot create executable HTML; use on:event with a Rust handler",
        ));
    }
    if boolean_attribute(attr) || attr == "checked" {
        let value = exact_expression(
            source,
            value,
            parts,
            offset,
            "this property requires exactly one {{ Rust boolean expression }}",
        )?;
        if attr == "checked" {
            if native.name != "input" {
                return Err(error(source, offset, "checked requires an <input>"));
            }
            return Ok(Some(Binding::Checked { node, value }));
        }
        return Ok(Some(Binding::Boolean {
            node,
            name: attr.to_owned(),
            value,
        }));
    }
    let value = string(value, parts, offset);
    // A checkbox or radio's value is the choice it submits, not edited text.
    let choice = native.attrs.get("type").is_some_and(|(kind, _)| {
        matches!(InputKind::of(kind), InputKind::Checkbox | InputKind::Radio)
    });
    Ok(Some(
        if attr == "value" && native.name == "input" && !choice {
            Binding::Value { node, value }
        } else {
            Binding::Attribute {
                node,
                name: attr.to_owned(),
                value,
            }
        },
    ))
}

/// `rust:slot` hands the element's children to a Rust constructor; `rust:if` and
/// `rust:key` only apply to it.
fn lower_slot(
    native: &Native,
    tag: &StartTag<usize>,
    component: &mut Component,
    opening: &mut Opening,
) -> Result<(), ExtractError> {
    let source = native.source;
    let name = native.name;
    let attrs = native.attrs;
    if let Some(constructor) = attrs.get("rust:slot") {
        if void_element(name) || text_only_element(name) || matches!(name, "select" | "option") {
            return Err(error(
                source,
                tag.span.start,
                "rust:slot requires an ordinary HTML container",
            ));
        }
        let expression = |(code, offset): &(String, usize)| Rust::parse(source, code, *offset);
        let value = expression(constructor)?;
        let condition = attrs.get("rust:if").map(expression).transpose()?;
        let key = attrs.get("rust:key").map(expression).transpose()?;
        component.bindings.push(Binding::Slot {
            node: native.node,
            content: value,
            condition,
            key,
        });
        opening.bound = true;
    } else {
        if attrs.contains_key("rust:if") {
            return Err(error(
                source,
                tag.span.start,
                "rust:if requires rust:slot on the same element",
            ));
        }
        if attrs.contains_key("rust:key") {
            return Err(error(
                source,
                tag.span.start,
                "rust:key requires a component mount or slot; use the key input on ForEach for lists",
            ));
        }
    }
    Ok(())
}

fn render_opening(
    native: &Native,
    tag: &StartTag<usize>,
    component: &mut Component,
    component_id: Option<ComponentId>,
    foreach_host: bool,
    opening: Opening,
) -> String {
    let Native {
        name, attrs, node, ..
    } = *native;
    let mut rendered = opening.rendered;
    if opening.bound {
        let managed = foreach_host || attrs.contains_key("rust:slot");
        rendered.push_str(&format!(" {}=\"{node}\"", template::ELEMENT_ATTRIBUTE));
        if managed {
            rendered.push_str(&format!(" {}=\"\"", template::MANAGED_ATTRIBUTE));
        }
        component.elements.push(Element {
            id: node,
            tag: name.to_string(),
            children: if managed {
                ChildPolicy::Managed
            } else {
                ChildPolicy::Static
            },
        });
    }
    if let Some(id) = component_id {
        rendered.push_str(&markup::component_attributes(id));
    }
    format!(
        "<{name}{rendered}{}>",
        if tag.self_closing { " /" } else { "" }
    )
}
