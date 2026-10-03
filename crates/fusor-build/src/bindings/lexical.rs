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
    let captures = Lexical {
        rows: &component.row_locals,
        routes: &component.route_locals,
        snapshots: &component.snapshot_locals,
        item_only: component.item_only_row,
    };
    Binding::visit_mut(&mut component.bindings, &mut |binding| {
        captures.binding(binding)
    });
}

struct Lexical<'a> {
    rows: &'a [(Rust, Rust)],
    routes: &'a [Rust],
    snapshots: &'a [Rust],
    item_only: bool,
}

impl Lexical<'_> {
    fn binding(&self, binding: &mut Binding) {
        match binding {
            Binding::Children { .. } | Binding::Router { .. } => {}
            Binding::Branch { value, .. }
            | Binding::Region { value, .. }
            | Binding::Property { value, .. }
            | Binding::Boolean { value, .. }
            | Binding::Checked { value, .. }
            | Binding::Class { value, .. }
            | Binding::Event { handler: value, .. } => self.wrap(value),
            Binding::Bind { value, control, .. } => {
                self.wrap(value);
                if let Some(choice) = control.choice_mut() {
                    choice
                        .expressions_mut()
                        .for_each(|value| self.wrap_text(value));
                }
            }
            Binding::ForEach { items, key, .. } => {
                self.wrap(items);
                self.wrap(key);
            }
            Binding::Invocation {
                inputs,
                condition,
                key,
                ..
            } => {
                expressions(inputs).for_each(|value| self.wrap(value));
                condition
                    .iter_mut()
                    .chain(key)
                    .for_each(|value| self.wrap(value));
            }
            Binding::Slot {
                content,
                condition,
                key,
                ..
            } => {
                self.wrap(content);
                condition
                    .iter_mut()
                    .chain(key)
                    .for_each(|value| self.wrap(value));
            }
            Binding::Island { inputs, .. } => {
                expressions(inputs).for_each(|value| self.wrap(value))
            }
            Binding::Text { value, .. } => self.wrap_text(value),
            Binding::Attribute { value, .. } | Binding::Value { value, .. } => value
                .expressions_mut()
                .for_each(|value| self.wrap_text(value)),
        }
    }

    fn wrap(&self, value: &mut Rust) {
        let original = &value.tokens;
        let mut aliases = Vec::new();
        let params = self.routes;
        let snapshots = self.snapshots;
        let snapshot_reads = quote! { #(
            #[allow(unused_variables, reason = "a binding may read only some enclosing Case captures")]
            let #snapshots = #snapshots.get();
        )* };
        let params = clone_locals(params);
        if self.rows.is_empty() {
            value.tokens = quote! {{ #snapshot_reads #params #original }};
            return;
        }
        aliases.push(quote! { #snapshot_reads #params });
        let depth = self.rows.len();
        let contexts: Vec<_> = (0..depth).map(|i| indexed("context", i)).collect();
        let first = &contexts[0];
        aliases.push(quote! { let #first = &state; });
        for i in 1..depth {
            let prev = &contexts[i - 1];
            let current = &contexts[i];
            aliases.push(quote! { let #current = &#prev.parent; });
        }
        for ((item, index), context) in self.rows.iter().zip(contexts.iter().rev()) {
            let index = (!self.item_only).then(|| {
                quote! {
                    #[allow(unused_variables, reason = "a row binding may ignore its index")]
                    let #index = &#context.index;
                }
            });
            aliases.push(quote! {
                #[allow(unused_variables, reason = "a row binding may ignore its item")]
                let #item = &#context.item;
                #index
            });
        }
        let outer = contexts
            .last()
            .expect("nonempty row locals create a context");
        value.tokens = quote! { { #(#aliases)* #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &#outer.parent; #original } };
    }

    fn wrap_text(&self, value: &mut Rust) {
        // Format before the lexical block ends: a string slice may borrow a
        // freshly read capture or a cloned route parameter inside that block.
        if !self.snapshots.is_empty() || !self.routes.is_empty() {
            let original = &value.tokens;
            value.tokens = quote! { ::std::string::ToString::to_string(&(#original)) };
        }
        self.wrap(value);
    }
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
