//! Rewrite lexical captures shared by lists, routes, Case and Await.
use super::{
    emit::{clone_locals, indexed},
    ir::*,
    tokens::Rust,
};
use quote::quote;

pub(super) fn rewrite(component: &mut Component) {
    if component.row_locals.is_empty()
        && component.route_locals.is_empty()
        && component.snapshot_locals.is_empty()
    {
        return;
    }
    let wrap = |value: &mut Rust| {
        let original = &value.tokens;
        let mut aliases = Vec::new();
        let params = &component.route_locals;
        let snapshots = &component.snapshot_locals;
        let snapshot_reads = quote! { #(let #snapshots = #snapshots.get();)* };
        let params = clone_locals(params);
        if component.row_locals.is_empty() {
            value.tokens = quote! {{ #snapshot_reads #params #original }};
            return;
        }
        aliases.push(quote! { #snapshot_reads #params });
        let depth = component.row_locals.len();
        let contexts: Vec<_> = (0..depth).map(|i| indexed("context", i)).collect();
        let first = &contexts[0];
        aliases.push(quote! { let #first = &state; });
        for i in 1..depth {
            let prev = &contexts[i - 1];
            let current = &contexts[i];
            aliases.push(quote! { let #current = &#prev.parent; });
        }
        for ((item, index), context) in component.row_locals.iter().zip(contexts.iter().rev()) {
            let index =
                (!component.item_only_row).then(|| quote! { let #index = &#context.index; });
            aliases.push(quote! { let #item = &#context.item; #index });
        }
        let outer = contexts.last().unwrap();
        value.tokens = quote! { { #(#aliases)* let state = &#outer.parent; #original } };
    };
    let wrap_text = |value: &mut Rust| {
        // Format before the lexical block ends: a string slice may borrow a
        // freshly read capture or a cloned route parameter inside that block.
        if !component.snapshot_locals.is_empty() || !component.route_locals.is_empty() {
            let original = &value.tokens;
            value.tokens = quote! { ::std::string::ToString::to_string(&(#original)) };
        }
        wrap(value);
    };
    let mut visit = |binding: &mut Binding| match binding {
        Binding::Children { .. } | Binding::Router { .. } => {}
        Binding::Branch { value, .. }
        | Binding::Region { value, .. }
        | Binding::Property { value, .. }
        | Binding::Boolean { value, .. }
        | Binding::Checked { value, .. }
        | Binding::Class { value, .. }
        | Binding::Event { handler: value, .. } => wrap(value),
        Binding::Bind { value, control, .. } => {
            wrap(value);
            if let Some(choice) = control.choice_mut() {
                choice.expressions_mut().for_each(wrap_text);
            }
        }
        Binding::ForEach { items, key, .. } => {
            wrap(items);
            wrap(key);
        }
        Binding::Invocation {
            inputs,
            condition,
            key,
            ..
        } => {
            expressions(inputs).for_each(&wrap);
            condition.iter_mut().chain(key).for_each(&wrap);
        }
        Binding::Slot {
            content,
            condition,
            key,
            ..
        } => {
            wrap(content);
            condition.iter_mut().chain(key).for_each(&wrap);
        }
        Binding::Island { inputs, .. } => expressions(inputs).for_each(&wrap),
        Binding::Text { value, .. } => wrap_text(value),
        Binding::Attribute { value, .. } | Binding::Value { value, .. } => {
            value.expressions_mut().for_each(wrap_text)
        }
    };
    Binding::visit_mut(&mut component.bindings, &mut visit);
}

/// The `{{ expression }}` inputs of a component tag; literals and content hold no locals.
fn expressions(inputs: &mut [Input]) -> impl Iterator<Item = &mut Rust> {
    inputs
        .iter_mut()
        .filter_map(|input| match &mut input.value {
            InputValue::Expression(value) => Some(value),
            _ => None,
        })
}
