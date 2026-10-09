use crate::bindings::ir::{Binding, Component, ComponentShape, RenderTarget};
use crate::{ExtractError, error};
use fusor::template::RootKind;
use std::collections::BTreeMap;

pub(super) fn roots(
    source: &str,
    components: &mut [Component],
    template_roots: &BTreeMap<usize, usize>,
) -> Result<(), ExtractError> {
    for (&index, &roots) in template_roots {
        let component = &mut components[index];
        if matches!(
            component.shape,
            ComponentShape::Declared(RootKind::Template)
        ) {
            if roots == 0 {
                return Err(error(
                    source,
                    component.ty.offset,
                    "a component template requires at least one native root element (Rust script blocks do not count)",
                ));
            }
            if roots > 1 {
                component.shape = ComponentShape::Declared(RootKind::Fragment);
            }
        } else if roots != 1 {
            return Err(error(
                source,
                component.ty.offset,
                "a component template requires exactly one root element (Rust script blocks do not count)",
            ));
        }
    }
    Ok(())
}

pub(super) fn components(source: &str, components: &[Component]) -> Result<(), ExtractError> {
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
    for component in components {
        render_bindings(source, component)?;
    }
    for component in components
        .iter()
        .filter(|component| !matches!(component.shape, ComponentShape::Fragment(_)))
    {
        let placements = count_children(&component.bindings, components);
        if component.app().is_some() && !placements.is_empty() {
            return Err(error(
                source,
                component.ty.offset,
                "App has no incoming Children; use Children in a reusable component",
            ));
        }
        if let Some((name, _)) = placements.iter().find(|(_, count)| **count > 1) {
            return Err(error(
                source,
                component.ty.offset,
                format!(
                    "a component can place Children slot {} only once, including forwarded children",
                    name.unwrap_or("<default>")
                ),
            ));
        }
    }
    Ok(())
}

fn render_bindings(source: &str, component: &Component) -> Result<(), ExtractError> {
    if component.fragment() && component.javascript.is_some() {
        return Err(error(
            source,
            component.ty.offset,
            "JavaScript component modules require one native root element; wrap this component's roots",
        ));
    }
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

fn count_children<'a>(
    bindings: &'a [Binding],
    components: &'a [Component],
) -> BTreeMap<Option<&'a str>, usize> {
    let mut placements = BTreeMap::new();
    for binding in Binding::walk(bindings) {
        for (name, count) in binding_children(binding, components) {
            *placements.entry(name).or_insert(0) += count;
        }
    }
    placements
}

fn binding_children<'a>(
    binding: &'a Binding,
    components: &'a [Component],
) -> BTreeMap<Option<&'a str>, usize> {
    let mut placements = BTreeMap::new();
    if let Binding::Children { name, .. } = binding {
        placements.insert(name.as_deref(), 1);
        return placements;
    }
    if !matches!(
        binding,
        Binding::Branch { .. } | Binding::Router { .. } | Binding::Invocation { .. }
    ) {
        return placements;
    }
    for child in binding.components() {
        for (name, count) in count_children(&components[child].bindings, components) {
            let total = placements.entry(name).or_insert(0);
            *total = if matches!(binding, Binding::Invocation { .. }) {
                *total + count
            } else {
                (*total).max(count)
            };
        }
    }
    placements
}
