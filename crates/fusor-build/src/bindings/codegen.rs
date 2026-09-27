//! Lower the binding plan into native Rust token trees.

mod template;

use super::{
    emit::{self, clone_locals, element, indexed, point, string, text},
    ir::*,
    tokens::{self, Origins, Rust},
};
use crate::BindingLocation;
use fusor::template::{ElementId, MountId, RootKind, TextId};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use std::collections::BTreeMap;

// The typed path changes the generated closure's result representation. Keep
// the original String result when authored code can return from that closure;
// opaque macros and attributes may introduce such a return after expansion.
pub(super) fn typed_text_eligible(value: &Rust) -> bool {
    fn transparent(tokens: TokenStream) -> bool {
        tokens.into_iter().all(|token| match token {
            proc_macro2::TokenTree::Group(group) => transparent(group.stream()),
            proc_macro2::TokenTree::Ident(ident) => ident != "return",
            proc_macro2::TokenTree::Punct(punct) => !matches!(punct.as_char(), '!' | '#'),
            proc_macro2::TokenTree::Literal(_) => true,
        })
    }
    transparent(value.tokens.clone())
}

/// What every lowering in one generation pass shares.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    components: &'a [Component],
    /// An enclosing `rust:await` exposes `ready`.
    ready: bool,
}

impl Ctx<'_> {
    fn component(self, index: usize) -> TokenStream {
        component(&self.components[index], self)
    }

    fn clone_ready(self) -> Option<TokenStream> {
        self.ready
            .then(|| quote! { let ready = ::std::rc::Rc::clone(&ready); })
    }
}

/// Copies of the lexical locals, `ready`, state and incoming children that a
/// binding closure owns.
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

/// Move the state and incoming children into a factory that may run more than
/// once; `restore` rebinds them on each call.
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

fn children_factory(index: Option<usize>, ctx: Ctx) -> TokenStream {
    let Some(index) = index.filter(|index| !ctx.components[*index].empty) else {
        return quote! { ::fusor::dom::Children::default() };
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
        ::fusor::dom::Children::new(move |__fusor_parent| {
            #restore
            #expose_ready
            #locals
            #prepare
        })
    }}
}

// Recursive constructors must occur once in the emitted Rust, before runtime
// mode selection. Each mode keeps its own scheduling and ownership adapter.
struct BrowserBinding {
    shared: TokenStream,
    ordinary: TokenStream,
    coherent: TokenStream,
}

fn browser_binding(item: &Binding, shared: bool, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
    match item {
        Binding::Branch { .. } => shared_branch(item, ctx, locals),
        Binding::ForEach { .. } => shared_list(item, shared, ctx, locals),
        // Named Content has only an ordinary path, and its creation supplies
        // retained slot identity. Only share factories used by both modes.
        Binding::Invocation {
            point,
            inputs,
            children: Some(child),
            ..
        } if !ctx.components[*child].empty
            && !inputs
                .iter()
                .any(|input| matches!(input.value, InputValue::Content { .. })) =>
        {
            shared_children(item, *point, *child, ctx, locals)
        }
        Binding::Region {
            node,
            kind: RegionKind::Await { alias: Some(_) },
            ..
        } => shared_await(item, *node, ctx, locals),
        _ => BrowserBinding {
            shared: TokenStream::new(),
            ordinary: binding(item, ctx, locals),
            coherent: coherent_binding(item, ctx, locals),
        },
    }
}

fn shared_branch(item: &Binding, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
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
    let point = point(*id);
    let read = indexed("branch_read", slot);
    let prepare = indexed("branch_prepare", slot);
    let selection = super::control::selection(value, cases, snapshots);
    let dispatch = branch_dispatch(span, ctx, cases, snapshots, "__fusor_capture");
    let clones = clone_locals(locals);
    let capture_ready = ctx.clone_ready();
    BrowserBinding {
        shared: quote_spanned! {span=>
            let (#read, #prepare) = {
                // Give rustc the selection payload before checking projections
                // in the hoisted constructor; application types stay inferred.
                fn __fusor_branch_parts<T, R, F>(read: R, prepare: F) -> (R, F)
                where
                    T: ::std::clone::Clone + ::std::cmp::PartialEq + 'static,
                    R: ::std::ops::Fn() -> (usize, T),
                    F: ::std::ops::Fn(usize, ::fusor::Signal<T>, &::fusor::OwnerHandle)
                        -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue>,
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
        },
        ordinary: quote_spanned! {span=> __fusor_scope.branch_at(&#point, #read, #prepare)?; },
        coherent: quote_spanned! {span=> __fusor_frame.branch_at(#slot, &#point, &#read, &#prepare)?; },
    }
}

fn shared_list(item: &Binding, shared: bool, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
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
    let node = element(*node);
    let read = indexed("list_read", slot);
    let key_fn = indexed("list_key", slot);
    let prepare = indexed("list_prepare", slot);
    let row = foreach_row(span, ctx, *body, "__fusor_row_state");
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
    let (values, value_key) = row_values(ctx, *body);
    let ordinary_row = quote! { move |entry| #prepare(entry, &__fusor_parent) };
    let mount = if shared {
        quote! { __fusor_scope.keyed_hydrated(&#node, #read, #key_fn, #ordinary_row, |key| ::fusor_islands::encode(key).map_err(|error| ::fusor::dom::JsValue::from_str(&error.to_string())))?; }
    } else {
        quote! { __fusor_scope.keyed(&#node, #read, #key_fn, #ordinary_row)?; }
    };
    BrowserBinding {
        shared: quote_spanned! {span=>
            let (#read, #key_fn, #prepare) = {
                fn __fusor_list_parts<T, K, R, KF, F>(read: R, key: KF, prepare: F) -> (R, KF, F)
                where
                    T: ::std::clone::Clone + ::std::cmp::PartialEq + 'static,
                    K: ::std::cmp::Ord + ::std::clone::Clone + 'static,
                    R: ::std::ops::Fn() -> ::std::vec::Vec<T>,
                    KF: ::std::ops::Fn(&T) -> K,
                    F: ::std::ops::Fn(::fusor::Signal<T>, &::fusor::OwnerHandle)
                        -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue>,
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
        },
        ordinary: quote_spanned! {span=> {
            let __fusor_parent = __fusor_scope.owner();
            #mount
        } },
        coherent: quote_spanned! {span=> __fusor_frame.keyed(#slot, #node.as_ref(), &#read, &#key_fn, &#prepare)?; },
    }
}

fn shared_children(
    item: &Binding,
    id: MountId,
    child: usize,
    ctx: Ctx,
    locals: &[Rust],
) -> BrowserBinding {
    let span = item.span();
    let make = indexed("make_children", id.index());
    let children = children_factory(Some(child), ctx);
    let captures = captures(span, ctx, &ctx.components[child].async_locals);
    BrowserBinding {
        shared: quote_spanned! {span=>
            let #make = {
                #captures
                move || #children
            };
        },
        ordinary: invocation(
            item,
            quote! {{ let __fusor_make_children = #make; __fusor_make_children() }},
            false,
            ctx,
            locals,
        ),
        coherent: invocation(item, quote! { #make() }, true, ctx, locals),
    }
}

fn shared_await(item: &Binding, node: ElementId, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
    let span = item.span();
    let render = indexed("await_render", node.index());
    let node = element(node);
    let body = coherent_binding(item, ctx, locals);
    let captures = captures(span, ctx, locals);
    BrowserBinding {
        shared: quote_spanned! {span=>
            let #render = {
                #captures
                let #node = #node.clone();
                move |__fusor_frame: &mut ::fusor::dom::coherent::Frame<'_>| {
                    let __fusor_attempt = __fusor_frame.attempt;
                    #body
                    ::std::result::Result::<(), ::std::string::String>::Ok(())
                }
            };
        },
        ordinary: quote_spanned! {span=> {
            let __fusor_region_root = #node.clone();
            __fusor_scope.async_region(&__fusor_region_root, ::fusor::coherence::AsyncBoundary::coherent(), #render)?;
        } },
        coherent: quote_spanned! {span=> #render(__fusor_frame)?; },
    }
}

fn invocation(
    binding: &Binding,
    children: TokenStream,
    coherent: bool,
    ctx: Ctx,
    locals: &[Rust],
) -> TokenStream {
    let Binding::Invocation {
        point: id,
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
    if coherent
        && inputs
            .iter()
            .any(|input| matches!(input.value, InputValue::Content { .. }))
    {
        return emit::reject(
            span,
            "projected content cannot participate in coherent rendering",
        );
    }
    let point = point(*id);
    let condition = emit::or(condition.as_ref(), quote! { true });
    let key = emit::or(key.as_ref(), quote! { () });
    let fields = emit::fields(inputs, |value| match value {
        InputValue::Content {
            component: index,
            origin,
        } => {
            let prepare = ctx.component(*index);
            let span = origin.span();
            let handoff = handoff(span, quote_spanned! {span=> state });
            let restore = restore(span);
            quote_spanned! {span=> {
                #handoff
                ::fusor::dom::Content::from_prepared(move |__fusor_parent| {
                    #restore
                    #prepare
                })
            }}
        }
        value => emit::braced(value),
    });
    let construct = emit::from_inputs(span, ty, fields);
    if coherent {
        let slot = id.index();
        return quote_spanned! {span=> __fusor_frame.component_at(#slot, &#point, if #condition { ::std::option::Option::Some({ #key }) } else { ::std::option::Option::None }, |owner| {
            #construct
        }, #children)?; };
    }
    let local_clones = clone_locals(locals);
    quote_spanned! {span=> {
        #local_clones
        let __fusor_identity_state = ::std::rc::Rc::clone(&state);
        let __fusor_child_state = ::std::rc::Rc::clone(&state);
        let __fusor_supplied = #children;
        let __fusor_children = __fusor_children.clone();
        __fusor_scope.component_at(&#point, { #local_clones move || {
            let state = &__fusor_identity_state;
            if #condition { ::std::option::Option::Some({ #key }) }
            else { ::std::option::Option::None }
        }}, { #local_clones move |owner| {
            let state = &__fusor_child_state;
            #construct
        }}, __fusor_supplied)?;
    }}
}

fn binding(binding: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let span = binding.span();
    let operation = match binding {
        Binding::Branch { .. }
        | Binding::ForEach { .. }
        | Binding::Region {
            kind: RegionKind::Await { alias: Some(_) },
            ..
        } => {
            unreachable!("recursive browser bindings share their constructors across modes")
        }
        Binding::Router { point, routes, .. } => {
            return router(span, *point, routes, ctx, locals);
        }
        Binding::Children { point: id, .. } => {
            let point = point(*id);
            quote! { __fusor_scope.children_at(&#point, &__fusor_children)?; }
        }
        Binding::Invocation { children, .. } => {
            let children = children_factory(*children, ctx);
            return invocation(binding, children, false, ctx, locals);
        }
        Binding::Island { .. } => {
            emit::fail(span, "independent islands must be rendered by the server")
        }
        Binding::Region {
            kind: RegionKind::Await { alias: None },
            ..
        } => emit::fail(
            span,
            "rust:await requires a coherent boundary ancestor; use Await for independent loading",
        ),
        Binding::Region {
            node,
            value,
            kind: RegionKind::Boundary,
            bindings,
        } => {
            let node = element(*node);
            let bindings = bindings
                .iter()
                .map(|binding| coherent_binding(binding, ctx, locals));
            quote_spanned! {span=>
                let __fusor_region_root = #node.clone();
                __fusor_scope.async_region(&__fusor_region_root, (#value).clone(), move |__fusor_frame| {
                    let __fusor_attempt = __fusor_frame.attempt;
                    #(#bindings)*
                    ::std::result::Result::Ok(())
                })?;
            }
        }
        Binding::Text { .. } | Binding::Attribute { .. } | Binding::Event { .. } => {
            scoped(binding, None).expect("scoped binding")
        }
        Binding::Property { node, name, value } => {
            let node = element(*node);
            quote_spanned! {span=> __fusor_scope.property(&#node, #name, move || { #value })?; }
        }
        Binding::Boolean { node, name, value } => {
            let node = element(*node);
            quote_spanned! {span=>
                __fusor_scope.attr(&#node, #name, move || {
                    let value: bool = { #value };
                    value.then(::std::string::String::new)
                })?;
            }
        }
        Binding::Value { node, value } => {
            let node = element(*node);
            let value = string(value);
            quote_spanned! {span=> __fusor_scope.value(&#node, move || { #value })?; }
        }
        Binding::Checked { node, value } => {
            let node = element(*node);
            quote_spanned! {span=> __fusor_scope.checked(&#node, move || { #value })?; }
        }
        Binding::Class { node, name, value } => {
            let node = element(*node);
            quote_spanned! {span=> __fusor_scope.class(&#node, #name, move || { #value })?; }
        }
        Binding::Bind {
            node,
            control,
            value,
        } => super::bind::browser(*node, control, value),
        Binding::Slot {
            node,
            content,
            condition,
            key,
        } => {
            let node = element(*node);
            let condition = condition.as_ref().map_or(
                quote! { true },
                |value| quote_spanned! {value.span()=> #value },
            );
            let pair = key.as_ref().map_or(
                quote! { ((), __fusor_content) },
                |value| quote_spanned! {value.span()=> ({ #value }, __fusor_content) },
            );
            quote_spanned! {span=>
                __fusor_scope.slot_with(&#node, move || {
                    if #condition {
                        let __fusor_content: ::std::option::Option<::fusor::dom::Content> =
                            ::std::convert::Into::into({ #content });
                        __fusor_content.map(|__fusor_content| #pair)
                    } else {
                        ::std::option::Option::None
                    }
                })?;
            }
        }
    };
    let captures = captures(span, ctx, locals);
    quote_spanned! {span=> {
        #captures
        #operation
    }}
}

/// Mount the declarative routes; each route body prepares without `ready`.
fn router(
    span: Span,
    id: MountId,
    routes: &[RouteBranch],
    ctx: Ctx,
    locals: &[Rust],
) -> TokenStream {
    let point = point(id);
    let factories = routes.iter().map(|route| {
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
        let restore = restore(Span::call_site());
        let factory = quote! {
            move |__fusor_parent: &::fusor::OwnerHandle,
                  __fusor_match: &::fusor_router::pattern::Match| {
                #restore
                #clones
                #params
                #prepare
            }
        };
        let construct = if let Some(path) = &route.path {
            quote! { ::fusor_router::browser::declarative::RouteView::new(#path, #factory)? }
        } else {
            quote! { ::fusor_router::browser::declarative::RouteView::fallback(#factory) }
        };
        let handoff = handoff(Span::call_site(), quote! { &state });
        quote! {{
            #handoff
            #clones
            #construct
        }}
    });
    quote_spanned! {span=> {
        ::fusor_router::browser::declarative::mount_routes(
            &mut __fusor_scope, &#point, ::std::env!("FUSOR_BASE_PATH"),
            ::std::vec![#(#factories),*]
        )?;
    }}
}

/// The ordinals of the binding bundle the server delivered, which stand in for
/// the typed handles.
struct Bundle<'a> {
    elements: &'a BTreeMap<ElementId, u32>,
    texts: &'a BTreeMap<TextId, u32>,
}

/// Call a scope method on the anchor's typed handle, or with a bundle, its
/// `bundle_` counterpart on the anchor's ordinal.
fn scope_call(
    span: Span,
    anchor: Anchor,
    bundle: Option<&Bundle>,
    method: &str,
    arguments: TokenStream,
) -> TokenStream {
    let Some(bundle) = bundle else {
        let method = Ident::new(method, span);
        let node = emit::handle(anchor);
        return quote_spanned! {span=> __fusor_scope.#method(&#node, #arguments)?; };
    };
    let method = format_ident!("bundle_{}", method, span = span);
    let slot = match anchor {
        Anchor::Element(id) => bundle.elements[&id],
        Anchor::Text(id) => bundle.texts[&id],
        Anchor::Mount(_) => unreachable!("bundles hold elements and text"),
    };
    quote_spanned! {span=> __fusor_scope.#method(&__fusor_bundle, #slot, #arguments)?; }
}

/// Text, attribute and event bindings, which the ordinary and bundle paths
/// install the same way. Other bindings return `None`: they need typed handles,
/// managed lifetimes or a coherent frame.
fn scoped(binding: &Binding, bundle: Option<&Bundle>) -> Option<TokenStream> {
    let span = binding.span();
    let anchor = binding.anchor();
    let (method, arguments) = match binding {
        Binding::Text { value, .. } if typed_text_eligible(value) => (
            "text_node_value",
            quote_spanned! {span=> move || {
                use ::fusor::dom::text_value::Convert as _;
                (&::fusor::dom::text_value::Value(&(#value))).__fusor_into_text()
            }},
        ),
        Binding::Text { value, .. } => (
            "text_node_string",
            quote_spanned! {span=> move || ::std::string::ToString::to_string(&(#value)) },
        ),
        // A lone interpolation shares the text path's exact-integer conversion.
        Binding::Attribute { name, value, .. } => match value.as_expression() {
            Some(expression) if typed_text_eligible(expression) => (
                "attr_value",
                quote_spanned! {span=> #name, move || {
                    use ::fusor::dom::text_value::Convert as _;
                    (&::fusor::dom::text_value::Value(&(#expression))).__fusor_into_text()
                }},
            ),
            _ => {
                let value = string(value);
                (
                    "attr",
                    quote_spanned! {span=> #name, move || ::std::option::Option::Some(#value) },
                )
            }
        },
        Binding::Event { name, handler, .. } => (
            "on",
            quote_spanned! {span=> #name, move |event| { #handler } },
        ),
        _ => return None,
    };
    Some(scope_call(span, anchor, bundle, method, arguments))
}

fn component(component: &Component, ctx: Ctx) -> TokenStream {
    let server = (component.render != RenderTarget::Browser && component.capture().is_none())
        .then(|| super::server::component(component, ctx.components));
    if component.render == RenderTarget::Server && component.capture().is_none() {
        return quote! { #server };
    }
    if let Some(forward) = forwarding_row(component, ctx) {
        return forward;
    }
    // Fragments mount from their HTML; the others only deliver it.
    let html = if component.fragment() {
        let html = &component.html;
        quote! { #html }
    } else {
        let name = delivery_html(component);
        quote! { #name }
    };
    let prepare = prepare(component, &html, ctx);
    match &component.shape {
        ComponentShape::App(expression) => app_entry(expression, &prepare),
        ComponentShape::Row | ComponentShape::Fragment(_) | ComponentShape::Content(_) => {
            captured(&prepare)
        }
        ComponentShape::Declared(_) => component_impl(component, server, &html, &prepare),
    }
}

/// A row whose HTML is a single component tag mounts that component directly.
fn forwarding_row(component: &Component, ctx: Ctx) -> Option<TokenStream> {
    if !component.inline()
        || !component.elements.is_empty()
        || !component.texts.is_empty()
        || !component.text_elements.is_empty()
    {
        return None;
    }
    let [
        Binding::Invocation {
            ty,
            inputs,
            children,
            condition: None,
            key: None,
            ..
        },
    ] = component.bindings.as_slice()
    else {
        return None;
    };
    let local_clones = clone_locals(&component.async_locals);
    let fields = emit::fields(inputs, emit::braced);
    let construct = emit::from_inputs(Span::call_site(), ty, fields);
    let supplied = children_factory(*children, ctx);
    Some(quote! {{
        #local_clones
        let state = ::std::rc::Rc::new(state);
        let __fusor_supplied = #supplied;
        __fusor_supplied.with(|| <#ty as ::fusor::dom::Component>::prepare(__fusor_parent, move |owner| {
            #construct
        }))
    }})
}

/// Mount the template, construct the component's state and install its bindings.
fn prepare(component: &Component, html: &TokenStream, ctx: Ctx) -> TokenStream {
    let local_clones = clone_locals(&component.async_locals);
    let template::TemplateCode {
        declarations,
        typed_handles,
        bundle,
    } = template::lower(component, ctx);
    let expose_capture = component.capture().map(|_| {
        quote! {
            let state = ::std::rc::Rc::clone(state.as_ref());
        }
    });
    // Declared components publish their HTML as a constant; the others embed it.
    let template_html = match component.shape {
        ComponentShape::Declared(_) => quote! { Self::TEMPLATE_HTML },
        _ => quote! { #html },
    };
    let mount_method = if component.fragment() {
        format_ident!("prepare_fragment")
    } else {
        format_ident!("prepare_with_points")
    };
    let mount = if bundle.is_some() {
        quote! { __FUSOR_TEMPLATE.prepare_with_binding_bundle({ #template_html }, parent)? }
    } else {
        quote! { __FUSOR_TEMPLATE.#mount_method({ #template_html }, __FUSOR_MOUNTS, parent)? }
    };
    let install = install(component, typed_handles, bundle, ctx);
    let incoming = if component.capture().is_none() && !component.inline() {
        quote! { let __fusor_children = ::fusor::dom::Children::take(); }
    } else {
        quote! { let __fusor_children = __fusor_children.clone(); }
    };
    let capture_ready = ctx.clone_ready();
    let javascript_inputs = component.javascript.as_ref().map(|_| {
        quote! {
            let __fusor_js_inputs = {
                use ::fusor::js::MaybeInputs as _;
                ::fusor::js::InputSource(&*state).inputs()
            };
        }
    });
    let javascript = component.javascript.as_ref().map(|module| {
        let id = &module.id;
        quote! { ::fusor::js::mount(&mut __fusor_scope, #id, __fusor_js_inputs)?; }
    });
    quote! {
                #local_clones
                #capture_ready
                #incoming
                let __fusor_nesting = ::fusor::dom::NestingGuard::enter()?;
                #declarations
                let (mut __fusor_scope, mut __fusor_nodes) = #mount;
                let state = if __fusor_scope.prepares_effects() {
                    ::fusor::coherence::prepare_state(__fusor_scope.owner(), make)?
                } else { make(__fusor_scope.owner())? };
                let state = __fusor_scope.retain_state(state);
                #expose_capture
                #javascript_inputs
                #install
                #javascript
                ::std::result::Result::Ok(__fusor_scope)
    }
}

/// Install every binding once the template is mounted. The runtime chooses the
/// bundled, hydrating, coherent or ordinary path.
fn install(
    component: &Component,
    typed_handles: TokenStream,
    bundle: Option<Vec<TokenStream>>,
    ctx: Ctx,
) -> TokenStream {
    let shared = component.render == RenderTarget::Shared;
    // A select's value applies after its options' own bindings set their values.
    let mut ordered: Vec<&Binding> = component.bindings.iter().collect();
    ordered.sort_by_key(|binding| {
        matches!(
            binding,
            Binding::Bind {
                control: Control::Select | Control::SelectMultiple,
                ..
            }
        )
    });
    let browser_bindings: Vec<_> = ordered
        .into_iter()
        .map(|item| browser_binding(item, shared, ctx, &component.async_locals))
        .collect();
    let shared_bindings = browser_bindings.iter().map(|binding| &binding.shared);
    let bindings = browser_bindings.iter().map(|binding| &binding.ordinary);
    let coherent_bindings = browser_bindings.iter().map(|binding| &binding.coherent);
    let adoptions = component
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Bind {
                node,
                control,
                value,
            } => Some(super::bind::adopt(*node, control, value)),
            _ => None,
        });
    let install = quote! {
        #typed_handles
        if __fusor_scope.is_hydrating() { #(#adoptions)* }
        #(#shared_bindings)*
        if __fusor_scope.is_coherent() {
            __fusor_scope.set_coherent_renderer(move |__fusor_frame| {
                let __fusor_attempt = __fusor_frame.attempt;
                #(#coherent_bindings)*
                ::std::result::Result::Ok(())
            });
        } else {
            #(#bindings)*
        }
    };
    if let Some(bindings) = bundle {
        quote! {
            if let ::std::option::Option::Some(__fusor_bundle) = __fusor_nodes.take_binding_bundle() {
                #(#bindings)*
            } else {
                #install
            }
        }
    } else {
        install
    }
}

/// Lints the browser lowering trips beyond the shared set.
fn allow_generated(span: Span) -> TokenStream {
    emit::allow_generated(
        span,
        quote! { , clippy::unused_unit, clippy::unit_arg, clippy::needless_ifs, clippy::needless_else },
    )
}

fn app_entry(expression: &Rust, prepare: &TokenStream) -> TokenStream {
    let span = expression.span();
    let allow = allow_generated(span);
    quote_spanned! {span=>
        #allow
        pub(crate) fn __fusor_mount() -> ::std::result::Result<(), ::fusor::dom::JsValue> {
            ::fusor_components::App::mount(|| {
                let parent = ::std::option::Option::None;
                let make = |owner: ::fusor::OwnerHandle| ::std::result::Result::<_, ::fusor::dom::JsValue>::Ok({ #expression });
                #prepare
            })
        }
        #[::wasm_bindgen::prelude::wasm_bindgen(start)]
        pub fn __fusor_start() -> ::std::result::Result<(), ::fusor::dom::JsValue> {
            __fusor_mount()
        }
    }
}

/// Fragments, content and rows are prepared inside their caller's code.
fn captured(prepare: &TokenStream) -> TokenStream {
    quote! {{
        let parent = ::std::option::Option::Some(__fusor_parent);
        let make = move |_owner| ::std::result::Result::<_, ::fusor::dom::JsValue>::Ok(state);
        #prepare
    }}
}

fn component_impl(
    component: &Component,
    server: Option<TokenStream>,
    html: &TokenStream,
    prepare: &TokenStream,
) -> TokenStream {
    let ty = &component.ty;
    let span = ty.span();
    let allow = allow_generated(span);
    let target = (component.render == RenderTarget::Shared)
        .then(|| quote! { #[cfg(target_arch = "wasm32")] });
    let template_impl = (component.kind() == RootKind::Template).then(|| {
        quote_spanned! {span=>
            #target
            impl ::fusor::dom::TemplateComponent for #ty {}
        }
    });
    let hash = delivery_hash(component);
    quote_spanned! {span=>
        #template_impl
        #server
        #target
        impl ::fusor::dom::Component for #ty {
            const TEMPLATE_HASH: &'static str = #hash;
            const TEMPLATE_HTML: &'static str = #html;
            #allow
            fn mount(self) -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue> {
                <Self as ::fusor::dom::Component>::try_mount_with(|_| ::std::result::Result::Ok(self))
            }
            #allow
            fn prepare_component(
                parent: ::std::option::Option<&::fusor::OwnerHandle>,
                make: ::fusor::dom::ComponentFactory<'_, Self>,
            ) -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue> {
                #prepare
            }
        }
    }
}

/// Prepare the case a branch selected, from the state held in `state`.
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

/// Prepare one ForEach row, whose state wraps the parent's state held in `state`.
/// A row proven never to read its index receives bare values, so that moving
/// it does not change its source. Other rows receive values with positions.
fn row_values(ctx: Ctx, body: usize) -> (TokenStream, TokenStream) {
    if ctx.components[body].item_only_row {
        (
            quote! { ::fusor_components::ForEach::values },
            quote! { ::fusor_components::ForEach::value_key },
        )
    } else {
        (
            quote! { ::fusor_components::ForEach::entries },
            quote! { ::fusor_components::ForEach::key },
        )
    }
}

fn foreach_row(span: Span, ctx: Ctx, body: usize, state: &str) -> TokenStream {
    let state = Ident::new(state, span);
    let constructor = if ctx.components[body].item_only_row {
        quote! { ::fusor_components::ForEach::item_row }
    } else {
        quote! { ::fusor_components::ForEach::row }
    };
    let row = ctx.component(body);
    quote_spanned! {span=>
        let state = #constructor(::std::rc::Rc::clone(&#state), entry);
        #row
    }
}

fn coherent_binding(binding: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let span = binding.span();
    match binding {
        Binding::Branch {
            point: id,
            value,
            cases,
            snapshots,
        } => {
            let slot = id.index();
            let point = point(*id);
            let selection = super::control::selection(value, cases, snapshots);
            let dispatch = branch_dispatch(span, ctx, cases, snapshots, "state");
            quote_spanned! {span=> __fusor_frame.branch_at(#slot, &#point, || { #selection },
            |__fusor_case, __fusor_data, __fusor_parent| {
                #dispatch
            })?; }
        }

        Binding::Children { point: id, .. } => {
            let slot = id.index();
            let point = point(*id);
            quote! { __fusor_frame.children_at(#slot, &#point, &__fusor_children)?; }
        }
        Binding::ForEach {
            node,
            items,
            key,
            body,
        } => {
            let slot = node.index();
            let node = element(*node);
            let row = foreach_row(span, ctx, *body, "state");
            let (values, value_key) = row_values(ctx, *body);
            quote_spanned! {span=> __fusor_frame.keyed(#slot, #node.as_ref(), || #values({ #items }),
            |entry| #value_key(entry, #key), |entry, __fusor_parent| {
                #row
            })?; }
        }
        Binding::Invocation { children, .. } => {
            let children = children_factory(*children, ctx);
            invocation(binding, children, true, ctx, locals)
        }
        Binding::Region {
            value,
            kind: RegionKind::Await { alias },
            bindings,
            ..
        } => {
            let mut nested = locals.to_vec();
            if let Some(alias) = alias {
                nested.push(alias.clone());
            }
            let resolved = emit::or(alias.as_ref(), quote! { ready });
            let inner = Ctx {
                ready: ctx.ready || alias.is_none(),
                ..ctx
            };
            let bindings = bindings
                .iter()
                .map(|binding| coherent_binding(binding, inner, &nested));
            quote_spanned! {span=>
                if let ::fusor_async::AsyncRead::Ready(#resolved) = (#value).read(__fusor_attempt)? {
                    #(#bindings)*
                }
            }
        }
        Binding::Region { .. } => emit::reject(span, "nested coherent boundaries are unsupported"),
        Binding::Text { slot, value } => {
            let node = text(*slot);
            quote_spanned! {span=> __fusor_frame.text(&#node, &(#value))?; }
        }
        Binding::Attribute { node, name, value } => {
            let node = element(*node);
            let value = string(value);
            quote_spanned! {span=> __fusor_frame.attr(#node.as_ref(), #name, ::std::option::Option::Some(#value))?; }
        }
        Binding::Boolean { node, name, value } => {
            let node = element(*node);
            quote_spanned! {span=> __fusor_frame.attr(#node.as_ref(), #name, ({ #value }).then(::std::string::String::new))?; }
        }
        Binding::Class { node, name, value } => {
            let node = element(*node);
            quote_spanned! {span=> __fusor_frame.class(#node.as_ref(), #name, { #value })?; }
        }
        Binding::Event {
            node,
            name,
            handler,
        } => {
            let node = element(*node);
            let ready = ctx.clone_ready();
            let locals = clone_locals(locals);
            quote_spanned! {span=> {
                #locals
                let state = ::std::rc::Rc::clone(&state);
                #ready
                __fusor_frame.on(#node.as_ref(), #name, move |event| { #handler })?;
            }}
        }
        Binding::Router { .. }
        | Binding::Island { .. }
        | Binding::Property { .. }
        | Binding::Value { .. }
        | Binding::Checked { .. }
        | Binding::Bind { .. }
        | Binding::Slot { .. } => emit::reject(
            span,
            "editable controls, widgets, outlets and opaque content must remain outside coherent regions",
        ),
    }
}

fn delivery_html(component: &Component) -> Ident {
    emit::indexed_constant("TEMPLATE_HTML", component.id.index())
}

fn delivery_hash(component: &Component) -> Ident {
    emit::indexed_constant("TEMPLATE_HASH", component.id.index())
}

/// Delivery ships each template's HTML and hash inside the Wasm. They are
/// constants written after the executable code, so the refresh fingerprint (the
/// code without them) stays the same when only HTML changes.
pub(super) fn delivery(components: &[Component]) -> TokenStream {
    let constants = components
        .iter()
        .filter(|component| !component.fragment())
        .map(|component| {
            let name = delivery_html(component);
            let html = &component.html;
            let hash = matches!(component.shape, ComponentShape::Declared(_)).then(|| {
                let name = delivery_hash(component);
                let hash = super::server::hash(component, components);
                quote! { #[allow(dead_code)] const #name: &str = #hash; }
            });
            quote! { #[allow(dead_code)] const #name: &str = #html; #hash }
        });
    quote! { #(#constants)* }
}

pub(super) fn generate(
    source: &str,
    components: &[Component],
    rust: &mut String,
) -> Vec<BindingLocation> {
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
    let ctx = Ctx {
        components,
        ready: false,
    };
    let mut locations = Vec::new();
    for item in components
        .iter()
        .filter(|item| !item.inline() && item.capture().is_none())
    {
        locations.extend(tokens::emit(
            source,
            rust,
            component(item, ctx),
            &origins,
            item.ty.offset,
        ));
    }
    locations
}
