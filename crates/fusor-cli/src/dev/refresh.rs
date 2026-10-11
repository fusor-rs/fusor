//! An edit that leaves the Rust the browser runs unchanged is patched in place,
//! keeping page state. When unclear, the answer is no and a normal build runs.
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    pipeline::{BuildMode, Publication, assets, html, manifest::OutputManifest, tailwind},
    workspace::Project,
};
use fusor_build::app::{self, ArtifactManifest};
use proc_macro2::{TokenStream, TokenTree};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) const CLIENT: &str = include_str!("refresh.js");

/// Checked once after a full build, not on the fast path. The compiler owns its
/// generated includes; other data must be watched and outside refreshable input.
pub(crate) fn enabled(
    cx: &Context,
    project: &Project,
    artifact: &ArtifactManifest,
) -> Result<bool> {
    if !project.config.dev_refresh || !artifact.javascript.is_empty() {
        return Ok(false);
    }
    let watched = super::sources::source_snapshot(cx, project)?;
    let inputs = RefreshInputs::new(project, &watched)?;
    for path in watched
        .keys()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
    {
        let source = fs::read_to_string(path)?;
        // A source that does not tokenize cannot be cleared.
        if app::includes_foreign_file_with(&source, |data| inputs.unchanged_data(path, data))
            != Some(false)
        {
            return Ok(false);
        }
    }
    // Generated include resolution can use an authored source's location. Do
    // not infer it from the output path; only compiler-owned includes are clear.
    for source in &artifact.sources {
        if app::includes_foreign_file(&fs::read_to_string(&source.rust)?) != Some(false) {
            return Ok(false);
        }
    }
    Ok(true)
}

struct RefreshInputs {
    watched: BTreeSet<PathBuf>,
    /// HTML, assets and the Tailwind stylesheet: a refresh republishes these
    /// without Cargo, so Rust that embeds one would keep stale data.
    republished: BTreeSet<PathBuf>,
}

impl RefreshInputs {
    fn new(project: &Project, watched: &super::sources::Snapshot) -> Result<Self> {
        let mut republished: BTreeSet<PathBuf> = project
            .config
            .discover_sources(&project.root)?
            .into_iter()
            .map(|source| source.canonical)
            .collect();
        for path in assets::sources(project)?
            .iter()
            .chain(&tailwind_stylesheet(project))
        {
            republished.insert(fs::canonicalize(path)?);
        }
        Ok(Self {
            watched: watched
                .keys()
                .filter_map(|path| path.canonicalize().ok())
                .collect(),
            republished,
        })
    }

    fn unchanged_data(&self, source: &Path, literal: &str) -> bool {
        let Some(path) = resolve_data(source, literal) else {
            return false;
        };
        self.watched.contains(&path) && !self.republished.contains(&path)
    }
}

fn tailwind_stylesheet(project: &Project) -> Option<PathBuf> {
    let stylesheet = project.config.tailwind.as_ref()?;
    Some(project.root.join(stylesheet))
}

/// Snapshot collection does not follow symlinks. Reject them in include paths
/// too, so an unwatched link edit cannot redirect a previously cleared include.
fn resolve_data(source: &Path, literal: &str) -> Option<PathBuf> {
    let mut path = source.parent()?.canonicalize().ok()?;
    for component in Path::new(literal).components() {
        match component {
            Component::Normal(name) => {
                path.push(name);
                if fs::symlink_metadata(&path).ok()?.file_type().is_symlink() {
                    return None;
                }
            }
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => return None,
        }
    }
    fs::symlink_metadata(&path).ok()?.is_file().then_some(path)
}

/// Publishing a copy of a Rust module or Cargo manifest does not stop it from
/// being compiled.
fn compiled(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
        || path
            .file_name()
            .is_some_and(|name| name == "Cargo.toml" || name == "Cargo.lock")
}

/// `false` for any edit that needs a real build.
pub(crate) fn try_refresh(
    cx: &Context,
    project: &Project,
    changed: &[PathBuf],
    watched: &mut super::sources::Snapshot,
) -> Result<bool> {
    let started = std::time::Instant::now();
    if project.config.delivery.is_some() {
        return Ok(false);
    }
    let html: Vec<_> = project
        .config
        .discover_sources(&project.root)?
        .into_iter()
        .map(|source| project.root.join(source.path))
        .collect();
    let assets = assets::sources(project)?;
    let stylesheet = tailwind_stylesheet(project);
    let republishable = changed.iter().all(|path| {
        html.contains(path)
            || assets.contains(path) && !compiled(path)
            || stylesheet.as_ref() == Some(path)
    });
    if !republishable {
        cx.reporter.note(
            "a changed file is not authored HTML, an asset or the Tailwind stylesheet; rebuilding",
        );
        return Ok(false);
    }

    let output = OutputManifest::read(&project.output(cx))?;
    let Some(previous) = output.rust_signature.as_ref() else {
        return Ok(false);
    };
    // Equal token trees and locations mean the compiled Wasm still matches.
    let work = project.target.join("fusor/refresh").join(&project.name);
    let artifact = app::generate(&project.manifest, &work)?;
    let Ok(next) = signature(&artifact) else {
        return Ok(false);
    };
    if &next != previous {
        cx.reporter
            .note("the Rust generated from this HTML changed; rebuilding");
        return Ok(false);
    }

    let effect = publish_revision(
        cx,
        project,
        &artifact,
        output,
        Changes {
            html: &html,
            paths: changed,
            watched,
        },
    )?;
    cx.reporter.done(format!(
        "Refreshed {} in {}, {effect}",
        project.name,
        crate::reporter::elapsed_text(started.elapsed())
    ));
    Ok(true)
}

struct Changes<'a> {
    html: &'a [PathBuf],
    paths: &'a [PathBuf],
    watched: &'a mut super::sources::Snapshot,
}

fn publish_revision(
    cx: &Context,
    project: &Project,
    artifact: &ArtifactManifest,
    mut output: OutputManifest,
    changes: Changes,
) -> Result<&'static str> {
    let site = project.output(cx);
    let publication = Publication::revise(cx, project, output.generation.clone(), changes.watched)?;
    let revision = output
        .revision()
        .checked_add(1)
        .ok_or_else(|| Error::internal("the refresh revision counter overflowed"))?;
    // Only CSS and authored HTML are patched live. Anything else needs a
    // reload, and the barrier catches clients that missed an update.
    if changes
        .paths
        .iter()
        .any(|path| !changes.html.contains(path) && path.extension().is_none_or(|ext| ext != "css"))
    {
        output.reload_after = Some(revision);
    }
    output.revision = Some(revision);

    if non_css_assets(&site)? != non_css_assets(publication.staging())? {
        output.reload_after = Some(revision);
    }
    let package = publication.generated().join(layout::PACKAGE);
    let stylesheet = tailwind::compile(project, artifact, &package, BuildMode::Development)?;
    fs::write(
        publication.staging().join("index.html"),
        html::render(
            artifact,
            &publication.url_prefix(),
            Some(revision),
            stylesheet.as_slice(),
            &[],
        )?,
    )?;
    publication.commit(&output)?;
    Ok(if output.reload_after == Some(revision) {
        "page reloads"
    } else {
        "page kept its state"
    })
}

/// Locations matter as much as tokens: `line!()` changes behavior without
/// changing a token.
pub(crate) fn signature(artifact: &ArtifactManifest) -> Result<Value> {
    let mut sources = BTreeMap::new();
    for source in &artifact.sources {
        let rust = fs::read_to_string(&source.fingerprint)?;
        let stream: TokenStream = rust
            .parse()
            .map_err(|error| Error::internal(format!("invalid generated Rust: {error}")))?;
        let mut signature = Vec::new();
        tokens(stream, &mut signature);
        // Registration is executable Rust too: an HTML-only change to `src` or
        // `rust:module` must never reuse the previous Wasm.
        signature.push(json!(source.external));
        if let Some(registration) = &source.registration {
            let rust = fs::read_to_string(&registration.rust)?;
            let stream = rust.parse().map_err(|error| {
                Error::internal(format!("invalid generated registration: {error}"))
            })?;
            tokens(stream, &mut signature);
        }
        sources.insert(source.name.clone(), signature);
    }
    Ok(serde_json::to_value(sources)?)
}

fn tokens(stream: TokenStream, output: &mut Vec<Value>) {
    for token in stream {
        let span = token.span();
        let (start, end) = (span.start(), span.end());
        match token {
            TokenTree::Group(group) => {
                output.push(json!([
                    format!("{:?}", group.delimiter()),
                    start.line,
                    start.column,
                    end.line,
                    end.column
                ]));
                tokens(group.stream(), output);
                output.push(json!(["end"]));
            }
            token => output.push(json!([
                token.to_string(),
                start.line,
                start.column,
                end.line,
                end.column
            ])),
        }
    }
}

/// A change here means the page must reload.
fn non_css_assets(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn collect(root: &Path, directory: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root)?;
            if relative.starts_with(layout::GENERATED)
                || relative == Path::new("index.html")
                || relative == Path::new(layout::OUTPUT_MANIFEST)
            {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(Error::project(
                    "published output contains a symlink; refresh cannot compare it",
                ));
            }
            if kind.is_dir() {
                collect(root, &path, out)?;
            } else if path.extension().is_none_or(|extension| extension != "css") {
                out.insert(relative.to_owned(), fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut assets = BTreeMap::new();
    collect(root, root, &mut assets)?;
    Ok(assets)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RefreshFixture {
        project: Project,
        dependency: PathBuf,
        artifact: ArtifactManifest,
        _cleanup: crate::transaction::Staging,
    }

    impl RefreshFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "fusor-refresh-{}-{}",
                crate::pipeline::publish::generation().unwrap(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            let app = root.join("app");
            let dependency = root.join("dependency");
            for path in [
                "app/web",
                "app/public",
                "dependency/src",
                "dependency/runtime",
            ] {
                fs::create_dir_all(root.join(path)).unwrap();
            }
            fs::write(app.join("web/index.html"), "<main>hello</main>").unwrap();
            fs::write(app.join("public/data.bin"), "asset").unwrap();
            fs::write(
                dependency.join("runtime/registry.js"),
                "export const x = 1;",
            )
            .unwrap();
            let project = Project {
                id: "test".into(),
                name: "test".into(),
                manifest: app.join("Cargo.toml"),
                workspace: root.clone(),
                target: root.join("target"),
                watch_roots: vec![app.clone(), dependency.clone()],
                config: app::AppConfig {
                    assets: toml::from_str(r#""/" = "public""#).unwrap(),
                    ..Default::default()
                },
                root: app,
            };
            let artifact = ArtifactManifest {
                version: app::ARTIFACT_VERSION,
                html: root.join("target/app.html"),
                module: root.join("target/module.rs"),
                loader_offset: 0,
                managed_entry: false,
                sources: vec![],
                javascript: vec![],
                classes: BTreeSet::new(),
            };
            Self {
                project,
                dependency,
                artifact,
                _cleanup: crate::transaction::Staging(root),
            }
        }

        fn enabled(&self, source: &str) -> bool {
            fs::write(self.dependency.join("src/lib.rs"), source).unwrap();
            enabled(&Context::default(), &self.project, &self.artifact).unwrap()
        }
    }

    #[test]
    fn watched_dependency_data_outside_refreshable_inputs_allows_static_refresh() {
        let fixture = RefreshFixture::new();
        for source in [
            r#"const JS: &str = include_str!("../runtime/registry.js");"#,
            r#"const JS: &[u8] = include_bytes!("../runtime/registry.js");"#,
        ] {
            assert!(fixture.enabled(source), "{source}");
        }
        assert!(
            !try_refresh(
                &Context::default(),
                &fixture.project,
                &[fixture.dependency.join("runtime/registry.js")],
                &mut super::super::sources::Snapshot::new(),
            )
            .unwrap()
        );
    }

    #[test]
    fn a_rust_source_published_as_an_asset_still_rebuilds() {
        let mut fixture = RefreshFixture::new();
        let module = fixture.project.root.join("src/lib.rs");
        fs::create_dir(module.parent().unwrap()).unwrap();
        fs::write(&module, "pub struct App;").unwrap();
        fixture.project.config.assets = toml::from_str(r#""/source/" = "src/*.rs""#).unwrap();
        assert!(
            !try_refresh(
                &Context::default(),
                &fixture.project,
                &[module],
                &mut super::super::sources::Snapshot::new(),
            )
            .unwrap()
        );
    }

    #[test]
    #[cfg(unix)]
    fn asset_hook_outputs_are_consumed_without_losing_concurrent_edits() {
        let cx = Context::default();
        for exit_code in [0, 1] {
            let mut fixture = RefreshFixture::new();
            let root = &fixture.project.root;
            let source = root.join("web/index.html");
            let asset = root.join("public/data.bin");
            fs::write(root.join("public/stale.css"), "stale").unwrap();
            fixture.project.config.assets_build = vec![
                "sh".into(),
                "-c".into(),
                r#"
printf generated > public/data.bin
printf added > public/new.css
rm public/stale.css
printf edited > web/index.html
exit "$1"
"#
                .into(),
                "asset-hook".into(),
                exit_code.to_string(),
            ];
            let mut watched = super::super::sources::snapshot(&cx, &fixture.project).unwrap();
            let original_source = watched[&source];
            let result = Publication::begin(&cx, &fixture.project, Some(&mut watched));
            match result {
                Ok(_) => assert_eq!(exit_code, 0),
                Err(error) => {
                    assert_eq!(exit_code, 1);
                    assert!(error.to_string().contains("exit status: 1"), "{error}");
                }
            }
            let mut current = super::super::sources::snapshot(&cx, &fixture.project).unwrap();
            assert_ne!(current[&source], original_source);
            current.insert(source, original_source);
            assert_eq!(watched, current);
            fs::write(&asset, "manual edit after the hook").unwrap();
            let edited = super::super::sources::snapshot(&cx, &fixture.project).unwrap();
            assert_ne!(watched[&asset], edited[&asset]);
        }
    }

    #[test]
    fn embedded_html_assets_and_uncertain_includes_still_disable_refresh() {
        let fixture = RefreshFixture::new();
        for source in [
            r#"const HTML: &str = include_str!("../../app/web/index.html");"#,
            r#"const DATA: &[u8] = include_bytes!("../../app/public/data.bin");"#,
            r#"include!("../runtime/registry.js");"#,
            r#"const JS: &str = include_str!(concat!("../runtime/", "registry.js"));"#,
            r#"const JS: &str = include_str!("../runtime/missing.js");"#,
            r#"macro_rules! data { () => { include_str!("../runtime/registry.js") } }"#,
            r#"opaque!(include_str!("../runtime/registry.js"));"#,
            "fn broken( {",
        ] {
            assert!(!fixture.enabled(source), "{source}");
        }
    }

    #[test]
    fn previous_javascript_inputs_do_not_clear_a_rust_data_include() {
        let fixture = RefreshFixture::new();
        let unwatched = fixture.project.workspace.join("unwatched");
        fs::create_dir(&unwatched).unwrap();
        let data = unwatched.join("data.txt");
        fs::write(&data, "previous JavaScript input").unwrap();
        let output = fixture.project.output(&Context::default());
        fs::create_dir_all(&output).unwrap();
        let mut manifest = OutputManifest::new(
            crate::pipeline::publish::generation().unwrap(),
            &fixture.project.config,
        );
        manifest.javascript = Some(crate::pipeline::manifest::Javascript {
            inputs: vec![data.to_string_lossy().into_owned()],
            styles: vec![],
        });
        manifest.write(&output).unwrap();
        assert!(
            !fixture.enabled(r#"const DATA: &str = include_str!("../../unwatched/data.txt");"#)
        );
    }

    #[test]
    fn data_outside_the_watcher_cannot_enable_refresh() {
        let fixture = RefreshFixture::new();
        let unwatched = fixture.project.workspace.join("unwatched");
        fs::create_dir(&unwatched).unwrap();
        fs::write(unwatched.join("data.txt"), "not watched").unwrap();
        assert!(
            !fixture.enabled(r#"const DATA: &str = include_str!("../../unwatched/data.txt");"#)
        );
    }

    #[test]
    fn generated_source_include_locations_remain_conservative() {
        let mut fixture = RefreshFixture::new();
        fs::create_dir_all(&fixture.project.target).unwrap();
        let rust = fixture.project.target.join("generated.rs");
        fs::write(
            &rust,
            r#"const JS: &str = include_str!("../dependency/runtime/registry.js");"#,
        )
        .unwrap();
        fixture.artifact.sources.push(app::SourceArtifact {
            name: "app".into(),
            source: fixture.project.root.join("web/index.html"),
            rust: rust.clone(),
            fingerprint: rust.clone(),
            map: rust.with_extension("map"),
            external: None,
            registration: None,
        });
        assert!(!fixture.enabled("fn source() {}"));
    }

    #[cfg(unix)]
    #[test]
    fn unwatched_symlink_changes_cannot_redirect_a_cleared_include() {
        let fixture = RefreshFixture::new();
        let data = fixture.dependency.join("runtime/registry.js");
        std::os::unix::fs::symlink(&data, fixture.dependency.join("runtime/link.js")).unwrap();
        std::os::unix::fs::symlink(
            fixture.dependency.join("runtime"),
            fixture.dependency.join("linked"),
        )
        .unwrap();
        for source in [
            r#"const JS: &str = include_str!("../runtime/link.js");"#,
            r#"const JS: &str = include_str!("../linked/registry.js");"#,
        ] {
            assert!(!fixture.enabled(source), "{source}");
        }
    }

    fn fingerprint(source: &str) -> Vec<Value> {
        let mut out = Vec::new();
        tokens(source.parse().unwrap(), &mut out);
        out
    }

    #[test]
    fn the_signature_preserves_rust_token_and_location_semantics() {
        assert_eq!(fingerprint("fn f() {}      "), fingerprint("fn f() {} "));
        assert_ne!(
            fingerprint("fn f() { line!(); }"),
            fingerprint("\nfn f() { line!(); }")
        );
        assert_ne!(
            fingerprint("fn f() { 'a: loop { break 'a } }"),
            fingerprint("fn f() { 'b: loop { break 'b } }")
        );
        assert_ne!(
            fingerprint("fn f() { r#\"a b\"#; }"),
            fingerprint("fn f() { r#\"ab\"#; }")
        );
    }

    #[test]
    fn bind_changes_participate_in_native_refresh_compatibility() {
        let html = r#"<script type="text/rust">struct Editor;</script>
<main rust:component="Editor"><input bind="state.title" placeholder="First"></main>"#;
        let source = |html: &str| fusor_build::extract(html).unwrap().fingerprint;
        let before = fingerprint(&source(html));
        assert_eq!(
            before,
            fingerprint(&source(&html.replace("First", "Other")))
        );
        assert_ne!(
            before,
            fingerprint(&source(&html.replace("state.title", "state.other")))
        );
    }
}
