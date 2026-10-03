//! Native lowering walks a structural HTML token stream. Dynamic Rust remains
//! native token trees with the same source spans as the browser target.
use super::{ir::*, tokens::Rust};
use fusor::template::{self, ElementId, MountId, MountMarker, RootKind, TextId, TextMarker};
use html5gum::{EndTag, HtmlString, Spanned, StartTag, Token};
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use sha2::{Digest, Sha256};

pub(super) fn hash(component: &Component, components: &[Component]) -> String {
    let mut hash = Sha256::new();
    hash.update(component.html.as_bytes());
    for fragment in component.bindings.iter().flat_map(Binding::fragments) {
        hash.update(fragment.tokens.to_string());
        hash.update([0]);
    }
    for binding in Binding::walk(&component.bindings) {
        if let Binding::Router { routes, .. } = binding {
            for route in routes {
                hash.update(route.path.as_deref().unwrap_or("<fallback>").as_bytes());
                hash.update([0]);
                if let Some(alias) = &route.params {
                    hash.update(alias.tokens.to_string());
                }
            }
        }
        for child in binding.components() {
            hash.update(self::hash(&components[child], components));
        }
    }
    format!("{:x}", hash.finalize())
}

fn construct_child(
    ty: &Rust,
    inputs: &[Input],
    children: Option<usize>,
    components: &[Component],
    into: bool,
) -> TokenStream {
    let fields = super::emit::fields(inputs, super::emit::braced);
    let body = children
        .filter(|index| !components[*index].empty)
        .map(|index| component_body(&components[index], components, false));
    let content = super::emit::option(body.map(|body| quote! { &(|__fusor_context: &mut ::fusor_server::Context<'_>| { #body }) as &::fusor_server::Children<'_> }));
    let method = if into {
        quote! { try_child_into_with_children }
    } else {
        quote! { try_child_with_children }
    };
    let writer = into.then(|| quote! { , __fusor_writer });
    let convert = quote! { |_| ::std::string::String::from(concat!("component ", stringify!(#ty), " input construction failed")) };
    let construct = super::emit::construct_inputs(ty.span(), ty, fields, convert);
    quote_spanned! {ty.span()=> __fusor_context.#method(|owner| {
        #construct
    }, #content #writer) }
}

/// Accumulate only static syntax. A dynamic statement flushes the exact prefix
/// before evaluating authored Rust, preserving output and error ordering.
#[derive(Default)]
struct Emission {
    statements: Vec<TokenStream>,
    markup: String,
    first_open: Option<usize>,
    editable: bool,
}

impl Emission {
    fn literal(&mut self, value: &str) {
        self.markup.push_str(value);
    }

    fn open(&mut self, tag: &str) {
        self.literal("<");
        self.literal(tag);
        self.first_open.get_or_insert(self.markup.len());
        self.editable |= matches!(tag, "input" | "textarea" | "select");
    }

    fn flush(&mut self) {
        if self.markup.is_empty() {
            return;
        }
        let markup = std::mem::take(&mut self.markup);
        let first_open =
            super::emit::option(self.first_open.take().map(|offset| quote! { #offset }));
        let editable = std::mem::take(&mut self.editable);
        self.statements.push(quote! {
            __fusor_writer.static_markup(#markup, #first_open, #editable);
        });
    }

    fn push(&mut self, statement: TokenStream) {
        if !statement.is_empty() {
            self.flush();
            self.statements.push(statement);
        }
    }

    fn finish(mut self) -> Vec<TokenStream> {
        self.flush();
        self.statements
    }
}

fn component_body(component: &Component, components: &[Component], into: bool) -> TokenStream {
    if let Some(child) = forwarding_child(component, components, into) {
        return child;
    }
    let mut render = ServerRender {
        component,
        components,
        body: Emission::default(),
        depth: 0,
        raw: false,
        first: true,
        select: None,
    };
    if component.fragment() {
        render.body.literal("<!--fusor:fragment-->");
    }
    let tokens: Vec<_> = crate::html::tokens(&component.html).collect();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::StartTag(tag) => render.start_tag(tag, &tokens[index + 1..]),
            Token::EndTag(tag) => render.end_tag(tag),
            Token::String(value) => render.text(value),
            Token::Comment(comment) => render.comment(comment),
            _ => {}
        }
    }
    if component.fragment() {
        render.body.literal("<!--/fusor:fragment-->");
    }
    let body = render.body.finish();
    if into {
        quote! {{ #(#body)* ::std::result::Result::<(), ::std::string::String>::Ok(()) }}
    } else {
        quote! {{
            let mut __fusor_writer = ::fusor_server::Writer::new();
            #(#body)*
            ::std::result::Result::<_, ::std::string::String>::Ok(__fusor_writer.finish())
        }}
    }
}

/// A row whose HTML is a single component tag renders that child directly.
fn forwarding_child(
    component: &Component,
    components: &[Component],
    into: bool,
) -> Option<TokenStream> {
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
    Some(construct_child(ty, inputs, *children, components, into))
}

/// Walks a component's compiled HTML and emits the Rust that writes it on the server.
struct ServerRender<'a> {
    component: &'a Component,
    components: &'a [Component],
    body: Emission,
    depth: i32,
    /// Inside script or style, whose text is written unescaped.
    raw: bool,
    /// The next start tag is the component's root.
    first: bool,
    /// The bound select whose options are being written.
    select: Option<super::bind::Select<'a>>,
}

impl<'a> ServerRender<'a> {
    /// `following` holds the tokens after this tag, where an option finds its text.
    fn start_tag(&mut self, tag: &StartTag<usize>, following: &[Token<usize>]) {
        let component = self.component;
        let name = String::from_utf8_lossy(&tag.name).into_owned();
        if name == "template" && self.depth == 0 && component.kind() == RootKind::Template {
            self.depth += 1;
            return;
        }
        let bindings = self.element_bindings(tag);
        let island = bindings
            .iter()
            .copied()
            .find(|binding| matches!(binding, Binding::Island { .. }));
        if let Some(island) = island {
            self.body.push(island_prelude(tag, &bindings, island));
        }
        let element = ServerElement {
            name: &name,
            sensitive: sensitive_input(tag),
            island: island.is_some(),
        };
        self.body.open(&name);
        self.static_attributes(tag, &bindings, &element);
        let mut content = Vec::new();
        if let Some(slot) = tag
            .attributes
            .get(template::TEXT_ELEMENT_ATTRIBUTE.as_bytes())
            .and_then(|id| String::from_utf8_lossy(id).parse::<TextId>().ok())
        {
            content.extend(self.text_write(slot));
        }
        for binding in &bindings {
            let operation = self.operation(binding, &element, &mut content);
            self.body.push(operation);
        }
        if bindings
            .iter()
            .any(|binding| matches!(binding, Binding::Class { .. }))
        {
            self.body.push(classes(tag, &bindings));
        }
        if name == "option" {
            if let Some(select) = &self.select {
                self.body.push(select.option(tag, &bindings, following));
            }
        }
        if let Some(select) = super::bind::Select::open(&bindings) {
            self.select = Some(select);
        }
        self.body.literal(">");
        self.body.push(quote! { #(#content)* });
        self.raw = matches!(name.as_str(), "script" | "style");
        if !super::tags::void_element(&name) {
            self.depth += 1;
        }
    }

    fn element_bindings(&self, tag: &StartTag<usize>) -> Vec<&'a Binding> {
        let id = tag
            .attributes
            .get(template::ELEMENT_ATTRIBUTE.as_bytes())
            .and_then(|id| String::from_utf8_lossy(id).parse::<ElementId>().ok());
        self.component
            .bindings
            .iter()
            .filter(|binding| id.is_some_and(|id| binding.anchor() == Anchor::Element(id)))
            .collect()
    }

    /// Authored attributes, minus those a binding writes, plus the root's identity.
    fn static_attributes(
        &mut self,
        tag: &StartTag<usize>,
        bindings: &[&Binding],
        element: &ServerElement,
    ) {
        let component = self.component;
        let mut static_attributes = String::new();
        let mut editable = false;
        for (key, value) in &tag.attributes {
            let key = String::from_utf8_lossy(key).into_owned();
            let value = String::from_utf8_lossy(value).into_owned();
            if element.sensitive && key == "value"
                || element.island && key == "id"
                || key == "class"
                    && bindings
                        .iter()
                        .any(|binding| matches!(binding, Binding::Class { .. }))
            {
                continue;
            }
            editable |= static_attribute(&mut static_attributes, &key, &value);
        }
        if self.first && component.kind() == RootKind::Template && !component.fragment() {
            let id = component.id.to_string();
            let version = template::VERSION.to_string();
            static_attribute(&mut static_attributes, template::COMPONENT_ATTRIBUTE, &id);
            static_attribute(
                &mut static_attributes,
                template::VERSION_ATTRIBUTE,
                &version,
            );
        }
        self.body.literal(&static_attributes);
        self.body.editable |= editable;
        self.first = false;
    }

    /// The statement a binding adds to its element's opening tag; content it writes
    /// between the tags goes to `content`.
    fn operation(
        &self,
        binding: &Binding,
        element: &ServerElement,
        content: &mut Vec<TokenStream>,
    ) -> TokenStream {
        let components = self.components;
        let span = binding.span();
        match binding {
            Binding::Attribute { name, value, .. } if !element.island || name != "id" => {
                let value = super::emit::format_args(value);
                quote_spanned! {span=> __fusor_writer.attr(#name, #value); }
            }
            Binding::Boolean { name, value, .. } => {
                quote_spanned! {span=> __fusor_writer.boolean(#name, { #value }); }
            }
            Binding::Checked { value, .. } => {
                quote_spanned! {span=> __fusor_writer.boolean("checked", { #value }); }
            }
            Binding::Value { value, .. } if !element.sensitive => {
                let value = super::emit::format_args(value);
                quote_spanned! {span=> __fusor_writer.attr("value", #value); }
            }
            Binding::Bind { control, value, .. } => {
                super::bind::server(element.name, element.sensitive, control, value, content)
            }
            Binding::ForEach {
                items,
                key,
                body: row,
                ..
            } => {
                content.push(server_list(&components[*row], components, items, key));
                quote! {}
            }
            Binding::Island { .. } => {
                // The same lowering serves owned root Writers and
                // borrowed nested Writers; the latter reborrow is
                // intentional. Scope the lint to framework calls.
                content.push(
                    quote_spanned! {span=> #[allow(clippy::needless_borrow, reason = "nested writers are borrowed while outer writers are owned")] __fusor_island.contents(&mut __fusor_writer); },
                );
                quote! { #[allow(clippy::needless_borrow, reason = "nested writers are borrowed while outer writers are owned")] __fusor_island.attributes(&mut __fusor_writer); }
            }
            // Guarded values omit sensitive inputs and island-owned IDs.
            Binding::Attribute { .. } | Binding::Value { .. }
            // Text, classes and structural anchors are emitted separately.
            | Binding::Text { .. } | Binding::Class { .. } | Binding::Branch { .. }
            | Binding::Children { .. } | Binding::Invocation { .. }
            // Events intentionally have no server effect; these browser-only
            // operations are rejected by template validation where applicable.
            | Binding::Event { .. } | Binding::Property { .. } | Binding::Region { .. }
            | Binding::Router { .. } | Binding::Slot { .. } => quote! {},
        }
    }

    fn text_write(&self, slot: TextId) -> Option<TokenStream> {
        self.component
            .bindings
            .iter()
            .find_map(|binding| match binding {
                Binding::Text { slot: text, value } if *text == slot => {
                    Some(quote_spanned! {value.span()=> {
                        let __fusor_value = &(#value);
                        __fusor_writer.text(__fusor_value);
                    } })
                }
                _ => None,
            })
    }

    fn end_tag(&mut self, tag: &EndTag<usize>) {
        self.depth -= 1;
        let name = String::from_utf8_lossy(&tag.name).into_owned();
        if name == "template" && self.depth == 0 && self.component.kind() == RootKind::Template {
            return;
        }
        self.body.literal(&format!("</{name}>"));
        self.raw = false;
        if name == "select" {
            self.select = None;
        }
    }

    fn text(&mut self, value: &Spanned<HtmlString, usize>) {
        let component = self.component;
        // Browser template mounting selects the single element root.
        // Formatting outside that root is not part of the component.
        if component.kind() == RootKind::Template && !component.fragment() && self.depth <= 1 {
            return;
        }
        let value = String::from_utf8_lossy(value).into_owned();
        let value = if self.raw {
            value
        } else {
            let mut escaped = String::new();
            template::escape_into(&mut escaped, &value, false);
            escaped
        };
        self.body.literal(&value);
    }

    fn comment(&mut self, comment: &Spanned<HtmlString, usize>) {
        let component = self.component;
        if component.kind() == RootKind::Template && !component.fragment() && self.depth <= 1 {
            return;
        }
        let value = String::from_utf8_lossy(comment);
        let literal = format!("<!--{value}-->");
        self.body.literal(&literal);
        if let Ok(Some(MountMarker::Start(id))) = MountMarker::parse(&value) {
            self.mount_point(id);
        }
        if let Ok(Some(TextMarker::Start(id))) = TextMarker::parse(&value) {
            if let Some(write) = self.text_write(id) {
                self.body.push(write);
            }
        }
    }

    /// Render whatever the browser would mount at this point: a branch, the
    /// caller's children or a child component.
    fn mount_point(&mut self, id: MountId) {
        let component = self.component;
        let components = self.components;
        if let Some(Binding::Branch { value, cases, .. }) = component
            .bindings
            .iter()
            .find(|binding| matches!(binding, Binding::Branch { point, .. } if *point == id))
        {
            let arms = cases.iter().enumerate().map(|(index, case)| {
                let pattern = &case.pattern;
                let captures = case
                    .names
                    .iter()
                    .map(|name| quote! { let #name = ::fusor::memo(move || #name.clone()); });
                let child = component_body(&components[case.body], components, true);
                let marker = format!("<!--fusor:branch:{index}-->");
                quote! { #pattern => { #(#captures)* __fusor_writer.static_markup(#marker, ::std::option::Option::None, false); #child?; } }
            });
            self.body.push(quote_spanned! {value.span()=> { let __fusor_value = { #value }; #[deny(non_snake_case)] match __fusor_value { #(#arms),* } } });
        }
        if component
            .bindings
            .iter()
            .any(|binding| matches!(binding, Binding::Children { point, .. } if *point == id))
        {
            self.body.push(quote! { if let ::std::option::Option::Some(children) = __fusor_children { let child = children(__fusor_context)?; __fusor_writer.child(&child); } });
        }
        if let Some(Binding::Invocation {
            ty,
            inputs,
            children,
            condition,
            ..
        }) = component
            .bindings
            .iter()
            .find(|binding| matches!(binding, Binding::Invocation { point, .. } if *point == id))
        {
            let condition = super::emit::or(condition.as_ref(), quote! { true });
            let child = construct_child(ty, inputs, *children, components, true);
            self.body.push(quote_spanned! {ty.span()=> if #condition {
                __fusor_writer.child_into(|__fusor_writer| #child)?;
            } });
        }
    }
}

/// Prepare an island before its host element: its ID, props and activation policy.
fn island_prelude(tag: &StartTag<usize>, bindings: &[&Binding], island: &Binding) -> TokenStream {
    let Binding::Island {
        descriptor,
        inputs,
        activation,
        prefetch,
        ..
    } = island
    else {
        unreachable!("island_prelude receives an island binding")
    };
    let span = descriptor.span();
    let activation = super::emit::variant(span, activation);
    let prefetch = super::emit::variant(span, prefetch);
    let id = if let Some(Binding::Attribute { value, .. }) = bindings
        .iter()
        .find(|binding| matches!(binding, Binding::Attribute { name, .. } if name == "id"))
    {
        let value = super::emit::string(value);
        quote! { ::std::option::Option::Some(#value) }
    } else if let Some(value) = tag.attributes.get(b"id".as_slice()) {
        let value = String::from_utf8_lossy(value).into_owned();
        quote! { ::std::option::Option::Some(::std::string::String::from(#value)) }
    } else {
        quote! { ::std::option::Option::<::std::string::String>::None }
    };
    let props = island_props(descriptor, inputs);
    quote_spanned! {span=>
        let __fusor_island_id = #id;
        let __fusor_island = __fusor_context.prepare_island::<#descriptor>(__fusor_island_id.as_deref(), &{ #props }, ::fusor_islands::Activation::#activation, ::fusor_islands::Prefetch::#prefetch)?;
    }
}

/// The island's serialized props, built from the component tag's inputs.
fn island_props(descriptor: &Rust, inputs: &[Input]) -> TokenStream {
    let fields = super::emit::fields(inputs, |value| match value {
        // String literals own their value across the serialized boundary.
        InputValue::Literal(value) => {
            quote_spanned! {value.span()=> ::core::convert::Into::into(#value) }
        }
        value => super::emit::braced(value),
    });
    quote_spanned! {descriptor.span()=> {
        type __FusorIslandProps = <#descriptor as ::fusor_islands::Island>::Props;
        __FusorIslandProps { #(#fields),* }
    }}
}

/// Class bindings extend the static class attribute.
fn classes(tag: &StartTag<usize>, bindings: &[&Binding]) -> TokenStream {
    let initial = tag
        .attributes
        .get(b"class".as_slice())
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default();
    let classes = bindings.iter().filter_map(|binding| match binding {
        Binding::Class { name, value, .. } => Some(
            quote_spanned! {value.span()=> if #value { __fusor_classes.push(' '); __fusor_classes.push_str(#name); } },
        ),
        _ => None,
    });
    quote! { { let mut __fusor_classes = ::std::string::String::from(#initial); #(#classes)* __fusor_writer.attr("class", __fusor_classes.trim()); } }
}

pub(super) fn component(component: &Component, components: &[Component]) -> TokenStream {
    let ty = &component.ty;
    let span = ty.span();
    let hash = hash(component, components);
    let body = component_body(component, components, true);
    quote_spanned! {span=>
        #[cfg(not(target_arch = "wasm32"))]
        impl ::fusor_server::Render for #ty {
            const TEMPLATE_HASH: &'static str = #hash;
            fn render(&self, __fusor_context: &mut ::fusor_server::Context<'_>) -> ::fusor_server::Result<::fusor_server::Html> {
                self.render_with_children(__fusor_context, ::std::option::Option::None)
            }
            fn render_with_children(&self, __fusor_context: &mut ::fusor_server::Context<'_>, __fusor_children: ::std::option::Option<&::fusor_server::Children<'_>>) -> ::fusor_server::Result<::fusor_server::Html> {
                let mut writer = ::fusor_server::Writer::new();
                ::fusor_server::Render::render_into(self, __fusor_context, __fusor_children, &mut writer)?;
                ::std::result::Result::Ok(writer.finish())
            }
            fn render_into(&self, __fusor_context: &mut ::fusor_server::Context<'_>, __fusor_children: ::std::option::Option<&::fusor_server::Children<'_>>, #[allow(unused_mut, reason = "only island rendering reborrows the writer")] mut __fusor_writer: &mut ::fusor_server::Writer) -> ::fusor_server::Result<()> {
                #[allow(unused_variables, reason = "static templates do not read their state")]
                let state = self;
                #body
            }
        }
    }
}

fn static_attribute(output: &mut String, name: &str, value: &str) -> bool {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    template::escape_into(output, value, true);
    output.push('"');
    name == "contenteditable" && value != "false"
}

struct ServerElement<'a> {
    name: &'a str,
    sensitive: bool,
    island: bool,
}

fn sensitive_input(tag: &StartTag<usize>) -> bool {
    tag.name.as_ref() == b"input"
        && tag.attributes.get(b"type".as_slice()).is_some_and(|value| {
            matches!(
                String::from_utf8_lossy(value).to_ascii_lowercase().as_str(),
                "password" | "file"
            )
        })
}

fn server_list(row: &Component, components: &[Component], items: &Rust, key: &Rust) -> TokenStream {
    let span = items.span();
    let row_constructor = if row.item_only_row {
        quote! { ::fusor_components::ForEach::server_item_row }
    } else {
        quote! { ::fusor_components::ForEach::server_row }
    };
    let row = component_body(row, components, true);
    quote_spanned! {span=> {
        let __fusor_items = ::fusor_components::ForEach::entries({ #items });
        let mut __fusor_keys = ::std::collections::BTreeSet::new();
        for __fusor_entry in __fusor_items {
            let __fusor_key = ::fusor_components::ForEach::key(&__fusor_entry, #key);
            #[allow(clippy::clone_on_copy, reason = "list keys have an inferred type and may not be Copy")]
            let __fusor_unique = __fusor_keys.insert(__fusor_key.clone());
            if !__fusor_unique { return ::std::result::Result::Err("duplicate key in ForEach".into()); }
            __fusor_writer.keyed_child(&__fusor_key, |mut __fusor_writer| {
                #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = #row_constructor(state, __fusor_entry);
                #[allow(unused_variables, reason = "template scope bindings may be unused")] let state = &state;
                #row
            })?;
        }
    }}
}

#[cfg(test)]
mod tests {
    use super::static_attribute;

    #[test]
    fn static_attributes_escape_values_and_preserve_preview_editability() {
        let mut output = String::new();
        assert!(!static_attribute(&mut output, "title", "\"<&>'日本語😀"));
        assert!(!static_attribute(&mut output, "contenteditable", "false"));
        assert_eq!(
            output,
            " title=\"&quot;&lt;&amp;&gt;&#39;日本語😀\" contenteditable=\"false\""
        );
        for value in ["", "true", "plaintext-only", "FALSE"] {
            assert!(static_attribute(
                &mut String::new(),
                "contenteditable",
                value
            ));
        }
    }
}
