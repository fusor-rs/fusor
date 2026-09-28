use crate::reporter::elapsed_text;
use crate::{
    context::Context,
    error::{Error, Result},
    pipeline::{
        self, Publication,
        cargo::{Mode, compile},
        manifest::{Javascript, OutputManifest},
        wasm,
    },
    toolchain,
    workspace::{self, Project},
};
use pipeline::site::{Mount, SiteConfig, shown};
use std::{fs, time::Instant};

/// `fusor build` for one application.
pub(crate) fn single(cx: &Context, project: &Project, debug: bool) -> Result {
    cx.reporter.banner(if debug {
        "debug build"
    } else {
        "production build"
    });
    run(cx, project, debug, false)?;
    cx.reporter
        .done(format!("Published {}", shown(&project.output(cx))));
    Ok(())
}

/// Each application builds into `target/fusor/site/<name>` first; only a
/// complete set is assembled and published.
pub(crate) fn site(cx: &Context, debug: bool) -> Result {
    cx.reporter.banner(if debug {
        "debug build"
    } else {
        "production build"
    });
    let started = Instant::now();
    let metadata = workspace::read_metadata(cx, cx.manifest_path.as_deref())?;
    let config = SiteConfig::from_metadata(&metadata.metadata)?;
    let manifest = metadata.workspace_root.join("Cargo.toml");
    let mut mounts = Vec::new();
    for (path, name) in &config.mounts {
        let mut app = cx.for_manifest(manifest.clone(), Some(name.clone()));
        let project = Project::discover(&app)?;
        let built = project
            .target
            .join(crate::layout::SITE_BUILD)
            .join(&project.name);
        let mount = Mount::new(path, &project, built.clone())?;
        app.output = Some(built);
        run(&app, &project, debug, false).map_err(|error| error.context(&project.name))?;
        mounts.push(mount);
    }
    let output = metadata.workspace_root.join(&config.output);
    let table = pipeline::site::describe(
        &output,
        config
            .mounts
            .iter()
            .map(|(path, name)| (path.as_str(), name.as_str())),
    );
    pipeline::site::publish(cx, &output, mounts)?;
    cx.reporter.done(format!(
        "Published {} in {}\n{table}",
        shown(&output),
        elapsed_text(started.elapsed())
    ));
    Ok(())
}

/// Build and publish one application, in development or release mode.
pub(crate) fn run(cx: &Context, project: &Project, debug: bool, dev: bool) -> Result {
    cx.reporter.step(format!("Compiling {} ...", project.name));
    let started = Instant::now();
    if project.config.delivery.is_some() {
        pipeline::islands::build(cx, project, debug, dev)?;
    } else {
        application(cx, project, debug, dev)?;
    }
    cx.reporter.done(format!(
        "Compiled {} in {}",
        project.name,
        elapsed_text(started.elapsed())
    ));
    Ok(())
}

fn application(cx: &Context, project: &Project, debug: bool, dev: bool) -> Result {
    let bindgen = toolchain::bindgen::resolve()?;
    cx.reporter
        .note(format!("using wasm-bindgen at {}", bindgen.display()));
    let publication = Publication::begin(cx, project)?;
    if dev {
        // A production build has no in-flight page to keep serving.
        publication.retain_previous()?;
    }

    let compilation = compile(cx, project, Mode::Build { release: !debug })?;
    let wasm_path = compilation
        .wasm
        .as_ref()
        .ok_or_else(|| Error::project("Cargo emitted no WebAssembly library"))?;
    let generated = publication.generated();
    let package = generated.join("pkg");
    wasm::bindgen(&bindgen, wasm_path, &package, "app", debug, dev)?;
    wasm::optimize(&package.join("app_bg.wasm"), debug, dev)?;

    let prefix = publication.url_prefix();
    let bundle: Javascript = serde_json::from_value(fusor_npm::bundle(
        &project.root,
        &package.join("app.js"),
        &serde_json::to_value(&compilation.manifest.javascript)?,
        &format!("{prefix}/pkg"),
        !debug,
    )?)?;
    fs::write(
        generated.join("boot.js"),
        boot_script(
            project,
            &compilation.manifest,
            &publication.generation,
            dev,
            generated,
        )?,
    )?;
    // Development pages keep the loader's own discovery order.
    let modules = if dev {
        Vec::new()
    } else {
        pipeline::html::entry_modules(&package)?
    };
    fs::write(
        publication.staging().join("index.html"),
        pipeline::html::render(
            &compilation.manifest,
            &prefix,
            dev.then_some(0),
            &bundle.styles,
            &modules,
        )?,
    )?;

    let output = output_manifest(cx, project, &publication, &compilation, bundle, dev)?;
    publication.commit(&output)
}

fn output_manifest(
    cx: &Context,
    project: &Project,
    publication: &Publication<'_>,
    compilation: &pipeline::cargo::Compilation,
    bundle: Javascript,
    dev: bool,
) -> Result<OutputManifest> {
    let mut output = OutputManifest::new(publication.generation.clone(), &project.config);
    if !compilation.manifest.javascript.is_empty() {
        output.javascript = Some(bundle);
    }
    if !dev {
        return Ok(output);
    }
    output.revision = Some(0);
    output.reload_after = Some(0);
    // Recorded only when reuse is possible, so the watcher does not
    // re-establish eligibility on every edit.
    if crate::dev::refresh::enabled(cx, project, &compilation.manifest)? {
        output.rust_signature = Some(crate::dev::refresh::signature(&compilation.manifest)?);
    }
    Ok(output)
}

/// A startup failure is dispatched as `fusor:error` so the page can show its
/// own error state.
fn boot_script(
    project: &Project,
    artifact: &fusor_build::app::ArtifactManifest,
    generation: &str,
    dev: bool,
    generated: &std::path::Path,
) -> Result<String> {
    let reload = if dev {
        fs::write(generated.join("refresh.js"), crate::dev::refresh::CLIENT)?;
        format!(
            "import {{ watch }} from './refresh.js';\nwatch({generation:?}, {:?});\n",
            project.config.base_path
        )
    } else {
        String::new()
    };
    // An entry that declares <App> without generated startup would otherwise
    // be a blank page.
    let startup_check = if artifact.managed_entry {
        let message = format!(
            "entry HTML declares App but generated startup is absent; include fusor::template!({:?}) in the entry state's Rust module",
            project.config.entry.to_string_lossy()
        );
        format!(
            "  if (typeof client.__fusor_start !== 'function') throw new Error({});\n",
            serde_json::to_string(&message)?
        )
    } else {
        String::new()
    };
    Ok(format!(
        "{reload}\ntry {{\n  const client = await import('./pkg/app.js');\n{startup_check}  await client.default();\n}} catch (error) {{\n  console.error('fusor failed to initialize:', error);\n  document.dispatchEvent(new CustomEvent('fusor:error', {{ detail: error }}));\n}}\n"
    ))
}
