//! Ordinary binding emission, control adoption and installation across DOM modes.
use super::*;

pub(super) fn emit(binding: &Binding, ctx: Ctx<'_>, locals: &[Rust]) -> TokenStream {
    let span = binding.span();
    let operation = match binding {
        Binding::Branch { .. }
        | Binding::ForEach { .. }
        | Binding::Invocation { .. }
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
        Binding::Island { .. } => fail(span, "independent islands must be rendered by the server"),
        Binding::Region {
            kind: RegionKind::Await { alias: None },
            ..
        } => fail(
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
                .map(|binding| coherent::binding(binding, ctx, locals));
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
        } => control_binding(*node, control, value, ControlMode::Install),
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

// The typed path changes the generated closure's result representation. Keep
// String results when authored code can return from that closure; opaque macros
// and attributes may introduce a return after expansion.
fn typed_text_eligible(value: &Rust) -> bool {
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

fn browser_binding(item: &Binding, shared: bool, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
    match item {
        Binding::Branch { .. } | Binding::ForEach { .. } => {
            shared_structure(item, shared, ctx, locals)
        }
        // Named Content has only an ordinary path, and its creation supplies
        // retained slot identity. Only share factories used by both modes.
        Binding::Invocation {
            point,
            inputs,
            children: Some(child),
            ..
        } if !ctx.components[*child].empty && !has_content(inputs) => {
            shared_children(item, *point, *child, ctx, locals)
        }
        Binding::Region {
            node,
            kind: RegionKind::Await { alias: Some(_) },
            ..
        } => coherent::shared_await(item, *node, ctx, locals),
        _ => BrowserBinding {
            shared: TokenStream::new(),
            ordinary: codegen::binding(item, ctx, locals),
            coherent: coherent::binding(item, ctx, locals),
        },
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
        ordinary: codegen::invocation(
            item,
            quote! {{ let __fusor_make_children = #make; __fusor_make_children() }},
            ctx,
            locals,
        ),
        coherent: coherent::invocation(item, quote! { #make() }, ctx),
    }
}

/// Each route body prepares without inheriting an enclosing await's `ready`.
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

enum ControlMode {
    Install,
    Adopt,
}

fn control_binding(
    node: ElementId,
    control: &Control,
    value: &Rust,
    mode: ControlMode,
) -> TokenStream {
    let node = emit::element(node);
    let (function, capture) = match mode {
        ControlMode::Install => (format_ident!("{}", control.kind()), Some(quote! { move })),
        ControlMode::Adopt => (format_ident!("adopt_{}", control.kind()), None),
    };
    let choice = control.choice().map(|choice| {
        let choice = emit::string(choice);
        quote! { #capture || #choice, }
    });
    match mode {
        // Clone the bound value before the choice closure takes `state`.
        ControlMode::Install => quote_spanned! {value.span()=> {
            let __fusor_bound = ::std::clone::Clone::clone(&(#value));
            ::fusor::dom::controls::#function(&mut __fusor_scope, &#node, #choice __fusor_bound)?;
        }},
        ControlMode::Adopt => quote_spanned! {value.span()=>
            ::fusor::dom::controls::#function(&__fusor_scope, &#node, #choice &(#value))?;
        },
    }
}

/// Call a typed scope method, or its bundle counterpart using descriptor ordinals.
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

/// Text, attribute and event bindings share ordinary and bundle installation.
/// Other bindings require typed handles, managed lifetimes or a coherent frame.
pub(super) fn scoped(binding: &Binding, bundle: Option<&Bundle>) -> Option<TokenStream> {
    let span = binding.span();
    let anchor = binding.anchor();
    let (method, arguments) = match binding {
        Binding::Text { value, .. } if typed_text_eligible(value) => {
            ("text_node_value", typed_text_closure(span, value))
        }
        Binding::Text { value, .. } => (
            "text_node_string",
            quote_spanned! {span=> move || ::std::string::ToString::to_string(&(#value)) },
        ),
        // A lone interpolation shares the text path's exact-integer conversion.
        Binding::Attribute { name, value, .. } => match value.as_expression() {
            Some(expression) if typed_text_eligible(expression) => {
                let read = typed_text_closure(span, expression);
                ("attr_value", quote_spanned! {span=> #name, #read })
            }
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

fn typed_text_closure(span: Span, value: &Rust) -> TokenStream {
    quote_spanned! {span=> move || {
        use ::fusor::dom::text_value::Convert as _;
        (&::fusor::dom::text_value::Value(&(#value))).__fusor_into_text()
    }}
}

/// The runtime selects bundled, hydrating, coherent or ordinary installation.
pub(super) fn install(
    component: &Component,
    typed_handles: TokenStream,
    bundle: Option<Vec<TokenStream>>,
    ctx: Ctx,
) -> TokenStream {
    let shared = component.render == RenderTarget::Shared;
    // A select's value applies after its options' own bindings set their values.
    let mut code = BrowserBinding::default();
    for item in order_bindings(&component.bindings) {
        let next = browser_binding(item, shared, ctx, &component.async_locals);
        code.shared.extend(next.shared);
        code.ordinary.extend(next.ordinary);
        code.coherent.extend(next.coherent);
    }
    let BrowserBinding {
        shared,
        ordinary,
        coherent,
    } = code;
    let adoptions = component
        .bindings
        .iter()
        .filter_map(|binding| match binding {
            Binding::Bind {
                node,
                control,
                value,
            } => Some(control_binding(*node, control, value, ControlMode::Adopt)),
            _ => None,
        });
    let install = quote! {
        #typed_handles
        if __fusor_scope.is_hydrating() { #(#adoptions)* }
        #shared
        if __fusor_scope.is_coherent() {
            __fusor_scope.set_coherent_renderer(move |__fusor_frame| {
                let __fusor_attempt = __fusor_frame.attempt;
                #coherent
                ::std::result::Result::Ok(())
            });
        } else {
            #ordinary
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

fn shared_structure(item: &Binding, shared: bool, ctx: Ctx, locals: &[Rust]) -> BrowserBinding {
    let span = item.span();
    let (setup, operation) = match item {
        Binding::Branch { .. } => codegen::branch(item, ctx, locals),
        Binding::ForEach { .. } => codegen::list(item, ctx, locals),
        _ => unreachable!("branch or keyed list"),
    };
    let anchor = emit::handle(item.anchor());
    let (ordinary, coherent) = match operation {
        OperationKind::Branch { read, prepare } => {
            let Anchor::Mount(id) = item.anchor() else {
                unreachable!("branch mount")
            };
            let slot = id.index();
            (
                quote_spanned! {span=> __fusor_scope.branch_at(&#anchor, #read, #prepare)?; },
                quote_spanned! {span=> __fusor_frame.branch_at(#slot, &#anchor, &#read, &#prepare)?; },
            )
        }
        OperationKind::Keyed { read, key, prepare } => {
            let Anchor::Element(id) = item.anchor() else {
                unreachable!("list element")
            };
            let slot = id.index();
            let row = quote! { move |entry| #prepare(entry, &__fusor_parent) };
            let ordinary = if shared {
                quote! { __fusor_scope.keyed_hydrated(&#anchor, #read, #key, #row, |key| ::fusor_islands::encode(key).map_err(|error| ::fusor::dom::JsValue::from_str(&error.to_string())))?; }
            } else {
                quote! { __fusor_scope.keyed(&#anchor, #read, #key, #row)?; }
            };
            (
                quote_spanned! {span=> {
                    let __fusor_parent = __fusor_scope.owner();
                    #ordinary
                } },
                quote_spanned! {span=> __fusor_frame.keyed(#slot, #anchor.as_ref(), &#read, &#key, &#prepare)?; },
            )
        }
        _ => unreachable!("structural operation"),
    };
    BrowserBinding {
        shared: setup,
        ordinary,
        coherent,
    }
}

/// Fail the mount of a binding the browser cannot install.
fn fail(span: Span, message: &str) -> TokenStream {
    let message = literal(span, message);
    quote_spanned! {span=> return ::std::result::Result::Err(::fusor::dom::JsValue::from_str(#message)); }
}
