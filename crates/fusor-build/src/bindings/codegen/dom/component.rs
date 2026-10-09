//! Component preparation, mounting implementations and static delivery constants.
use super::*;

pub(super) fn emit(component: &Component, ctx: Ctx<'_>) -> TokenStream {
    let server = (component.render != RenderTarget::Browser && component.capture().is_none())
        .then(|| crate::bindings::server::component(component, ctx.components));
    if component.render == RenderTarget::Server && component.capture().is_none() {
        return quote! { #server };
    }
    if let Some(forward) = forwarding_row(component, ctx) {
        return forward;
    }
    // Fragments and independently compiled libraries mount from their own HTML.
    let html = if matches!(component.shape, ComponentShape::Fragment(_)) {
        let html = &component.html;
        quote! { #html }
    } else {
        let name = emit::indexed_constant("TEMPLATE_HTML", component.id.index());
        quote! { #name }
    };
    let prepare = prepare(component, &html, ctx);
    match &component.shape {
        ComponentShape::App(expression) => app_entry(expression, &prepare),
        ComponentShape::Row | ComponentShape::Fragment(_) | ComponentShape::Content(_) => {
            captured(&prepare, ctx)
        }
        ComponentShape::Declared(_) => component_impl(component, server, &html, &prepare),
    }
}

/// A row containing only a component tag mounts that component directly.
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
    let construct = codegen::construct_inputs(Span::call_site(), ty, inputs, ctx);
    let supplied = children_factory(children, ctx);
    Some(quote! {{
        #local_clones
        #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::new(state);
        let __fusor_supplied = #supplied;
        __fusor_supplied.with(|| <#ty as ::fusor::dom::Component>::prepare(__fusor_parent, move |owner| {
            #construct
        }))
    }})
}

/// Mount the template, construct its state and install bindings in existing order.
fn prepare(component: &Component, html: &TokenStream, ctx: Ctx) -> TokenStream {
    let local_clones = clone_locals(&component.async_locals);
    let template::TemplateCode {
        declarations,
        typed_handles,
        bundle,
    } = template::lower(component, ctx);
    let expose_capture = component.capture().map(|_| {
        quote! {
            #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::std::rc::Rc::clone(state.as_ref());
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
    let mount = if ctx.embedded_templates && !component.fragment() {
        let bundled = bundle.is_some();
        quote! { __FUSOR_TEMPLATE.prepare_embedded({ #template_html }, __FUSOR_MOUNTS, parent, #bundled)? }
    } else if bundle.is_some() {
        quote! { __FUSOR_TEMPLATE.prepare_with_binding_bundle({ #template_html }, parent)? }
    } else {
        quote! { __FUSOR_TEMPLATE.#mount_method({ #template_html }, __FUSOR_MOUNTS, parent)? }
    };
    let install = bindings::install(component, typed_handles, bundle, ctx);
    let incoming = incoming_children(component, ctx);
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
                #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = ::fusor::render::construct(&mut __fusor_scope, make)?;
                #expose_capture
                #javascript_inputs
                #install
                #javascript
                ::std::result::Result::Ok(__fusor_scope)
    }
}

fn app_entry(expression: &Rust, prepare: &TokenStream) -> TokenStream {
    let span = expression.span();
    quote_spanned! {span=>
        #[cfg(not(fusor_worker))]
        pub(crate) fn __fusor_mount() -> ::std::result::Result<(), ::fusor::dom::JsValue> {
            ::fusor_components::App::mount(|| {
                let parent = ::std::option::Option::None;
                let make = |#[allow(unused_variables, reason = "application constructors may ignore the owner")] owner: ::fusor::OwnerHandle| ::std::result::Result::<_, ::fusor::dom::JsValue>::Ok({ #expression });
                #prepare
            })
        }
        #[cfg(not(fusor_worker))]
        #[::wasm_bindgen::prelude::wasm_bindgen(start)]
        pub fn __fusor_start() -> ::std::result::Result<(), ::fusor::dom::JsValue> {
            __fusor_mount()
        }
    }
}

fn component_impl(
    component: &Component,
    server: Option<TokenStream>,
    html: &TokenStream,
    prepare: &TokenStream,
) -> TokenStream {
    let ty = &component.ty;
    let span = ty.span();
    let target = (component.render == RenderTarget::Shared)
        .then(|| quote! { #[cfg(target_arch = "wasm32")] });
    let fragment = component.kind() == RootKind::Fragment;
    let template_impl = (component.kind() != RootKind::Existing).then(|| {
        quote_spanned! {span=>
            #target
            impl ::fusor::dom::TemplateComponent for #ty {}
        }
    });
    let hash = emit::indexed_constant("TEMPLATE_HASH", component.id.index());
    quote_spanned! {span=>
        #template_impl
        #server
        #target
        impl ::fusor::dom::Component for #ty {
            const FRAGMENT: bool = #fragment;
            const TEMPLATE_HASH: &'static str = #hash;
            const TEMPLATE_HTML: &'static str = #html;
            fn mount(self) -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue> {
                <Self as ::fusor::dom::Component>::try_mount_with(|_| ::std::result::Result::Ok(self))
            }
            fn prepare_component(
                parent: ::std::option::Option<&::fusor::OwnerHandle>,
                make: ::fusor::dom::ComponentFactory<'_, Self>,
            ) -> ::std::result::Result<::fusor::dom::Scope, ::fusor::dom::JsValue> {
                #prepare
            }
        }
    }
}

/// Append HTML/hash constants after executable code so static HTML changes do
/// not change the refresh fingerprint. Web document/asset delivery stays separate.
pub(in crate::bindings) fn delivery(components: &[Component]) -> TokenStream {
    let constants = components
        .iter()
        .filter(|component| !matches!(component.shape, ComponentShape::Fragment(_)))
        .map(|component| {
            let name = emit::indexed_constant("TEMPLATE_HTML", component.id.index());
            let html = &component.html;
            let hash = matches!(component.shape, ComponentShape::Declared(_)).then(|| {
                let name = emit::indexed_constant("TEMPLATE_HASH", component.id.index());
                let hash = crate::bindings::server::hash(component, components);
                quote! { #[allow(dead_code, reason = "template metadata is used only by matching rendering modes")] const #name: &str = #hash; }
            });
            quote! { #[allow(dead_code, reason = "template metadata is used only by matching rendering modes")] const #name: &str = #html; #hash }
        });
    quote! { #(#constants)* }
}
