//! Canonical semantic lowering shared by DOM and external compiler backends.
mod dom;
mod external;

use dom::DomBackend;
pub(super) use dom::delivery;
pub(crate) use external::generate as generate_backend;

use super::{
    emit::{self, clone_locals, indexed},
    ir::*,
    tokens::{self, Origins, Rust},
};
use crate::{
    BindingLocation,
    backend::{OperationKind, Runtime},
};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};

/// The private compiler emission contract. Backends own target-specific code;
/// the public external facade is adapted to this without exposing the private IR.
trait CompilerBackend {
    fn runtime(&self) -> Runtime;
    /// Own scope preparation, binding installation and the mounting implementation.
    /// Recursive factories must use `ctx.component` to retain the selected backend.
    fn component(&self, component: &Component, ctx: Ctx<'_>) -> TokenStream;
    /// Emit target-specific leaf bindings and modes such as coherent DOM frames.
    fn binding(&self, binding: &Binding, ctx: Ctx<'_>, locals: &[Rust]) -> TokenStream;
    /// Install a structural operation whose captures and factories are already
    /// lowered.
    fn operation(&self, binding: &Binding, operation: OperationKind) -> TokenStream;
    /// Wrap a canonical projected-content factory in the target's content type.
    fn content(&self, factory: TokenStream) -> TokenStream;
}

#[derive(Clone, Copy)]
struct Ctx<'a> {
    components: &'a [Component],
    backend: &'a dyn CompilerBackend,
    runtime: &'a Runtime,
    /// An enclosing await exposes a ready value to nested factories.
    ready: bool,
}

impl Ctx<'_> {
    fn component(self, index: usize) -> TokenStream {
        self.backend.component(&self.components[index], self)
    }
    fn clone_ready(self) -> Option<TokenStream> {
        self.ready
            .then(|| quote! { let ready = ::std::rc::Rc::clone(&ready); })
    }
}

/// Give each binding closure its lexical locals, ready value, state and children.
fn captures(span: Span, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let locals = clone_locals(locals);
    let ready = ctx.clone_ready();
    quote_spanned! {span=>
        #locals
        #ready
        let state = ::std::rc::Rc::clone(&state);
        let __fusor_children = __fusor_children.clone();
    }
}

/// Capture state and children for a reusable factory; `restore` rebinds each call.
fn handoff(span: Span, state: TokenStream) -> TokenStream {
    quote_spanned! {span=>
        let __fusor_capture = ::std::rc::Rc::clone(#state);
        let __fusor_forward = __fusor_children.clone();
    }
}

fn restore(span: Span) -> TokenStream {
    quote_spanned! {span=>
        let state = ::std::rc::Rc::clone(&__fusor_capture);
        let __fusor_children = __fusor_forward.clone();
    }
}

/// Fragments, content and rows prepare inside their caller's factory.
fn captured(prepare: &TokenStream, ctx: Ctx) -> TokenStream {
    let error = &ctx.runtime.error;
    quote! {{
        let parent = ::std::option::Option::Some(__fusor_parent);
        let make = move |_owner| ::std::result::Result::<_, #error>::Ok(state);
        #prepare
    }}
}

fn incoming_children(component: &Component, ctx: Ctx) -> TokenStream {
    let children = &ctx.runtime.children;
    if component.capture().is_none() && !component.inline() {
        quote! { let __fusor_children = #children::take(); }
    } else {
        quote! { let __fusor_children = __fusor_children.clone(); }
    }
}

fn children_factory(index: Option<usize>, ctx: Ctx) -> TokenStream {
    let children = &ctx.runtime.children;
    let Some(index) = index.filter(|index| !ctx.components[*index].empty) else {
        return quote! { #children::default() };
    };
    let prepare = ctx.component(index);
    let locals = clone_locals(&ctx.components[index].async_locals);
    let capture_ready = ctx
        .ready
        .then(|| quote! { let __fusor_ready = ::std::rc::Rc::clone(&ready); });
    let expose_ready = ctx
        .ready
        .then(|| quote! { let ready = ::std::rc::Rc::clone(&__fusor_ready); });
    let handoff = handoff(Span::call_site(), quote! { &state });
    let restore = restore(Span::call_site());
    quote! {{
        #handoff
        #capture_ready
        #locals
        #children::new(move |__fusor_parent| {
            #restore
            #expose_ready
            #locals
            #prepare
        })
    }}
}

/// Prepare the selected case from the shared branch payload and lexical captures.
fn branch_dispatch(
    span: Span,
    ctx: Ctx,
    cases: &[CaseBranch],
    snapshots: &[(Rust, Rust)],
    state: &str,
) -> TokenStream {
    let state = Ident::new(state, span);
    let factories = cases.iter().enumerate().map(|(index, case)| {
        let projections = super::control::projections(case, index, snapshots);
        let body = ctx.component(case.body);
        quote! { #index => { #projections #body }, }
    });
    quote_spanned! {span=>
        let state = ::std::rc::Rc::clone(&#state);
        match __fusor_case { #(#factories)* _ => unreachable!("generated branch index") }
    }
}

/// Index-free rows retain their source when moving; other rows include positions.
fn foreach_parts(
    span: Span,
    ctx: Ctx,
    body: usize,
    state: &str,
) -> (TokenStream, TokenStream, TokenStream) {
    let (values, value_key, constructor) = if ctx.components[body].item_only_row {
        (
            quote! { ::fusor_components::ForEach::values },
            quote! { ::fusor_components::ForEach::value_key },
            quote! { ::fusor_components::ForEach::item_row },
        )
    } else {
        (
            quote! { ::fusor_components::ForEach::entries },
            quote! { ::fusor_components::ForEach::key },
            quote! { ::fusor_components::ForEach::row },
        )
    };
    let state = Ident::new(state, span);
    let row = ctx.component(body);
    let prepare = quote_spanned! {span=>
        let state = #constructor(::std::rc::Rc::clone(&#state), entry);
        #row
    };
    (values, value_key, prepare)
}

/// Select values apply after option values and authored change listeners.
fn order_bindings(bindings: &[Binding]) -> Vec<&Binding> {
    let mut ordered: Vec<_> = bindings.iter().collect();
    ordered.sort_by_key(|binding| {
        matches!(
            binding,
            Binding::Bind {
                control: Control::Select | Control::SelectMultiple,
                ..
            }
        )
    });
    ordered
}

fn branch(item: &Binding, ctx: Ctx, locals: &[Rust]) -> (TokenStream, OperationKind) {
    let Binding::Branch {
        point: id,
        value,
        cases,
        snapshots,
    } = item
    else {
        unreachable!("branch binding")
    };
    let span = item.span();
    let slot = id.index();
    let read = indexed("branch_read", slot);
    let prepare = indexed("branch_prepare", slot);
    let selection = super::control::selection(value, cases, snapshots);
    let dispatch = branch_dispatch(span, ctx, cases, snapshots, "__fusor_capture");
    let clones = clone_locals(locals);
    let capture_ready = ctx.clone_ready();
    let scope = &ctx.runtime.scope;
    let error = &ctx.runtime.error;
    let setup = quote_spanned! {span=>
        let (#read, #prepare) = {
            // Give rustc the selection payload before checking projections
            // in the hoisted constructor; application types stay inferred.
            fn __fusor_branch_parts<T, R, F>(read: R, prepare: F) -> (R, F)
            where
                T: ::std::clone::Clone + ::std::cmp::PartialEq + 'static,
                R: ::std::ops::Fn() -> (usize, T),
                F: ::std::ops::Fn(usize, ::fusor::Signal<T>, &::fusor::OwnerHandle)
                    -> ::std::result::Result<#scope, #error>,
            {
                (read, prepare)
            }
            let __fusor_read_state = ::std::rc::Rc::clone(&state);
            let __fusor_capture = ::std::rc::Rc::clone(&state);
            let __fusor_children = __fusor_children.clone();
            __fusor_branch_parts(
                { #clones #capture_ready move || { let state = &__fusor_read_state; #selection } },
                { #clones #capture_ready move |__fusor_case, __fusor_data, __fusor_parent| {
                    #dispatch
                } }
            )
        };
    };
    (
        setup,
        OperationKind::Branch {
            read: quote! { #read },
            prepare: quote! { #prepare },
        },
    )
}

fn list(item: &Binding, ctx: Ctx, locals: &[Rust]) -> (TokenStream, OperationKind) {
    let Binding::ForEach {
        node,
        items,
        key,
        body,
    } = item
    else {
        unreachable!("ForEach binding")
    };
    let span = item.span();
    let slot = node.index();
    let read = indexed("list_read", slot);
    let key_fn = indexed("list_key", slot);
    let prepare = indexed("list_prepare", slot);
    let (values, value_key, row) = foreach_parts(span, ctx, *body, "__fusor_row_state");
    let ready_capture = ctx.ready.then(|| {
        quote! {
            let __fusor_read_ready = ::std::rc::Rc::clone(&ready);
            let __fusor_key_ready = ::std::rc::Rc::clone(&ready);
            let __fusor_row_ready = ::std::rc::Rc::clone(&ready);
        }
    });
    let read_ready = ctx
        .ready
        .then(|| quote! { let ready = &__fusor_read_ready; });
    let key_ready = ctx
        .ready
        .then(|| quote! { let ready = &__fusor_key_ready; });
    let row_ready = ctx
        .ready
        .then(|| quote! { let ready = &__fusor_row_ready; });
    let clones = clone_locals(locals);
    let scope = &ctx.runtime.scope;
    let error = &ctx.runtime.error;
    let setup = quote_spanned! {span=>
        let (#read, #key_fn, #prepare) = {
            fn __fusor_list_parts<T, K, R, KF, F>(read: R, key: KF, prepare: F) -> (R, KF, F)
            where
                T: ::std::clone::Clone + ::std::cmp::PartialEq + 'static,
                K: ::std::cmp::Ord + ::std::clone::Clone + 'static,
                R: ::std::ops::Fn() -> ::std::vec::Vec<T>,
                KF: ::std::ops::Fn(&T) -> K,
                F: ::std::ops::Fn(::fusor::Signal<T>, &::fusor::OwnerHandle)
                    -> ::std::result::Result<#scope, #error>,
            {
                (read, key, prepare)
            }
            #ready_capture
            let __fusor_read_state = ::std::rc::Rc::clone(&state);
            let __fusor_key_state = ::std::rc::Rc::clone(&state);
            let __fusor_row_state = ::std::rc::Rc::clone(&state);
            let __fusor_children = __fusor_children.clone();
            __fusor_list_parts(
                { #clones move || { #read_ready let state = &__fusor_read_state; #values({ #items }) } },
                { #clones move |entry| { #key_ready let state = &__fusor_key_state; #value_key(entry, #key) } },
                { #clones move |entry, __fusor_parent| {
                    #row_ready
                    #row
                } }
            )
        };
    };
    (
        setup,
        OperationKind::Keyed {
            read: quote! { #read },
            key: quote! { #key_fn },
            prepare: quote! { #prepare },
        },
    )
}

fn invocation(binding: &Binding, children: TokenStream, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let Binding::Invocation {
        ty,
        inputs,
        condition,
        key,
        ..
    } = binding
    else {
        unreachable!("component invocation")
    };
    let span = binding.span();
    let condition = emit::or(condition.as_ref(), quote! { true });
    let key = emit::or(key.as_ref(), quote! { () });
    let construct = construct_inputs(span, ty, inputs, ctx);
    let local_clones = clone_locals(locals);
    let identity = quote_spanned! {span=> { #local_clones move || {
        let state = &__fusor_identity_state;
        if #condition { ::std::option::Option::Some({ #key }) }
        else { ::std::option::Option::None }
    }}};
    let make = quote_spanned! {span=> { #local_clones move |owner| {
        let state = &__fusor_child_state;
        #construct
    }}};
    let mount = ctx.backend.operation(
        binding,
        OperationKind::Component {
            ty: quote! { #ty },
            identity,
            make,
            children: quote! { __fusor_supplied },
        },
    );
    quote_spanned! {span=> {
        #local_clones
        let __fusor_identity_state = ::std::rc::Rc::clone(&state);
        let __fusor_child_state = ::std::rc::Rc::clone(&state);
        let __fusor_supplied = #children;
        let __fusor_children = __fusor_children.clone();
        #mount
    }}
}

fn has_content(inputs: &[Input]) -> bool {
    inputs
        .iter()
        .any(|input| matches!(input.value, InputValue::Content { .. }))
}

/// Lower input structs and projected-content factories once for every target.
fn construct_inputs(span: Span, ty: &Rust, inputs: &[Input], ctx: Ctx) -> TokenStream {
    let fields = emit::fields(inputs, |value| match value {
        InputValue::Content {
            component: index,
            origin,
        } => {
            let prepare = ctx.component(*index);
            let span = origin.span();
            let handoff = handoff(span, quote_spanned! {span=> state });
            let restore = restore(span);
            let content = ctx.backend.content(quote! { move |__fusor_parent| {
                #restore
                #prepare
            }});
            quote_spanned! {span=> { #handoff #content }}
        }
        value => emit::braced(value),
    });
    emit::construct_inputs(span, ty, fields, &ctx.runtime.convert_error)
}

/// All recursive factories pass through the same backend as their parent.
fn binding(item: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let (setup, operation) = match item {
        Binding::Branch { .. } => branch(item, ctx, locals),
        Binding::ForEach { .. } => list(item, ctx, locals),
        Binding::Invocation { children, .. } => {
            return invocation(item, children_factory(*children, ctx), ctx, locals);
        }
        _ => return ctx.backend.binding(item, ctx, locals),
    };
    let operation = ctx.backend.operation(item, operation);
    quote! { #setup #operation }
}

/// The browser is an explicit built-in target, never the absence of a backend.
pub(super) fn generate(
    source: &str,
    components: &[Component],
    rust: &mut String,
) -> Vec<BindingLocation> {
    generate_using(source, components, rust, &DomBackend)
}

fn generate_using(
    source: &str,
    components: &[Component],
    rust: &mut String,
    backend: &dyn CompilerBackend,
) -> Vec<BindingLocation> {
    let runtime = backend.runtime();
    let ctx = Ctx {
        components,
        backend,
        runtime: &runtime,
        ready: false,
    };
    let mut origins = Origins::default();
    for component in components {
        origins.register(&component.ty);
        if let Some(app) = component.app() {
            origins.register(app);
        }
        for fragment in component.bindings.iter().flat_map(Binding::fragments) {
            origins.register(fragment);
        }
    }
    let mut locations = Vec::new();
    for item in components
        .iter()
        .filter(|item| !item.inline() && item.capture().is_none())
    {
        locations.extend(tokens::emit(
            source,
            rust,
            ctx.backend.component(item, ctx),
            &origins,
            item.ty.offset,
        ));
    }
    locations
}

#[cfg(test)]
mod tests {
    //! Structural dispatch regression; public consumers execute the generated code.
    use super::*;
    use std::{cell::RefCell, collections::BTreeSet};

    #[derive(Default)]
    struct RecordingDom {
        components: RefCell<Vec<usize>>,
        bindings: RefCell<Vec<&'static str>>,
        content: RefCell<usize>,
    }

    impl CompilerBackend for RecordingDom {
        fn runtime(&self) -> Runtime {
            DomBackend.runtime()
        }

        fn component(&self, component: &Component, ctx: Ctx<'_>) -> TokenStream {
            self.components.borrow_mut().push(component.id.index());
            DomBackend.component(component, ctx)
        }

        fn binding(&self, binding: &Binding, ctx: Ctx<'_>, locals: &[Rust]) -> TokenStream {
            self.bindings.borrow_mut().push(match binding {
                Binding::Bind { .. } => "bind",
                Binding::Children { .. } => "children",
                Binding::Slot { .. } => "slot",
                _ => "leaf",
            });
            DomBackend.binding(binding, ctx, locals)
        }

        fn operation(&self, binding: &Binding, operation: OperationKind) -> TokenStream {
            DomBackend.operation(binding, operation)
        }

        fn content(&self, factory: TokenStream) -> TokenStream {
            *self.content.borrow_mut() += 1;
            DomBackend.content(factory)
        }
    }

    #[test]
    fn delegated_dom_backend_receives_nested_factories_and_preserves_output_and_origins() {
        let source = r#"<template rust:component="Root"><section>
  <input bind="state.query">
  <If condition="{{ state.visible.get() }}">
    <div><ForEach items="{{ state.groups.get() }}" key="{{ |group| group.id }}">
      <article>
        <p>{{ state.title }}: {{ item.get().label }}</p>
        <Match value="{{ item.get().selected }}">
          <Case pattern="Some(chosen)">
            <ul><ForEach items="{{ chosen.get().children }}" key="{{ |child| child.id }}">
              <button on:click="state.open(item.get().id)">{{ item.get().label }} {{ chosen.get().label }}</button>
            </ForEach></ul>
          </Case>
          <Case pattern="None"><p>empty</p></Case>
        </Match>
        <Frame title="{{ item.get().label }}"><p>{{ state.title }} {{ item.get().label }}</p></Frame>
      </article>
    </ForEach></div>
    <Else><p>hidden</p></Else>
  </If>
  <Panel><template rust:content="body"><div><If condition="{{ state.visible.get() }}"><p>{{ state.title }}</p></If></div></template></Panel>
</section></template>
<template rust:component="Frame"><section><Children></Children></section></template>
<template rust:component="Panel"><section rust:slot="state.body.clone()"></section></template>"#;
        let plan = crate::bindings::parse::parse(source, &[], 0).unwrap();
        let mut direct = String::new();
        let direct_origins = generate(source, &plan.components, &mut direct);
        let recorder = RecordingDom::default();
        let mut delegated = String::new();
        let delegated_origins = generate_using(source, &plan.components, &mut delegated, &recorder);

        assert_eq!(delegated, direct);
        assert_eq!(delegated_origins, direct_origins);
        syn::parse_file(&delegated).expect("delegated backend emits valid Rust syntax");

        // A direct call to DomBackend from a nested factory would preserve output,
        // but omit its component from the selected backend's observed dispatch.
        let expected: BTreeSet<_> = plan
            .components
            .iter()
            .filter(|item| !item.empty)
            .map(|item| item.id.index())
            .collect();
        let observed: BTreeSet<_> = recorder.components.borrow().iter().copied().collect();
        assert!(
            expected.is_subset(&observed),
            "nonempty factories bypassed the selected backend: {:?}",
            expected.difference(&observed).collect::<Vec<_>>()
        );
        assert!(plan.components.iter().any(Component::inline));
        assert!(plan.components.iter().any(Component::fragment));
        assert!(
            plan.components
                .iter()
                .any(|item| matches!(item.shape, ComponentShape::Content(_)))
        );
        for binding in ["bind", "children", "slot"] {
            assert!(recorder.bindings.borrow().contains(&binding));
        }
        assert_eq!(*recorder.content.borrow(), 1);
    }
}
