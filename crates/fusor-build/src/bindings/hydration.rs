//! Component-tag authoring for the existing typed delivery protocol.
use super::{ir::*, tag_input::TagInput, tags};
use crate::ExtractError;
use fusor::template::{self, ChildPolicy, ElementId, MountId};
use html5gum::StartTag;

/// The attributes a hydrated component tag reads itself; the others are inputs.
pub(super) const ATTRIBUTES: [&str; 3] = ["hydrate", "hydrate:id", "hydrate:prefetch"];

/// Why a `hydrate*` attribute cannot be used where it was written.
pub(super) fn misplaced(name: &str) -> String {
    match name {
        "hydrate:target" => {
            "hydrate:target belongs on the native button that activates an island".into()
        }
        _ if ATTRIBUTES.contains(&name) => {
            format!("{name} requires hydrate on the same component tag")
        }
        _ => format!(
            "unknown attribute {name}; component tags accept {}",
            ATTRIBUTES.join(", ")
        ),
    }
}

pub(super) fn lower(
    source: &str,
    tag: &StartTag<usize>,
    node: ElementId,
    component: &mut Component,
) -> Result<String, ExtractError> {
    let name = tags::name(source, tag.span.start);
    let input = TagInput::new(source, tag, name);
    if !tags::is_component(name) || tag.self_closing {
        return Err(
            input.error("hydrate requires a Rust component tag with an explicit closing tag")
        );
    }
    let activation =
        policy(&input, "hydrate", &Activation::ALL, Activation::as_str)?.expect("hydrate present");
    let prefetch =
        policy(&input, "hydrate:prefetch", &Prefetch::ALL, Prefetch::as_str)?.unwrap_or_default();
    let id = instance_id(&input, activation)?;
    let Binding::Invocation {
        ty,
        inputs,
        condition,
        key,
        ..
    } = tags::invocation(source, tag, MountId::new(0), true)?
    else {
        unreachable!("tags::invocation returns an invocation")
    };
    if condition.is_some() || key.is_some() {
        return Err(input.error(
            "hydrated components cannot use rust:if or rust:key; their lifetime belongs to the server-rendered page",
        ));
    }
    component.bindings.push(Binding::Island {
        node,
        descriptor: ty,
        inputs,
        activation,
        prefetch,
    });
    component.elements.push(Element {
        id: node,
        tag: "div".into(),
        children: ChildPolicy::Managed,
    });
    let id = id
        .map(|(id, _)| format!(" id=\"{}\"", super::markup::escape_attribute(&id)))
        .unwrap_or_default();
    Ok(format!(
        "<div{id} {}=\"{node}\" {}=\"\">",
        template::ELEMENT_ATTRIBUTE,
        template::MANAGED_ATTRIBUTE
    ))
}

fn instance_id(
    input: &TagInput,
    activation: Activation,
) -> Result<Option<(String, usize)>, ExtractError> {
    let id = input.text("hydrate:id");
    if let Some((id, offset)) = &id {
        if id.trim().is_empty() || id.contains("{{") {
            return Err(
                input.error_at(*offset, "hydrate:id requires a nonempty static instance ID")
            );
        }
    }
    if activation == Activation::Interaction && id.is_none() {
        return Err(input.error(
            "hydrate=\"interaction\" requires hydrate:id and a native button with the matching hydrate:target",
        ));
    }
    Ok(id)
}

/// A hydration policy attribute: one of `all`, spelled as the islands runtime spells it.
fn policy<T: Copy>(
    input: &TagInput,
    name: &str,
    all: &[T],
    as_str: fn(T) -> &'static str,
) -> Result<Option<T>, ExtractError> {
    let Some((value, offset)) = input.text(name) else {
        return Ok(None);
    };
    let found = all.iter().copied().find(|policy| as_str(*policy) == value);
    found.map(Some).ok_or_else(|| {
        let names: Vec<_> = all.iter().map(|policy| as_str(*policy)).collect();
        input.error_at(
            offset,
            format!("{name} must be one of {}", names.join(", ")),
        )
    })
}
