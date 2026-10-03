//! Coherent frame emission and shared await factories.
use super::*;

pub(super) fn shared_await(
    item: &Binding,
    node: ElementId,
    ctx: Ctx,
    locals: &[Rust],
) -> BindingCode {
    let span = item.span();
    let render = indexed("await_render", node.index());
    let node = element(node);
    let render_body = coherent_renderer(binding(item, ctx, locals), ctx);
    let captures = captures(span, ctx, locals);
    BindingCode {
        shared: quote_spanned! {span=>
            let #render = {
                #captures
                let #node = #node.clone();
                #render_body
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
    if let Some(message) = coherent_rejection(binding) {
        return reject(span, message);
    }
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
            quote_spanned! {span=> __fusor_frame.keyed(::fusor::dom::coherent::RenderSlot { index: #slot, target: #node.as_ref() }, || #values({ #items }),
            |entry| #value_key(entry, #key), |entry, __fusor_parent| {
                #row
            })?; }
        }
        Binding::Invocation { children, .. } => {
            let children = children_factory(*children, ctx);
            invocation(binding, children, ctx)
        }
        Binding::Region {
            kind: RegionKind::Await { .. },
            ..
        } => await_binding(binding, ctx, locals, self::binding),
        _ => value_binding(binding, ctx, locals),
    }
}

fn value_binding(binding: &Binding, ctx: Ctx, locals: &[Rust]) -> TokenStream {
    let span = binding.span();
    match binding {
        Binding::Text { slot, value } => {
            let node = text(*slot);
            quote_spanned! {span=> {
                let __fusor_value = &(#value);
                __fusor_frame.text(&#node, __fusor_value)?;
            } }
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
                #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(&state);
                #ready
                __fusor_frame.on(#node.as_ref(), #name, move |#[allow(unused_variables, reason = "handlers may ignore the event")] event| { #handler })?;
            }}
        }
        Binding::Branch { .. }
        | Binding::ForEach { .. }
        | Binding::Invocation { .. }
        | Binding::Children { .. }
        | Binding::Router { .. }
        | Binding::Island { .. }
        | Binding::Property { .. }
        | Binding::Value { .. }
        | Binding::Checked { .. }
        | Binding::Bind { .. }
        | Binding::Slot { .. }
        | Binding::Region { .. } => unreachable!("rejected coherent binding"),
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
    let identity = emit::identity(condition.as_ref(), key.as_ref());
    let construct = construct_inputs(span, ty, inputs, ctx);
    quote_spanned! {span=> __fusor_frame.component_at(::fusor::dom::coherent::RenderSlot { index: #slot, target: &#point }, #identity, |owner| {
        #construct
    }, #children)?; }
}

/// Refuse a binding a coherent frame cannot render.
fn reject(span: Span, message: &str) -> TokenStream {
    let message = literal(span, message);
    quote_spanned! {span=> __fusor_frame.reject(#message)?; }
}
