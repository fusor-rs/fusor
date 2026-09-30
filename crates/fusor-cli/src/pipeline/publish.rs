//! A failed build leaves the last successful site serving. The switch is two
//! renames, and a failure between them puts the previous output back.
use crate::{
    context::Context,
    error::{Error, Result},
    layout,
    workspace::Project,
};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

/// Held across the rename and by the dev server while it reads a file.
/// Compilation runs outside it, so the last site stays responsive.
pub(crate) static OUTPUT_ACCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Unique across builds, and URL- and filename-safe.
pub(crate) fn generation() -> String {
    format!(
        "g-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos(),
        std::process::id()
    )
}

pub(crate) fn copy_assets(project: &Project, staging: &Path) -> Result {
    let Some(assets) = &project.config.assets else {
        return Ok(());
    };
    let assets = project.root.join(assets);
    if fs::symlink_metadata(&assets).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(Error::project(format!(
            "the assets directory is a symlink: {}",
            assets.display()
        ))
        .remedy("point assets at a real directory"));
    }
    if !assets.is_dir() {
        return Err(Error::project(format!(
            "the assets directory does not exist: {}",
            assets.display()
        ))
        .remedy("create it, or change assets in [package.metadata.fusor]"));
    }
    for name in [
        "index.html",
        layout::GENERATED,
        layout::OUTPUT_MANIFEST,
        layout::WORKER_HEADERS,
    ] {
        if assets.join(name).exists() {
            return Err(Error::project(format!(
                "the asset named {name:?} would be overwritten by generated output"
            ))
            .remedy("rename it; fusor owns that name in the published site"));
        }
    }
    copy_tree(&assets, staging)
}

/// A published symlink would escape the site root on some hosts.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Result {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(Error::project(format!(
                "{} is a symlink or special file; published sites contain only files and directories",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

pub(crate) fn publish(cx: &Context, project: &Project, staging: &Path) -> Result {
    let _access = OUTPUT_ACCESS
        .lock()
        .map_err(|_| Error::internal("the output publication lock was poisoned"))?;
    project.validate_output(cx)?;
    swap(cx, &project.output(cx), staging)
}

/// The caller has already checked that `site` is ours to replace.
pub(crate) fn swap(cx: &Context, site: &Path, staging: &Path) -> Result {
    let previous = staging.with_extension("previous");
    if site.exists() {
        fs::rename(site, &previous)?;
    }
    if let Err(error) = fs::rename(staging, site) {
        if previous.exists() {
            fs::rename(&previous, site)?;
        }
        return Err(error.into());
    }
    if previous.exists() {
        if let Err(error) = fs::remove_dir_all(&previous) {
            // The new site is live; failing now would wrongly suggest the
            // previous output was kept.
            cx.reporter.warn(format!(
                "could not remove the previous output {}: {error}",
                previous.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{context::Context, workspace::Project};

    fn project(root: std::path::PathBuf) -> Project {
        Project {
            id: "test".into(),
            name: "test".into(),
            manifest: root.join("Cargo.toml"),
            workspace: root.clone(),
            target: root.join("target"),
            watch_roots: vec![root.clone()],
            config: toml::from_str("").unwrap(),
            root,
        }
    }

    #[test]
    fn a_failed_swap_restores_the_previous_output_and_allows_a_retry() {
        let cx = Context::default();
        let root = std::env::temp_dir().join(format!("fusor-publish-{}", generation()));
        fs::create_dir(&root).unwrap();
        let _cleanup = crate::transaction::Staging(root.clone());
        let project = project(root);
        let site = project.output(&cx);
        fs::create_dir(&site).unwrap();
        fs::write(site.join(layout::OUTPUT_MANIFEST), "{}").unwrap();
        fs::write(site.join("index.html"), "previous").unwrap();

        // The old output moves aside successfully, then the missing stage fails.
        let staging = project.root.join("next");
        publish(&cx, &project, &staging).unwrap_err();
        assert_eq!(
            fs::read_to_string(site.join("index.html")).unwrap(),
            "previous"
        );
        assert!(site.join(layout::OUTPUT_MANIFEST).is_file());
        assert!(!staging.with_extension("previous").exists());

        fs::create_dir(&staging).unwrap();
        fs::write(staging.join(layout::OUTPUT_MANIFEST), "{}").unwrap();
        fs::write(staging.join("index.html"), "next").unwrap();
        publish(&cx, &project, &staging).unwrap();
        assert_eq!(fs::read_to_string(site.join("index.html")).unwrap(), "next");
        assert!(!staging.exists());
        assert!(!staging.with_extension("previous").exists());
    }

    #[test]
    fn a_missing_assets_directory_is_named_with_its_remedy() {
        let root = std::env::temp_dir().join(format!("fusor-assets-{}", generation()));
        let mut project = project(root.clone());
        project.config.assets = Some("public".into());
        let error = copy_assets(&project, &root.join("stage")).unwrap_err();
        assert!(error.to_string().contains("does not exist"), "{error}");
    }
}
