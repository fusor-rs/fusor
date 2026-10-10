//! A failed build leaves the last successful site serving. The switch is two
//! renames, and a failure between them puts the previous output back.
use crate::{
    context::Context,
    error::{Error, Result},
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
pub(crate) fn generation() -> Result<String> {
    Ok(format!(
        "g-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(
                |error| Error::project(format!("cannot name a build generation: {error}"))
                    .remedy("set the system clock to a date after the Unix epoch")
            )?
            .as_nanos(),
        std::process::id()
    ))
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
    use crate::{context::Context, layout, workspace::Project};

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
    fn corrupt_output_metadata_fails_retention_and_source_watching() {
        let cx = Context::default();
        let root = std::env::temp_dir().join(format!("fusor-corrupt-{}", generation().unwrap()));
        let _cleanup = crate::transaction::Staging(root.clone());
        let project = project(root);
        let site = project.output(&cx);
        fs::create_dir_all(&site).unwrap();
        fs::write(site.join(layout::OUTPUT_MANIFEST), "{broken").unwrap();
        fs::write(site.join("index.html"), "previous").unwrap();
        let publication = crate::pipeline::Publication::begin(&cx, &project, None).unwrap();
        assert!(
            publication
                .retain_previous()
                .unwrap_err()
                .to_string()
                .contains(".fusor-output.json")
        );
        assert!(
            crate::dev::sources::snapshot(&cx, &project)
                .unwrap_err()
                .to_string()
                .contains(".fusor-output.json")
        );
        assert_eq!(
            fs::read_to_string(site.join("index.html")).unwrap(),
            "previous"
        );
    }

    #[test]
    fn dropping_a_source_watcher_joins_it_and_releases_its_project() {
        let cx = Context::default();
        let root = std::env::temp_dir().join(format!("fusor-watcher-{}", generation().unwrap()));
        fs::create_dir(&root).unwrap();
        let _cleanup = crate::transaction::Staging(root.clone());
        let project = project(root);
        let current = std::sync::Arc::new(std::sync::RwLock::new(project.clone()));
        let released = std::sync::Arc::downgrade(&current);
        let watcher = crate::dev::watch::start(&cx, project, current).unwrap();
        assert!(released.upgrade().is_some());
        drop(watcher);
        assert!(released.upgrade().is_none());
    }

    #[test]
    fn a_failed_swap_restores_the_previous_output_and_allows_a_retry() {
        let cx = Context::default();
        let root = std::env::temp_dir().join(format!("fusor-publish-{}", generation().unwrap()));
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
}
