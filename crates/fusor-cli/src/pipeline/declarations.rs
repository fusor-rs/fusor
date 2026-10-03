//! TypeScript declarations come from the Rust artifact, so even `fusor check`
//! writes them without Node.
use crate::{
    error::{Error, Result},
    layout,
    transaction::read_optional,
    workspace::Project,
};
use fusor_build::app::ArtifactManifest;
use std::{fs, path::Path};

pub(crate) fn write(project: &Project, artifact: &ArtifactManifest) -> Result {
    let directory = project.root.join(layout::TYPES);
    let index = directory.join(layout::TYPES_INDEX);
    let previous = read_index(&index)?;
    if artifact.javascript.is_empty() && previous.is_none() {
        return Ok(());
    }
    fs::create_dir_all(&directory)?;

    let mut names = Vec::new();
    for module in &artifact.javascript {
        let name = &module.declaration_name;
        if !is_declaration_name(name) || names.contains(name) {
            return Err(Error::internal(format!(
                "the compiler emitted an invalid or duplicate declaration name {name:?}"
            )));
        }
        write_if_changed(&directory.join(name), &fs::read(&module.declaration)?)?;
        names.push(name.clone());
    }
    for name in previous.unwrap_or_default() {
        if !names.contains(&name) {
            let path = directory.join(name);
            match fs::remove_file(&path) {
                Ok(()) => {}
                // A prior failed cleanup may already have removed this declaration.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::from(error).context(path.display())),
            }
        }
    }
    write_if_changed(&index, &serde_json::to_vec_pretty(&names)?)
}

/// A missing index starts ownership; an unreadable or invalid one must be repaired.
fn read_index(path: &Path) -> Result<Option<Vec<String>>> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    let invalid = |message| {
        Error::project(message).context(path.display()).remedy(
            "Restore the declaration ownership index to a JSON array of generated .d.ts file names",
        )
    };
    let names: Vec<String> =
        serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
    if let Some(name) = names.iter().find(|name| !is_declaration_name(name)) {
        return Err(invalid(format!("invalid declaration name {name:?}")));
    }
    Ok(Some(names))
}

/// These come from the compiler, but they become filesystem paths.
fn is_declaration_name(name: &str) -> bool {
    let path = Path::new(name);
    name.ends_with(".d.ts")
        && path.components().count() == 1
        && matches!(
            path.components().next(),
            Some(std::path::Component::Normal(_))
        )
}

/// Editors watch this directory and reload on any write.
fn write_if_changed(path: &Path, contents: &[u8]) -> Result {
    if read_optional(path)?.as_deref() != Some(contents) {
        fs::write(path, contents).map_err(|error| Error::from(error).context(path.display()))?;
    }
    Ok(())
}
