//! Coherent frame emission and shared await factories.
use super::*;

pub(super) fn shared_await(
    item: &Binding,
    node: ElementId,
    ctx: Ctx,
    locals: &[Rust],
) -> BrowserBinding {
    let span = item.span();
    let render = indexed("await_render", node.index());
    let node = element(node);
    let body = binding(item, ctx, locals);
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

pub(super) fn binding(binding: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
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
            let selection = crate::bindings::control::selection(value, cases, snapshots);
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
            let (values, value_key, row) = foreach_parts(span, ctx, *body, "state");
            quote_spanned! {span=> __fusor_frame.keyed(#slot, #node.as_ref(), || #values({ #items }),
            |entry| #value_key(entry, #key), |entry, __fusor_parent| {
                #row
            })?; }
        }
        Binding::Invocation { children, .. } => {
            let children = children_factory(*children, ctx);
            invocation(binding, children, ctx)
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
                .map(|binding| self::binding(binding, inner, &nested));
            quote_spanned! {span=>
                if let ::fusor_async::AsyncRead::Ready(#resolved) = (#value).read(__fusor_attempt)? {
                    #(#bindings)*
                }
            }
        }
        Binding::Region { .. } => reject(span, "nested coherent boundaries are unsupported"),
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
        | Binding::Slot { .. } => reject(
            span,
            "editable controls, widgets, outlets and opaque content must remain outside coherent regions",
        ),
    }
}

pub(super) fn invocation(binding: &Binding, children: TokenStream, ctx: Ctx) -> TokenStream {
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
    if has_content(inputs) {
        return reject(
            span,
            "projected content cannot participate in coherent rendering",
        );
    }
    let slot = id.index();
    let point = point(*id);
    let condition = emit::or(condition.as_ref(), quote! { true });
    let key = emit::or(key.as_ref(), quote! { () });
    let construct = construct_inputs(span, ty, inputs, ctx);
    quote_spanned! {span=> __fusor_frame.component_at(#slot, &#point, if #condition { ::std::option::Option::Some({ #key }) } else { ::std::option::Option::None }, |owner| {
        #construct
    }, #children)?; }
}

/// Refuse a binding a coherent frame cannot render.
fn reject(span: Span, message: &str) -> TokenStream {
    let message = literal(span, message);
    quote_spanned! {span=> __fusor_frame.reject(#message)?; }
}
