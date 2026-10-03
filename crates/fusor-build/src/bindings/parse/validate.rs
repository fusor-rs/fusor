use crate::bindings::ir::{Binding, Component, RenderTarget};
use crate::{ExtractError, error};
use std::collections::BTreeMap;

// Validation consumes authored bindings and root counts; it does not rewrite IR.
pub(super) fn components(
    source: &str,
    components: &[Component],
    template_roots: &BTreeMap<usize, usize>,
) -> Result<(), ExtractError> {
    if let Some(app) = components
        .iter()
        .filter_map(|component| component.app())
        .nth(1)
    {
        return Err(error(
            source,
            app.offset,
            "declare exactly one App boundary per application",
        ));
    }
    for (&index, &roots) in template_roots {
        if roots != 1 {
            return Err(error(
                source,
                components[index].ty.offset,
                "a component template requires exactly one root element (Rust script blocks do not count)",
            ));
        }
    }
    for component in components {
        render_bindings(source, component)?;
    }
    for component in components.iter().filter(|component| !component.fragment()) {
        let placements = count_children(&component.bindings, components);
        if component.app().is_some() && placements > 0 {
            return Err(error(
                source,
                component.ty.offset,
                "App has no incoming Children; use Children in a reusable component",
            ));
        }
        if placements > 1 {
            return Err(error(
                source,
                component.ty.offset,
                "a component can place Children only once, including forwarded children",
            ));
        }
    }
    Ok(())
}

fn render_bindings(source: &str, component: &Component) -> Result<(), ExtractError> {
    for binding in &component.bindings {
        if component.render == RenderTarget::Shared
            && matches!(binding, Binding::Value { .. } | Binding::Checked { .. })
        {
            return Err(error(
                source,
                binding.origin().offset,
                "shared templates require bind for editable controls so activation can adopt native edits",
            ));
        }
        if component.render == RenderTarget::Browser && matches!(binding, Binding::Island { .. }) {
            return Err(error(
                source,
                binding.origin().offset,
                super::HYDRATE_SERVER,
            ));
        }
        if component.render != RenderTarget::Browser
            && matches!(
                binding,
                Binding::Region { .. }
                    | Binding::Property { .. }
                    | Binding::Slot { .. }
                    | Binding::Router { .. }
            )
        {
            return Err(error(
                source,
                binding.origin().offset,
                "server templates need resolved data; move async regions, widgets, opaque content and outlets into a browser preview",
            ));
        }
    }
    Ok(())
}

fn count_children(bindings: &[Binding], components: &[Component]) -> usize {
    let count = |child: usize| count_children(&components[child].bindings, components);
    Binding::walk(bindings)
        .into_iter()
        .map(|binding| match binding {
            Binding::Children { .. } => 1,
            // Cases and sibling routes are exclusive placements of the caller's children.
            Binding::Branch { .. } | Binding::Router { .. } => binding
                .components()
                .into_iter()
                .map(count)
                .max()
                .unwrap_or(0),
            Binding::Invocation { .. } => binding.components().into_iter().map(count).sum(),
            _ => 0,
        })
        .sum()
}
