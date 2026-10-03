//! HTML files whose Rust lives in an ordinary module of the application.
use super::{Result, Source, error::SourceError, includes::BINDINGS_PREFIX};
use crate::{ExternalRust, location};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Links each external Rust module to at most one HTML file.
pub(super) struct Linker<'a> {
    package_root: &'a Path,
    /// The package's output directory, which may not hold authored sources.
    package_output: PathBuf,
    out: &'a Path,
    /// Canonical paths of the modules linked so far.
    linked: BTreeSet<PathBuf>,
}

pub(super) struct LinkedSource {
    /// Relative to the HTML file, as authored; the generated assertion compares it.
    pub(super) path: PathBuf,
    /// Appended to the HTML file's generated Rust.
    pub(super) marker_code: String,
    /// A generated file asserting that `path` holds this HTML file's bindings.
    pub(super) registration_path: PathBuf,
    pub(super) registration_code: String,
    /// The `mod` item that compiles `registration_path`.
    pub(super) registration_item: TokenStream,
    pub(super) line: usize,
    pub(super) column: usize,
}

impl<'a> Linker<'a> {
    /// `output` is the configured output directory, relative to the package.
    pub(super) fn new(package_root: &'a Path, output: &Path, out: &'a Path) -> Self {
        Self {
            package_root,
            package_output: package_root.join(output),
            out,
            linked: BTreeSet::new(),
        }
    }

    /// Check an external Rust source and prepare the assertion that its module
    /// contains this HTML file's `fusor::bindings!`.
    pub(super) fn link(
        &mut self,
        html: &Source,
        source: &str,
        offset: usize,
        rust: &ExternalRust,
    ) -> Result<LinkedSource> {
        let html_path = &html.canonical;
        let name = &html.name;
        let path = html_path.parent().expect("HTML parent").join(&rust.src);
        let canonical = path.canonicalize().map_err(|error| {
            SourceError::at_offset(
                html_path,
                source,
                offset,
                format!("external Rust source {}: {error}", path.display()),
            )
        })?;
        if !canonical.starts_with(self.package_root)
            || canonical.starts_with(&self.package_output)
            || canonical
                .extension()
                .is_none_or(|extension| extension != "rs")
        {
            return Err(SourceError::new(html_path, "external Rust source must be a .rs file inside this package, outside its output directory").into());
        }
        if !self.linked.insert(canonical) {
            return Err(SourceError::new(
                &path,
                "Rust source registered more than once; share logic through ordinary Rust modules",
            )
            .into());
        }
        let module: syn::Path = syn::parse_str(&rust.module)
            .map_err(|error| SourceError::at_offset(html_path, source, offset, error))?;
        // `name` is a module identifier: discovered `@` sources never have Rust.
        let marker = format_ident!("__FUSOR_BINDINGS_{}", name.to_uppercase());
        let expected = path.to_str().ok_or("external source path must be UTF-8")?;
        let registration_path = self
            .out
            .join(format!("{BINDINGS_PREFIX}{name}_registration.rs"));
        let registration_file = registration_path
            .to_str()
            .ok_or("registration path must be UTF-8")?;
        let registration_mod = format_ident!("__fusor_registration_{name}");
        let (line, column) = location(source, offset);
        Ok(LinkedSource {
            marker_code: quote! {
                #[doc(hidden)]
                pub(crate) const #marker: (&str, &str) = __FUSOR_BINDINGS_ORIGIN;
            }
            .to_string(),
            registration_code: quote! {
                const _: () = assert!(
                    ::fusor::authoring::source_matches(#module::#marker, #expected),
                    "external Rust source mismatch: src must identify the module containing fusor::bindings!(name)"
                );
            }
            .to_string(),
            registration_item: quote! { #[path = #registration_file] mod #registration_mod; },
            registration_path,
            path,
            line,
            column,
        })
    }
}
