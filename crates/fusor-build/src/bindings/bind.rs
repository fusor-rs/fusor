//! `bind` on a form control: parsing, the browser install, hydration adoption
//! and server HTML. `fusor::bind` defines what each bound value means.
use super::{emit, interpolation, ir::*, tokens::Rust};
use crate::{ExtractError, error};
use fusor::template::{ElementId, InputKind};
use html5gum::{StartTag, Token};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use std::collections::BTreeMap;

/// Parse `bind` on a native element, given its attributes and where each value starts.
pub(super) fn parse(
    source: &str,
    element: &str,
    attributes: &BTreeMap<String, (String, usize)>,
    node: ElementId,
    value: &str,
    offset: usize,
) -> Result<Binding, ExtractError> {
    let fail = |message: &str| Err(error(source, offset, message));
    if value.trim().is_empty() {
        return fail("bind requires a Rust value, such as bind=\"state.name\"");
    }
    let attribute = |name: &str| attributes.get(name);
    if attribute("rust:slot").is_some() {
        return fail("bind owns the control's contents; remove rust:slot");
    }
    let choice = || {
        attribute("value")
            .map(|(value, offset)| interpolation::attribute(source, value, *offset))
            .transpose()
    };
    let control = match element {
        "textarea" => Control::Text,
        "select" => match attribute("multiple") {
            None => Control::Select,
            Some((flag, _)) if !flag.contains("{{") => Control::SelectMultiple,
            Some(_) => return fail("bind requires a static multiple attribute on select"),
        },
        "input" => {
            let kind = attribute("type").map_or("text", |(kind, _)| kind);
            if kind.contains("{{") {
                return fail("bind requires a static input type");
            }
            match InputKind::of(kind) {
                InputKind::Text => Control::Text,
                InputKind::Checkbox => Control::Checkbox(
                    choice()?.unwrap_or_else(|| InterpolatedString::literal("on")),
                ),
                InputKind::Radio => match choice()? {
                    Some(choice) => Control::Radio(choice),
                    None => return fail("bind on a radio button requires the value it chooses"),
                },
                InputKind::File => {
                    return fail(
                        "bind cannot set a file input; read its files in an on:change handler",
                    );
                }
                InputKind::Uneditable => return fail("bind requires a control the user can edit"),
            }
        }
        _ => return fail("bind requires an <input>, <textarea> or <select>"),
    };
    // The control owns its value; a checkbox or radio keeps `value` as its choice.
    let owned: &[&str] = match control {
        Control::Checkbox(_) | Control::Radio(_) => &["checked"],
        _ => &["value", "checked"],
    };
    if owned.iter().any(|name| attribute(name).is_some()) {
        return fail("bind owns this control's value; remove the value or checked attribute");
    }
    Ok(Binding::Bind {
        node,
        control,
        value: Rust::parse(source, value, offset)?,
    })
}

/// The function in `fusor::dom::controls` that installs this control;
/// `adopt_` plus the same name adopts its native edits.
fn runtime(control: &Control) -> &'static str {
    match control {
        Control::Text => "text",
        Control::Select => "select",
        Control::SelectMultiple => "select_multiple",
        Control::Checkbox(_) => "checkbox",
        Control::Radio(_) => "radio",
    }
}

/// Install a bound control. rustc checks the bound value's type at the attribute.
pub(super) fn browser(node: ElementId, control: &Control, value: &Rust) -> TokenStream {
    let node = emit::element(node);
    let function = format_ident!("{}", runtime(control));
    let choice = control.choice().map(|choice| {
        let choice = emit::string(choice);
        quote! { move || #choice, }
    });
    // Clone the bound value before the choice closure takes `state`.
    quote_spanned! {value.span()=> {
        let __fusor_bound = ::std::clone::Clone::clone(&(#value));
        ::fusor::dom::controls::#function(&mut __fusor_scope, &#node, #choice __fusor_bound)?;
    }}
}

/// Take a native edit made before hydration into the bound value.
pub(super) fn adopt(node: ElementId, control: &Control, value: &Rust) -> TokenStream {
    let node = emit::element(node);
    let function = format_ident!("adopt_{}", runtime(control));
    let choice = control.choice().map(|choice| {
        let choice = emit::string(choice);
        quote! { || #choice, }
    });
    quote_spanned! {value.span()=>
        ::fusor::dom::controls::#function(&__fusor_scope, &#node, #choice &(#value))?;
    }
}

/// How the runtime decides whether the bound value chooses a checkbox, radio or option.
fn chooses(control: &Control) -> TokenStream {
    match control {
        Control::Checkbox(_) => quote! { ::fusor::bind::Checkbox::checked },
        Control::SelectMultiple => quote! { ::fusor::bind::selected },
        _ => quote! { ::fusor::bind::TextValue::shows },
    }
}

/// Server HTML for a bound control's own element: its value attribute or text,
/// or whether it is checked.
pub(super) fn server(
    element: &str,
    sensitive: bool,
    control: &Control,
    value: &Rust,
    content: &mut Vec<TokenStream>,
) -> TokenStream {
    let span = value.span();
    let text = quote_spanned! {span=> ::fusor::bind::TextValue::text(&(#value)) };
    match control {
        Control::Text if element == "textarea" => {
            content.push(quote_spanned! {span=> __fusor_writer.text(#text); });
            quote! {}
        }
        Control::Text if !sensitive => {
            quote_spanned! {span=> __fusor_writer.attr("value", #text); }
        }
        // Password text is never rendered; a select's options are written selected.
        Control::Text | Control::Select | Control::SelectMultiple => quote! {},
        Control::Checkbox(choice) | Control::Radio(choice) => {
            let (chooses, choice) = (chooses(control), emit::string(choice));
            quote_spanned! {span=> __fusor_writer.boolean("checked", #chooses(&(#value), &#choice)); }
        }
    }
}

/// A bound select whose options the server is writing.
pub(super) struct Select<'a> {
    control: &'a Control,
    value: &'a Rust,
}

impl<'a> Select<'a> {
    /// The bound select among an element's bindings.
    pub fn open(bindings: &[&'a Binding]) -> Option<Self> {
        bindings.iter().find_map(|binding| match binding {
            Binding::Bind {
                control: control @ (Control::Select | Control::SelectMultiple),
                value,
                ..
            } => Some(Self { control, value }),
            _ => None,
        })
    }

    /// Mark an option selected when the bound value chooses it. The option's
    /// value is its `value` attribute or, without one, the text that follows.
    pub fn option(
        &self,
        tag: &StartTag<usize>,
        bindings: &[&Binding],
        following: &[Token<usize>],
    ) -> TokenStream {
        let choice = bindings
            .iter()
            .find_map(|binding| match binding {
                Binding::Attribute { name, value, .. } if name == "value" => Some(value),
                _ => None,
            })
            .map_or_else(
                || {
                    let text = tag.attributes.get(b"value".as_slice()).map_or_else(
                        || option_text(following),
                        |value| String::from_utf8_lossy(value).into_owned(),
                    );
                    emit::string(&InterpolatedString::literal(&text))
                },
                emit::string,
            );
        let (value, chooses) = (self.value, chooses(self.control));
        quote_spanned! {value.span()=> __fusor_writer.boolean("selected", #chooses(&(#value), &#choice)); }
    }
}

/// An option's text, as the browser reads it for the option's value: trimmed,
/// with whitespace collapsed. The parser rejects interpolated text here.
fn option_text(following: &[Token<usize>]) -> String {
    let text: String = following
        .iter()
        .take_while(|token| !matches!(token, Token::StartTag(_) | Token::EndTag(_)))
        .filter_map(|token| match token {
            Token::String(text) => Some(String::from_utf8_lossy(text)),
            _ => None,
        })
        .collect();
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
