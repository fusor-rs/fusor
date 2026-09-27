//! Find HTML interpolation boundaries without defining a Rust expression grammar.
use super::{
    ir::{InterpolatedString, StringPart},
    tokens::Rust,
};
use crate::{ExtractError, error};
use html5gum::{Token, Tokenizer};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use std::ops::Range;

pub(super) struct Interpolation {
    pub range: Range<usize>,
    pub tokens: TokenStream,
}

fn decode(value: &str) -> String {
    let mut decoded = String::new();
    for token in Tokenizer::new(value) {
        if let Ok(Token::String(text)) = token {
            decoded.push_str(&String::from_utf8_lossy(&text));
        }
    }
    decoded
}

pub(super) fn interpolations(
    source: &str,
    value: &str,
    offset: usize,
    html_text: bool,
) -> Result<Vec<Interpolation>, ExtractError> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(start) = value[cursor..].find("{{").map(|n| cursor + n) {
        let mut search = start + 2;
        let mut found = None;
        while let Some(end) = value[search..].find("}}").map(|n| search + n) {
            let raw = &value[start + 2..end];
            let expression = if html_text {
                decode(raw)
            } else {
                raw.to_owned()
            };
            // A complete native Rust group must survive tokenization. An HTML
            // delimiter inside a line comment consumes the closing brace and
            // fails this check. No sentinel identifier or custom Rust lexer.
            let probe = format!("{{{expression}}}");
            if let Ok(tokens) = probe.parse::<TokenStream>() {
                let mut tokens = tokens.into_iter();
                if let (Some(TokenTree::Group(group)), None) = (tokens.next(), tokens.next()) {
                    debug_assert_eq!(group.delimiter(), Delimiter::Brace);
                    if group.stream().is_empty() {
                        return Err(error(source, offset + start, "empty Rust interpolation"));
                    }
                    found = Some(Interpolation {
                        range: start..end + 2,
                        tokens: group.stream(),
                    });
                    break;
                }
            }
            // Advance one byte, so nested `}}}` endings are not skipped.
            search = end + 1;
        }
        let interpolation = found.ok_or_else(|| error(
            source, offset + start,
            "unclosed {{ Rust expression }} or unbalanced Rust tokens; escape < and & using HTML entities in text",
        ))?;
        cursor = interpolation.range.end;
        result.push(interpolation);
    }
    Ok(result)
}

// Attribute values are already HTML-decoded; keep the feature's diagnostic while
// consuming the shared tokenizer's expression boundaries. `offset` is where the
// value starts, so the expression is located at its `{{`.
pub(super) fn exact_expression(
    source: &str,
    value: &str,
    mut parts: Vec<Interpolation>,
    offset: usize,
    message: &str,
) -> Result<Rust, ExtractError> {
    if parts.len() != 1
        || !value[..parts[0].range.start].trim().is_empty()
        || !value[parts[0].range.end..].trim().is_empty()
    {
        return Err(error(source, offset, message));
    }
    let part = parts.remove(0);
    Rust::new(source, part.tokens, offset + part.range.start)
}

/// An attribute value split into its literal text and `{{ }}` expressions.
pub(super) fn string(value: &str, parts: Vec<Interpolation>, offset: usize) -> InterpolatedString {
    let mut result = Vec::new();
    let mut cursor = 0;
    for part in parts {
        result.push(StringPart::Literal(
            value[cursor..part.range.start].to_owned(),
        ));
        result.push(StringPart::Expression(Rust::authored(
            part.tokens,
            offset + part.range.start,
        )));
        cursor = part.range.end;
    }
    result.push(StringPart::Literal(value[cursor..].to_owned()));
    InterpolatedString(result)
}

/// Parse an attribute value's interpolations.
pub(super) fn attribute(
    source: &str,
    value: &str,
    offset: usize,
) -> Result<InterpolatedString, ExtractError> {
    let parts = interpolations(source, value, offset, false)?;
    Ok(string(value, parts, offset))
}
