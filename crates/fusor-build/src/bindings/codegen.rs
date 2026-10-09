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
    backend::{OperationKind, OperationMode, Runtime},
};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, quote_spanned};

// Hoist recursive factories once before selecting an ordinary or coherent mode.
#[derive(Default)]
struct BindingCode {
    shared: TokenStream,
    ordinary: TokenStream,
    coherent: TokenStream,
}

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
    fn operation(
        &self,
        binding: &Binding,
        operation: OperationKind,
        mode: OperationMode,
    ) -> TokenStream;
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
    mode: OperationMode,
    embedded_templates: bool,
}

impl Ctx<'_> {
    fn component(self, index: usize) -> TokenStream {
        self.backend.component(&self.components[index], self)
    }
    fn clone_ready(self) -> Option<TokenStream> {
        self.ready
            .then(|| quote! { #[allow(unused_variables, reason = "template scope bindings may be unused")] let ready = ::std::rc::Rc::clone(&ready); })
    }
}

/// Give each binding closure its lexical locals, ready value, state and children.
fn captures(span: Span, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let locals = clone_locals(locals);
    let ready = ctx.clone_ready();
    quote_spanned! {span=>
        #locals
        #ready
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(&state);
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
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(&__fusor_capture);
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

fn children_factory(slots: &[ChildFragment], ctx: Ctx) -> TokenStream {
    let default = slots
        .iter()
        .find(|slot| slot.name.is_none())
        .map(|slot| slot.body);
    let default = child_factory(default, ctx);
    let mut named = slots
        .iter()
        .filter_map(|slot| {
            slot.name.as_ref().map(|name| {
                let factory = child_factory(Some(slot.body), ctx);
                quote! { (#name, #factory) }
            })
        })
        .peekable();
    if named.peek().is_none() {
        default
    } else {
        quote! { (#default).with_named([#(#named),*]) }
    }
}

fn selected_children(name: &Option<String>) -> TokenStream {
    match name {
        Some(name) => quote! { __fusor_children.named(#name) },
        None => quote! { __fusor_children },
    }
}

fn child_factory(index: Option<usize>, ctx: Ctx) -> TokenStream {
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
        .then(|| quote! { #[allow(unused_variables, reason = "template scope bindings may be unused")] let ready = ::std::rc::Rc::clone(&__fusor_ready); });
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

fn shared_children_factory(
    id: fusor::template::MountId,
    slots: &[ChildFragment],
    ctx: Ctx,
) -> (Ident, TokenStream) {
    let make = indexed("make_children", id.index());
    let children = children_factory(slots, ctx);
    let locals = slots
        .first()
        .map(|slot| ctx.components[slot.body].async_locals.as_slice())
        .unwrap_or(&[]);
    let captures = captures(Span::call_site(), ctx, locals);
    let setup = quote! {
        let #make = {
            #captures
            move || #children
        };
    };
    (make, setup)
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
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(&#state);
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
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = #constructor(::std::rc::Rc::clone(&#state), entry);
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

/// Await reads and aliases have the same lexical meaning in every renderer.
fn await_binding(
    item: &Binding,
    ctx: Ctx,
    locals: &[Rust],
    emit: impl Fn(&Binding, Ctx, &[Rust]) -> TokenStream,
) -> TokenStream {
    let Binding::Region {
        value,
        kind: RegionKind::Await { alias },
        bindings,
        ..
    } = item
    else {
        unreachable!("Await binding")
    };
    let mut nested = locals.to_vec();
    if let Some(alias) = alias {
        nested.push(alias.clone());
    }
    let resolved = emit::or(alias.as_ref(), quote! { ready });
    let inner = Ctx {
        ready: ctx.ready || alias.is_none(),
        ..ctx
    };
    let bindings = bindings.iter().map(|binding| emit(binding, inner, &nested));
    quote_spanned! {item.span()=>
        if let ::fusor_async::AsyncRead::Ready(__fusor_ready_value) = (#value).read(__fusor_attempt)? {
            #[allow(unused_variables, reason = "an Await subtree may ignore its resolved value")]
            let #resolved = __fusor_ready_value;
            #(#bindings)*
        }
    }
}

fn coherent_rejection(binding: &Binding) -> Option<&'static str> {
    match binding {
        Binding::Region {
            kind: RegionKind::Boundary,
            ..
        } => Some("nested coherent boundaries are unsupported"),
        Binding::Invocation { inputs, .. } if has_content(inputs) => {
            Some("projected content cannot participate in coherent rendering")
        }
        Binding::Router { .. }
        | Binding::Island { .. }
        | Binding::Property { .. }
        | Binding::Value { .. }
        | Binding::Checked { .. }
        | Binding::Bind { .. }
        | Binding::Slot { .. } => Some(
            "editable controls, widgets, outlets and opaque content must remain outside coherent regions",
        ),
        _ => None,
    }
}

fn coherent_renderer(body: TokenStream, ctx: Ctx) -> TokenStream {
    let frame = ctx
        .runtime
        .coherent_frame
        .as_ref()
        .expect("coherent backend");
    quote! { move |__fusor_frame: &mut #frame<'_>| {
        let __fusor_attempt = __fusor_frame.attempt;
        #body
        ::std::result::Result::<(), ::std::string::String>::Ok(())
    }}
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
                { #clones #capture_ready move || { #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &__fusor_read_state; #selection } },
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

fn list_ready(ctx: Ctx) -> (TokenStream, [TokenStream; 3]) {
    let names = [
        "__fusor_read_ready",
        "__fusor_key_ready",
        "__fusor_row_ready",
    ]
    .map(|name| Ident::new(name, Span::call_site()));
    let captures = ctx
        .ready
        .then(|| quote! { #(let #names = ::std::rc::Rc::clone(&ready);)* });
    let aliases = names.map(|name| {
        if ctx.ready {
            quote! {
                #[allow(unused_variables, reason = "template scope bindings may be unused")]
                let ready = &#name;
            }
        } else {
            TokenStream::new()
        }
    });
    (captures.unwrap_or_default(), aliases)
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
    let (ready_capture, [read_ready, key_ready, row_ready]) = list_ready(ctx);
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
                { #clones move || { #read_ready #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &__fusor_read_state; #values({ #items }) } },
                { #clones move |entry| { #key_ready #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &__fusor_key_state; #value_key(entry, #key) } },
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
    let identity = emit::identity(condition.as_ref(), key.as_ref());
    let construct = construct_inputs(span, ty, inputs, ctx);
    let local_clones = clone_locals(locals);
    let ready = ctx.clone_ready();
    let identity = quote_spanned! {span=> { #local_clones #ready move || {
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &__fusor_identity_state;
        #identity
    }}};
    let make = quote_spanned! {span=> { #local_clones #ready move |owner| {
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &__fusor_child_state;
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
        ctx.mode,
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

fn router(routes: &[RouteBranch], ctx: Ctx, locals: &[Rust]) -> OperationKind {
    let routes = routes
        .iter()
        .map(|route| {
            let prepare = Ctx {
                ready: false,
                ..ctx
            }
            .component(route.body);
            let clones = clone_locals(locals);
            let params = route.params.as_ref().map(|alias| {
                let names = &route.names;
                let keys = names.iter().map(|name| name.tokens.to_string());
                quote! {
                    #[derive(Clone)]
                    struct __Params { #(pub #names: ::std::string::String),* }
                    let #alias = __Params {
                        #(#names: __fusor_match.params.get(#keys)
                            .expect("validated route capture").clone()),*
                    };
                }
            });
            let handoff = handoff(Span::call_site(), quote! { &state });
            let restore = restore(Span::call_site());
            crate::backend::Route {
                pattern: route.path.clone(),
                prepare: quote! {{
                    #handoff
                    #clones
                    move |__fusor_parent: &::fusor::OwnerHandle,
                          __fusor_match: &::fusor_router::pattern::Match| {
                        #restore
                        #clones
                        #params
                        #prepare
                    }
                }},
            }
        })
        .collect();
    OperationKind::Router { routes }
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
        Binding::Router { routes, .. } => (TokenStream::new(), router(routes, ctx, locals)),
        Binding::Invocation { children, .. } => {
            return invocation(item, children_factory(children, ctx), ctx, locals);
        }
        _ => return ctx.backend.binding(item, ctx, locals),
    };
    let operation = ctx.backend.operation(item, operation, ctx.mode);
    quote! { #setup #operation }
}

/// The browser is an explicit built-in target, never the absence of a backend.
pub(super) fn generate(
    source: &str,
    components: &[Component],
    rust: &mut String,
    embedded_templates: bool,
) -> Vec<BindingLocation> {
    generate_using(source, components, rust, &DomBackend, embedded_templates)
}

fn generate_using(
    source: &str,
    components: &[Component],
    rust: &mut String,
    backend: &dyn CompilerBackend,
    embedded_templates: bool,
) -> Vec<BindingLocation> {
    let runtime = backend.runtime();
    let ctx = Ctx {
        components,
        backend,
        runtime: &runtime,
        ready: false,
        mode: OperationMode::Reactive,
        embedded_templates,
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

        fn operation(
            &self,
            binding: &Binding,
            operation: OperationKind,
            mode: OperationMode,
        ) -> TokenStream {
            DomBackend.operation(binding, operation, mode)
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
        let direct_origins = generate(source, &plan.components, &mut direct, false);
        let recorder = RecordingDom::default();
        let mut delegated = String::new();
        let delegated_origins =
            generate_using(source, &plan.components, &mut delegated, &recorder, false);

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
