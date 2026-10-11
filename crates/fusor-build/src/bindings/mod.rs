//! Lower HTML bindings to a typed plan, then emit ordinary Rust.
mod async_tags;
mod bind;
mod codegen;
mod control;
mod emit;
mod foreach;
mod hydration;
mod interpolation;
mod ir;
mod lexical;
mod markup;
mod parse;
mod router_tags;
mod server;
mod tag_input;
mod tags;
mod tokens;

use crate::{BindingLocation, ExtractError, RustBlock};
use ir::Binding;
pub(crate) use ir::Edit;
use std::collections::BTreeSet;

pub(crate) use codegen::generate_backend;

pub(crate) struct Compiled {
    pub edits: Vec<Edit>,
    pub templates: String,
    pub locations: Vec<BindingLocation>,
    pub component_count: usize,
    pub app_offset: Option<usize>,
    pub fingerprint: String,
    pub javascript: Vec<crate::JavaScriptModule>,
    pub classes: BTreeSet<String>,
}

pub(crate) fn compile(
    source: &str,
    blocks: &[RustBlock],
    rust: &mut String,
    first_component: usize,
    embedded_templates: bool,
) -> Result<Compiled, ExtractError> {
    let plan = parse::parse(source, blocks, first_component)?;
    let app = plan
        .components
        .iter()
        .find_map(|component| component.app().map(|_| component.range.start));
    let locations = codegen::generate(source, &plan.components, rust, embedded_templates);
    // Static delivery templates belong to the immutable unit protocol. Ordinary
    // app refresh compares the executable bindings, which precede them.
    let fingerprint = rust.clone();
    rust.push_str(&codegen::delivery(&plan.components).to_string());
    rust.push('\n');
    let classes = plan
        .components
        .iter()
        .flat_map(|component| Binding::walk(&component.bindings))
        .filter_map(|binding| match binding {
            Binding::Class { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    Ok(Compiled {
        classes,
        javascript: plan
            .components
            .iter()
            .filter_map(|component| component.javascript.clone())
            .collect(),
        edits: plan.edits,
        templates: plan.templates,
        locations,
        component_count: plan.components.len(),
        app_offset: app,
        fingerprint,
    })
}
