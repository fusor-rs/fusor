use crate::reporter::elapsed_text;
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    pipeline::{
        self, BuildMode, Publication,
        cargo::{Mode, compile},
        manifest::Javascript,
        wasm,
    },
    toolchain,
    workspace::{self, Project},
};
use pipeline::site::{Mount, SiteConfig, shown};
use std::{fs, time::Instant};

/// `fusor build` for one application.
pub(crate) fn single(cx: &Context, project: &Project, mode: BuildMode) -> Result {
    cx.reporter.banner(if mode == BuildMode::Debug {
        "debug build"
    } else {
        "production build"
    });
    run(cx, project, mode)?;
    cx.reporter
        .done(format!("Published {}", shown(&project.output(cx))));
    Ok(())
}

/// Each application builds into `target/fusor/site/<name>` first; only a
/// complete set is assembled and published.
pub(crate) fn site(cx: &Context, mode: BuildMode) -> Result {
    cx.reporter.banner(if mode == BuildMode::Debug {
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
        run(&app, &project, mode).map_err(|error| error.context(&project.name))?;
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
pub(crate) fn run(cx: &Context, project: &Project, mode: BuildMode) -> Result {
    cx.reporter.step(format!("Compiling {} ...", project.name));
    let started = Instant::now();
    if project.config.delivery.is_some() {
        pipeline::islands::build(cx, project, mode)?;
    } else {
        application(cx, project, mode)?;
    }
    cx.reporter.done(format!(
        "Compiled {} in {}",
        project.name,
        elapsed_text(started.elapsed())
    ));
    Ok(())
}

fn application(cx: &Context, project: &Project, mode: BuildMode) -> Result {
    let dev = mode == BuildMode::Development;
    let bindgen = toolchain::bindgen::resolve()?;
    cx.reporter
        .note(format!("using wasm-bindgen at {}", bindgen.display()));
    let publication = Publication::begin(cx, project)?;
    if dev {
        // A production build has no in-flight page to keep serving.
        publication.retain_previous()?;
    }

    let compilation = compile(
        cx,
        project,
        Mode::Build {
            release: mode == BuildMode::Release,
        },
    )?;
    let wasm_path = compilation
        .wasm
        .as_ref()
        .ok_or_else(|| Error::project("Cargo emitted no WebAssembly library"))?;
    let generated = publication.generated();
    let package = generated.join(layout::PACKAGE);
    wasm::bindgen(&bindgen, wasm_path, &package, layout::APP_NAME, mode)?;
    wasm::optimize(&package.join(layout::APP_WASM), mode)?;

    let workers = pipeline::workers::build(
        &publication,
        &bindgen,
        compilation.manifest.managed_entry,
        mode,
    )?;
    let prefix = publication.url_prefix();
    let bundle: Javascript = serde_json::from_value(fusor_npm::bundle(
        &project.root,
        &package.join(layout::APP_MODULE),
        &serde_json::to_value(&compilation.manifest.javascript)?,
        &format!("{prefix}/{}", layout::PACKAGE),
        mode == BuildMode::Release,
    )?)?;
    write_boot(project, &publication, &compilation.manifest, workers, mode)?;
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

    let output = publication.output_manifest(&compilation.manifest, bundle, mode)?;
    publication.commit(&output)
}

/// A startup failure is dispatched as `fusor:error` so the page can show its
/// own error state.
fn write_boot(
    project: &Project,
    publication: &Publication<'_>,
    artifact: &fusor_build::app::ArtifactManifest,
    workers: Option<pipeline::workers::Artifacts>,
    mode: BuildMode,
) -> Result {
    let mut boot = workers
        .map(|workers| workers.boot())
        .transpose()?
        .unwrap_or_default();
    let reload = if mode == BuildMode::Development {
        fs::write(
            publication.generated().join(layout::REFRESH_MODULE),
            crate::dev::refresh::CLIENT,
        )?;
        format!(
            "import {{ watch }} from './{}';\nwatch({:?}, {:?});\n",
            layout::REFRESH_MODULE,
            publication.generation,
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
    boot.push_str(&format!(
        "{reload}\ntry {{\n  const client = await import('./{}/{}');\n{startup_check}  await client.default();\n}} catch (error) {{\n  console.error('fusor failed to initialize:', error);\n  document.dispatchEvent(new CustomEvent('fusor:error', {{ detail: error }}));\n}}\n",
        layout::PACKAGE,
        layout::APP_MODULE
    ));
    fs::write(publication.generated().join(layout::BOOT_MODULE), boot)?;
    Ok(())
}
