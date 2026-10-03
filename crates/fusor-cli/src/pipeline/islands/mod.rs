//! A native renderer plus independently compiled Wasm units. The build fails
//! unless both halves report the same registrations.
use super::{
    BuildMode, Publication,
    cargo::{Mode, compile},
    manifest::{Delivery, OutputManifest},
    wasm,
};
use crate::{
    context::Context,
    error::{Error, Result},
    layout, toolchain,
    workspace::Project,
};
use fusor_islands::{DeliveryManifest, Unit, UnitWitness};
use std::{collections::BTreeMap, env, fs, path::Path, process::Command};
use witness::{reject_javascript, validate_pairs, witness};

mod witness;

pub(crate) fn check(cx: &Context, project: &Project) -> Result {
    reject_javascript(&compile(cx, project, Mode::Check)?.manifest)?;
    for unit in units(cx, project)? {
        reject_javascript(&compile(&unit.cx, &unit.project, Mode::Check)?.manifest)?;
    }
    Ok(())
}

pub(crate) fn build(cx: &Context, project: &Project, mode: BuildMode) -> Result {
    let dev = mode == BuildMode::Development;
    let bindgen = toolchain::bindgen::resolve()?;
    let (executable, native_registrations) = build_renderer(cx, project, mode)?;

    let publication = Publication::begin(cx, project)?;
    // A page holds props and markup from the generation it was served; pairing
    // them with new code is worse than a missing asset.
    publication.retain_previous()?;
    let prefix = publication.url_prefix();
    let mut delivery = DeliveryManifest {
        version: fusor_islands::PROTOCOL_VERSION,
        generation: publication.generation.clone(),
        units: BTreeMap::new(),
    };
    for unit in units(cx, project)? {
        let built = build_unit(&unit, &bindgen, &publication, mode)?;
        delivery.units.insert(unit.name, built);
    }
    delivery.validate()?;
    validate_pairs(&native_registrations, &delivery)?;

    let manifest_path = publication.generated().join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&delivery)?)?;
    write_runtime(
        publication.generated(),
        &prefix,
        &publication.generation,
        project,
        dev,
    )?;
    render_document(&executable, &manifest_path, publication.staging(), &prefix)?;

    let mut output = OutputManifest::new(publication.generation.clone(), &project.config);
    output.delivery = Some(Delivery::Islands);
    output.revision = Some(0);
    output.reload_after = Some(0);
    let count = delivery.units.len();
    publication.commit(&output)?;
    cx.reporter
        .note(format!("{count} independent Wasm delivery units"));
    Ok(())
}

/// Runs before staging, so disagreeing halves fail before producing output.
fn build_renderer(
    cx: &Context,
    project: &Project,
    mode: BuildMode,
) -> Result<(std::path::PathBuf, UnitWitness)> {
    let native = compile(
        cx,
        project,
        Mode::Build {
            release: mode == BuildMode::Release,
        },
    )?;
    reject_javascript(&native.manifest)?;
    let executable = native.executable.ok_or_else(|| {
        Error::project("Cargo produced no native renderer executable for this islands application")
    })?;
    let registrations = witness(
        Command::new(&executable).arg("--fusor-manifest"),
        "the native renderer",
    )?;
    Ok((executable, registrations))
}

/// A unit declares its own Cargo features and must not inherit the site's,
/// which may name a feature the unit does not have.
struct Selected {
    name: String,
    cx: Context,
    project: Project,
}

fn units(cx: &Context, project: &Project) -> Result<Vec<Selected>> {
    let delivery = project
        .config
        .delivery
        .as_ref()
        .expect("units is only called for an islands application");
    delivery
        .units
        .iter()
        .map(|(name, unit)| {
            let mut unit_cx = cx.for_manifest(
                project.workspace.join("Cargo.toml"),
                Some(unit.package.clone()),
            );
            unit_cx.features = unit.features.clone();
            let discovered = Project::discover(&unit_cx)?;
            if discovered.config.delivery.is_some() {
                return Err(Error::project(format!(
                    "delivery unit {} is itself an islands site",
                    unit.package
                ))
                .remedy("a unit must be a Wasm application package"));
            }
            Ok(Selected {
                name: name.clone(),
                cx: unit_cx,
                project: discovered,
            })
        })
        .collect()
}

fn build_unit(
    unit: &Selected,
    bindgen: &Path,
    publication: &Publication<'_>,
    mode: BuildMode,
) -> Result<Unit> {
    let name = &unit.name;
    let prefix = publication.url_prefix();
    let generated = publication.generated();
    let compilation = compile(
        &unit.cx,
        &unit.project,
        Mode::Build {
            release: mode == BuildMode::Release,
        },
    )?;
    reject_javascript(&compilation.manifest)?;
    let wasm_path = compilation
        .wasm
        .ok_or_else(|| Error::project(format!("delivery unit {name} emitted no Wasm artifact")))?;
    let destination = generated.join(name);
    fs::create_dir(&destination)?;
    wasm::bindgen(bindgen, &wasm_path, &destination, "unit", mode).map_err(|error| {
        Error::tooling(format!("delivery unit {name}: {error}")).remedy(
            "units reserve wasm-bindgen's start slot; remove the unit's application start function",
        )
    })?;
    wasm::optimize(&destination.join("unit_bg.wasm"), mode)?;
    fs::write(destination.join("package.json"), "{\"type\":\"module\"}\n")?;

    let node = env::var_os("FUSOR_NODE").unwrap_or_else(|| "node".into());
    let registrations = witness(
        Command::new(node)
            .args(["--input-type=module", "-e", include_str!("witness.mjs")])
            .arg(destination.join("unit_bg.wasm")),
        &format!("delivery unit {name}"),
    )?;
    let mut dependencies = Vec::new();
    collect_dependencies(
        &destination,
        &destination,
        &format!("{prefix}/{name}"),
        &mut dependencies,
    )?;
    Ok(Unit {
        javascript: format!("{prefix}/{name}/unit.js"),
        wasm: format!("{prefix}/{name}/unit_bg.wasm"),
        dependencies,
        entries: registrations.entries,
    })
}

fn write_runtime(
    generated: &Path,
    prefix: &str,
    generation: &str,
    project: &Project,
    dev: bool,
) -> Result {
    fs::write(
        generated.join("registry.js"),
        fusor_islands::REGISTRY_JAVASCRIPT,
    )?;
    fs::write(
        generated.join("composition.js"),
        fusor_islands::COMPOSITION_JAVASCRIPT,
    )?;
    let mut boot = format!(
        "import {{ install }} from './registry.js';\nconst response = await fetch({:?});\nif (!response.ok) throw new Error('Immutable island manifest is unavailable');\ninstall(await response.json());\n",
        format!("{prefix}/manifest.json")
    );
    if dev {
        fs::write(
            generated.join(layout::REFRESH_MODULE),
            crate::dev::refresh::CLIENT,
        )?;
        boot = format!(
            "import {{ watch }} from './{}';\nwatch({generation:?}, {:?});\n{boot}",
            layout::REFRESH_MODULE,
            project.config.base_path
        );
    }
    fs::write(generated.join(layout::BOOT_MODULE), boot)?;
    Ok(())
}

/// The native renderer writes the document; the CLI adds the runtime's two
/// script tags.
fn render_document(executable: &Path, manifest: &Path, staging: &Path, prefix: &str) -> Result {
    let path = staging.join("index.html");
    crate::process::checked(
        Command::new(executable)
            .arg("--fusor-render")
            .arg(manifest)
            .arg(&path),
    )?;
    let mut html = fs::read_to_string(&path)?;
    let scripts = format!(
        "<script src=\"{prefix}/composition.js\"></script><script type=\"module\" src=\"{prefix}/{}\"></script>",
        layout::BOOT_MODULE
    );
    html.insert_str(head_start(&html)?, &scripts);
    fs::write(path, html)?;
    Ok(())
}

/// For the runtime to preload.
fn collect_dependencies(
    root: &Path,
    directory: &Path,
    prefix: &str,
    output: &mut Vec<String>,
) -> Result {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_dependencies(root, &path, prefix, output)?;
        } else if path.extension().is_some_and(|extension| extension == "js")
            && path.file_name().is_none_or(|name| name != "unit.js")
        {
            output.push(format!(
                "{prefix}/{}",
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/")
            ));
        }
    }
    output.sort();
    Ok(())
}

fn head_start(html: &str) -> Result<usize> {
    let mut emitter = html5gum::DefaultEmitter::<usize>::new_with_span();
    emitter.naively_switch_states(true);
    html5gum::Tokenizer::new_with_emitter(html, emitter)
        .find_map(|token| match token.ok()? {
            html5gum::Token::StartTag(tag) if &*tag.name == b"head" => Some(tag.span.end),
            _ => None,
        })
        .ok_or_else(|| {
            Error::project("the native renderer produced a document without a <head>")
                .remedy("render a complete HTML document from the application's entry")
        })
}
