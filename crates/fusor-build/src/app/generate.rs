use super::{
    ARTIFACT_VERSION, AppConfig, ArtifactManifest, HTML_FILE, MANIFEST_FILE, MODULE_FILE,
    RegistrationArtifact, Result, Source, SourceArtifact, SourceError, SourceKind,
    external::{LinkedSource, Linker},
    includes::{BINDINGS_PREFIX, TEMPLATE_DIRECTORY},
    validate,
};
use crate::{
    Page, RustBlock, SourceMap,
    extract::extract_from,
    html,
    javascript::{self, PlannedModule},
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Compile registered sources to a Cargo output directory. No files in the
/// authored application are modified. Useful to hosts and `cargo fusor expand`.
pub fn generate(manifest: &Path, out: &Path) -> Result<ArtifactManifest> {
    let config = AppConfig::load(manifest)?;
    let root = manifest
        .parent()
        .ok_or("manifest has no parent directory")?;
    let package_root = root.canonicalize()?;
    fs::create_dir_all(out)?;
    let out = out.canonicalize()?;
    let templates = out.join(TEMPLATE_DIRECTORY);
    if templates.exists() {
        fs::remove_dir_all(&templates)?;
    }
    let mut generator = Generator {
        config: &config,
        package_root: &package_root,
        out: &out,
        next_component: 0,
        embedded_templates: false,
        linker: Linker::new(&package_root, &config.output, &out),
    };
    // Compiling a source writes nothing, so a rejected source leaves no new output.
    let sources = config
        .discover_sources(root)?
        .into_iter()
        .map(|source| generator.compile(source))
        .collect::<Result<Vec<_>>>()?;
    generator.write(sources)
}

struct Generator<'a> {
    config: &'a AppConfig,
    package_root: &'a Path,
    out: &'a Path,
    next_component: usize,
    embedded_templates: bool,
    linker: Linker<'a>,
}

struct CompiledSource {
    source: Source,
    page: Page,
    map: String,
    rust_path: PathBuf,
    /// Inline Rust becomes a module of `crate::ui`.
    ui_module: Option<TokenStream>,
    external: Option<LinkedSource>,
    javascript: Vec<PlannedModule>,
}

struct Entry {
    html: String,
    loader_offset: usize,
    managed: bool,
}

impl Generator<'_> {
    fn extract(&mut self, source: &Source, text: &str) -> Result<Page> {
        let html_path = &source.canonical;
        if source.kind == SourceKind::Entry {
            self.embedded_templates = validate::component_file(text).is_ok();
        }
        if source.kind != SourceKind::Entry {
            validate::component_file(text)
                .map_err(|error| SourceError::extracted(html_path, error))?;
        }
        let page = extract_from(text, self.next_component, self.embedded_templates)
            .map_err(|error| SourceError::extracted(html_path, error))?;
        let native = page.blocks.is_empty();
        if native && page.component_count == 0 {
            return Err(SourceError::new(
                html_path,
                "native template files require an App boundary or rust:component declarations",
            )
            .into());
        }
        if !native && source.kind == SourceKind::Discovered {
            return Err(SourceError::new(html_path, "discovered HTML uses fusor::template! in an ordinary Rust module; register script-based HTML explicitly in Cargo metadata").into());
        }
        if source.kind != SourceKind::Entry {
            if let Some(offset) = page.app_offset {
                return Err(SourceError::at_offset(
                    html_path,
                    text,
                    offset,
                    "App is only allowed in the entry document",
                )
                .into());
            }
        }
        self.next_component += page.component_count;
        Ok(page)
    }

    fn compile(&mut self, source: Source) -> Result<CompiledSource> {
        let html_path = &source.canonical;
        let text = fs::read_to_string(html_path)?;
        let mut page = self.extract(&source, &text)?;
        let native = page.blocks.is_empty();
        let rust_path = if native {
            let path = source.path.to_str().ok_or("template path must be UTF-8")?;
            self.out.join(TEMPLATE_DIRECTORY).join(format!("{path}.rs"))
        } else {
            self.out
                .join(format!("{BINDINGS_PREFIX}{}.rs", source.name))
        };
        let mut ui_module = None;
        let mut external = None;
        // Extraction rejects an external script next to any other Rust block.
        if let Some(RustBlock {
            element,
            external: Some(rust),
            ..
        }) = page.blocks.first()
        {
            let linked = self.linker.link(&source, &text, element.start, rust)?;
            page.rust.push('\n');
            page.rust.push_str(&linked.marker_code);
            external = Some(linked);
        } else if !native {
            let module = format_ident!("{}", source.name);
            let path = rust_path
                .to_str()
                .ok_or("generated module path must be UTF-8")?;
            ui_module = Some(quote! { #[path = #path] pub mod #module; });
        }
        self.check_javascript_delivery(&page, html_path)?;
        let javascript = javascript::plan(
            self.package_root,
            self.out,
            html_path,
            &page,
            external.as_ref().map(|linked| linked.path.as_path()),
        )?;
        let map = SourceMap::new(page.locations.clone())?.to_string();
        Ok(CompiledSource {
            source,
            page,
            map,
            rust_path,
            ui_module,
            external,
            javascript,
        })
    }

    fn check_javascript_delivery(&self, page: &Page, html: &Path) -> Result<()> {
        if let Some(module) = page
            .javascript
            .first()
            .filter(|_| self.config.delivery.is_some())
        {
            return Err(SourceError::at(
                    html,
                    module.line,
                    module.column,
                    "component JavaScript is unsupported in server/island delivery; use a browser application",
                )
                .into());
        }
        Ok(())
    }

    fn write(self, sources: Vec<CompiledSource>) -> Result<ArtifactManifest> {
        let mut ui_modules = Vec::new();
        let mut registrations = Vec::new();
        let mut artifacts = Vec::new();
        let mut javascript = Vec::new();
        let mut classes = BTreeSet::new();
        let mut entry = None;
        let mut templates = String::new();
        for compiled in sources {
            artifacts.push(write_source(&compiled)?);
            ui_modules.extend(compiled.ui_module);
            registrations.extend(compiled.external.map(|linked| linked.registration_item));
            javascript.extend(
                compiled
                    .javascript
                    .into_iter()
                    .map(|module| module.artifact),
            );
            let page = compiled.page;
            classes.extend(page.classes);
            if compiled.source.kind == SourceKind::Entry {
                // A native entry has no script to load next to, so load before </body>.
                let loader_offset = page
                    .loader_offset
                    .unwrap_or_else(|| html::body_end(&page.html));
                entry = Some(Entry {
                    managed: page.app_offset.is_some(),
                    html: page.html,
                    loader_offset,
                });
            } else {
                templates.push_str(&page.html);
                templates.push('\n');
            }
        }
        let mut entry = entry.expect("the entry is always the first source");
        html::insert_before_body_end(&mut entry.html, &templates, Some(&mut entry.loader_offset));
        let html_path = self.out.join(HTML_FILE);
        let module_path = self.out.join(MODULE_FILE);
        fs::write(&html_path, entry.html)?;
        fs::write(
            &module_path,
            quote! { pub mod ui { #(#ui_modules)* } #(#registrations)* }.to_string(),
        )?;
        let artifact = ArtifactManifest {
            version: ARTIFACT_VERSION,
            html: html_path,
            module: module_path,
            loader_offset: entry.loader_offset,
            managed_entry: entry.managed,
            sources: artifacts,
            javascript,
            classes,
        };
        fs::write(
            self.out.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&artifact)?,
        )?;
        Ok(artifact)
    }
}

fn write_source(compiled: &CompiledSource) -> Result<SourceArtifact> {
    let rust_path = &compiled.rust_path;
    fs::create_dir_all(rust_path.parent().expect("generated parent"))?;
    let fingerprint = rust_path.with_extension("fingerprint.rs");
    let map = rust_path.with_extension("map");
    if let Some(linked) = &compiled.external {
        fs::write(&linked.registration_path, &linked.registration_code)?;
    }
    javascript::write(&compiled.javascript)?;
    fs::write(rust_path, &compiled.page.rust)?;
    fs::write(&fingerprint, &compiled.page.fingerprint)?;
    fs::write(&map, &compiled.map)?;
    let linked = compiled.external.as_ref();
    Ok(SourceArtifact {
        name: compiled.source.name.clone(),
        source: compiled.source.canonical.clone(),
        rust: rust_path.clone(),
        fingerprint,
        map,
        external: linked.map(|linked| linked.path.clone()),
        registration: linked.map(|linked| RegistrationArtifact {
            rust: linked.registration_path.clone(),
            line: linked.line,
            column: linked.column,
        }),
    })
}
