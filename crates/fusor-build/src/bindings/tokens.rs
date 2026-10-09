//! Native Rust token construction and source-aware serialization.
//!
//! This module never recognizes Rust expressions or invents punctuation. The
//! emitter walks TokenTree groups and respects Spacing::Joint, just as the token
//! stream's Display implementation does. Line breaks preserve HTML origins when
//! the build-script boundary requires tokens to be written as a .rs file.

use crate::{BindingLocation, ExtractError, error};
use proc_macro2::{Delimiter, Group, Ident, LexError, Spacing, Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::BTreeMap;

/// Rust tokens and the HTML offset they came from. `span` identifies the tokens
/// for the source map; it survives rewrites that wrap `tokens` in generated code.
#[derive(Clone)]
pub(super) struct Rust {
    pub tokens: TokenStream,
    pub offset: usize,
    span: Span,
}

impl Rust {
    pub fn parse(source: &str, code: &str, offset: usize) -> Result<Self, ExtractError> {
        let tokens = code.parse().map_err(|problem| {
            quote_error(source, code, offset, &problem)
                .unwrap_or_else(|| error(source, offset, format!("invalid Rust tokens: {problem}")))
        })?;
        Self::new(source, tokens, offset)
    }

    pub fn new(source: &str, tokens: TokenStream, offset: usize) -> Result<Self, ExtractError> {
        if tokens.is_empty() {
            return Err(error(source, offset, "expected a Rust expression or type"));
        }
        Ok(Self::authored(tokens, offset))
    }

    /// Nonempty tokens parsed from the HTML at `offset`.
    pub fn authored(tokens: TokenStream, offset: usize) -> Self {
        let span = tokens
            .clone()
            .into_iter()
            .next()
            .expect("nonempty Rust")
            .span();
        Self {
            tokens,
            offset,
            span,
        }
    }

    /// Tokens the compiler writes for the HTML at `offset`, such as a generated
    /// type name. They get a span identity of their own, so the source map
    /// points them at that HTML rather than at other generated code.
    pub fn synthetic(tokens: TokenStream, offset: usize) -> Self {
        let span = fresh_span();
        Self {
            tokens: respan(tokens, span),
            offset,
            span,
        }
    }

    pub fn ident(name: &str, offset: usize) -> Self {
        Self::synthetic(
            Ident::new(name, Span::call_site()).into_token_stream(),
            offset,
        )
    }

    /// Generated tokens that belong to this fragment's origin.
    pub fn derived(&self, tokens: TokenStream) -> Self {
        Self {
            tokens,
            offset: self.offset,
            span: self.span,
        }
    }

    /// Whether two fragments are the same Rust, such as one local shadowing another.
    pub fn same_tokens(&self, other: &Rust) -> bool {
        self.tokens.to_string() == other.tokens.to_string()
    }

    pub fn span(&self) -> Span {
        self.span
    }
}

pub(super) fn quote_error(
    source: &str,
    code: &str,
    offset: usize,
    problem: &LexError,
) -> Option<ExtractError> {
    let position = problem.span().byte_range().start;
    if !code[position..].starts_with('\'') {
        return None;
    }
    // HTML entity decoding can change offsets. Refine the binding's location
    // only when the Rust prefix still matches the authored source.
    let offset = if source
        .get(offset..)
        .is_some_and(|authored| authored.starts_with(&code[..position]))
    {
        offset + position
    } else {
        offset
    };
    Some(error(
        source,
        offset,
        "Rust strings use double quotes, e.g. \"dark\"; \
         single-quoted character literals must contain one character, e.g. 'd'",
    ))
}

/// Every parsed string is a new source file to proc-macro2, so its span is a
/// new identity. Nothing else can mint one.
fn fresh_span() -> Span {
    let tokens: TokenStream = "_".parse().expect("an identifier");
    tokens.into_iter().next().expect("one token").span()
}

fn respan(tokens: TokenStream, span: Span) -> TokenStream {
    tokens
        .into_iter()
        .map(|mut token| {
            if let TokenTree::Group(group) = &token {
                let mut inner = Group::new(group.delimiter(), respan(group.stream(), span));
                inner.set_span(span);
                token = TokenTree::Group(inner);
            } else {
                token.set_span(span);
            }
            token
        })
        .collect()
}

impl ToTokens for Rust {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(self.tokens.clone());
    }
}

#[derive(Default)]
pub(super) struct Origins(BTreeMap<String, usize>);

impl Origins {
    pub fn register(&mut self, rust: &Rust) {
        // Span::file is the native source-file identity, never parsed or fabricated.
        // Call-site tokens are generated scaffolding and keep the caller's fallback.
        let file = rust.span().file();
        if file != Span::call_site().file() {
            self.0.insert(file, rust.offset);
        }
    }

    fn offset(&self, span: Span, fallback: usize) -> usize {
        self.0.get(&span.file()).copied().unwrap_or(fallback)
    }
}

pub(super) fn emit(
    source: &str,
    output: &mut String,
    tokens: TokenStream,
    origins: &Origins,
    fallback: usize,
) -> Vec<BindingLocation> {
    if !output.ends_with('\n') {
        output.push('\n');
    }
    let mut writer = Writer {
        source,
        line: output.bytes().filter(|byte| *byte == b'\n').count() + 1,
        output,
        origins,
        locations: Vec::new(),
        current_origin: None,
        joint: false,
        indent: 0,
    };
    writer.stream(tokens, fallback);
    writer.newline();
    writer.locations
}

struct Writer<'a> {
    source: &'a str,
    output: &'a mut String,
    origins: &'a Origins,
    line: usize,
    locations: Vec<BindingLocation>,
    current_origin: Option<usize>,
    joint: bool,
    indent: usize,
}

impl Writer<'_> {
    fn newline(&mut self) {
        if !self.output.ends_with('\n') {
            self.output.push('\n');
            self.line += 1;
        }
    }

    fn token(&mut self, text: &str, offset: usize) {
        if !self.joint && self.current_origin != Some(offset) {
            self.newline();
        }
        if self.output.ends_with('\n') {
            self.output.push_str(&"    ".repeat(self.indent));
        } else if !self.joint {
            self.output.push(' ');
        }
        let start = self.line;
        self.output.push_str(text);
        self.line += text.bytes().filter(|byte| *byte == b'\n').count();
        let end = self.line + 1;
        // A joint punctuation sequence is indivisible even across origins.
        let start = self
            .locations
            .last()
            .map_or(start, |last| start.max(last.generated_end));
        if start < end {
            let (line, column) = crate::location(self.source, offset);
            if let Some(last) = self.locations.last_mut().filter(|last| {
                last.generated_end == start && last.line == line && last.column == column
            }) {
                last.generated_end = end;
            } else {
                self.locations.push(BindingLocation {
                    generated_start: start,
                    generated_end: end,
                    line,
                    column,
                });
            }
        }
        self.current_origin = Some(offset);
        self.joint = false;
    }

    fn stream(&mut self, tokens: TokenStream, fallback: usize) {
        for token in tokens {
            let offset = self.origins.offset(token.span(), fallback);
            match token {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => {
                            self.stream(group.stream(), offset);
                            continue;
                        }
                    };
                    self.token(open, offset);
                    let block = group.delimiter() == Delimiter::Brace;
                    if block {
                        self.indent += 1;
                        self.newline();
                    }
                    self.stream(group.stream(), offset);
                    if block {
                        self.indent -= 1;
                        self.newline();
                    }
                    self.token(close, offset);
                    if block {
                        self.newline();
                    }
                }
                TokenTree::Punct(punctuation) => {
                    self.token(&punctuation.to_string(), offset);
                    self.joint = punctuation.spacing() == Spacing::Joint;
                    if punctuation.as_char() == ';' && !self.joint {
                        self.newline();
                    }
                }
                token => self.token(&token.to_string(), offset),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_preserves_native_tokens_and_joint_punctuation() {
        for source in [
            r#"fn r#match<'a>(x: &'a str) -> &'a str { x }"#,
            r#"{ let π = 1..=3; π.map(|n| n << 2).sum::<i32>() >= -1 }"#,
            r###"format!(r#"{{ }} {}"#, "quotes \" backslash \\ newline\n")"###,
            "'outer: loop { break 'outer; }",
            r#"('d', '🦀', '\n', '\'', b'd', "'dark'")"#,
            "macro_rules! test { ($($t:tt)*) => { $($t)* }; }",
            "{ /* ignored */ 1 // line comment\n+ 2 }",
        ] {
            let rust = Rust::parse(source, source, 0).unwrap();
            let mut origins = Origins::default();
            origins.register(&rust);
            let mut output = String::new();
            let locations = emit(source, &mut output, rust.tokens.clone(), &origins, 0);
            assert_eq!(
                output.parse::<TokenStream>().unwrap().to_string(),
                rust.tokens.to_string(),
                "{source}"
            );
            assert!(crate::SourceMap::new(locations).is_ok());
        }
    }
}
