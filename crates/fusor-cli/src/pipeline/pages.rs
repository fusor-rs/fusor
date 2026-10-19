//! Static delivery: the native renderer writes every page of the site, and
//! nothing runs in the browser. The CLI links the Tailwind stylesheet, and in
//! development the live-reload module, into each document it publishes.
use super::{
    BuildMode, Publication,
    cargo::{Mode, compile},
    html::{Page, head_start},
    islands::reject_javascript,
    manifest::{Delivery, OutputManifest},
    tailwind,
};
use crate::{
    context::Context,
    dev::sources::Snapshot,
    error::{Error, Result},
    layout,
    process::checked,
    workspace::Project,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn build(
    cx: &Context,
    project: &Project,
    mode: BuildMode,
    watched: Option<&mut Snapshot>,
) -> Result {
    let native = compile(
        cx,
        project,
        Mode::Build {
            release: mode == BuildMode::Release,
        },
    )?;
    reject_javascript(&native.manifest)?;
    let executable = native.executable.as_deref().ok_or_else(|| {
        Error::project("Cargo produced no native renderer executable for this static application")
    })?;
    // Rendered before staging, so a failing renderer leaves the output as it was.
    let rendered = render(project, executable)?;

    let publication = Publication::begin(cx, project, watched)?;
    // A page loaded a moment ago still links the previous generation's stylesheet.
    publication.retain_previous()?;
    let prefix = publication.url_prefix();
    let page = Page {
        artifact: &native.manifest,
        libraries: &native.libraries,
    };
    let mut head = String::new();
    if let Some(stylesheet) = tailwind::compile(project, page, publication.generated(), mode)? {
        head.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"{prefix}/{stylesheet}\">"
        ));
    }
    if mode == BuildMode::Development {
        fs::write(
            publication.generated().join(layout::BOOT_MODULE),
            publication.reload_module()?,
        )?;
        head.push_str(&format!(
            "<script type=\"module\" src=\"{prefix}/{}\"></script>",
            layout::BOOT_MODULE
        ));
    }
    let documents = publish(&rendered, publication.staging(), &head)?;

    let mut output = OutputManifest::new(publication.generation.clone(), &project.config);
    output.delivery = Some(Delivery::Static);
    output.revision = Some(0);
    output.reload_after = Some(0);
    publication.commit(&output)?;
    cx.reporter.note(format!("{documents} static pages"));
    Ok(())
}

/// Runs the renderer in the package directory, so it can read its content by
/// relative path, and returns the fresh directory it wrote.
fn render(project: &Project, executable: &Path) -> Result<PathBuf> {
    let directory = project.target.join(layout::PAGES_BUILD).join(&project.name);
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    checked(
        Command::new(executable)
            .arg("--fusor-render-pages")
            .arg(&directory)
            .current_dir(&project.root),
    )?;
    if !directory.join("index.html").is_file() {
        return Err(
            Error::project("the native renderer wrote no index.html").remedy(
                "write the site's first page to index.html in the directory after --fusor-render-pages",
            ),
        );
    }
    Ok(directory)
}

/// Moves every rendered file into the publication, completing each document's
/// `<head>`, and returns the number of documents.
fn publish(rendered: &Path, staging: &Path, head: &str) -> Result<usize> {
    let mut documents = 0;
    for file in files(rendered)? {
        let relative = file.strip_prefix(rendered)?;
        // Messages name a file by its published path, which uses / on every platform.
        let published = relative.to_string_lossy().replace('\\', "/");
        let target = staging.join(relative);
        if relative.starts_with(layout::GENERATED) {
            return Err(Error::project(format!(
                "the native renderer wrote {published}, under the directory fusor reserves for generated files"
            ))
            .remedy(format!("write pages outside {}/", layout::GENERATED)));
        }
        if target.exists() {
            return Err(Error::project(format!(
                "the native renderer wrote {published}, which a published asset also provides"
            ))
            .remedy("render the page at another path, or remove the asset from [package.metadata.fusor.assets]"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if file
            .extension()
            .is_some_and(|extension| extension == "html")
        {
            let mut document = fs::read_to_string(&file)?;
            let start = head_start(&document).map_err(|error| error.context(&published))?;
            document.insert_str(start, head);
            fs::write(target, document)?;
            documents += 1;
        } else {
            fs::copy(&file, target)?;
        }
    }
    Ok(documents)
}

fn files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            found.extend(files(&path)?);
        } else {
            found.push(path);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> (PathBuf, crate::transaction::Staging) {
        let path = std::env::temp_dir().join(format!(
            "fusor-pages-{name}-{}",
            crate::pipeline::publish::generation().unwrap()
        ));
        fs::create_dir_all(&path).unwrap();
        (path.clone(), crate::transaction::Staging(path))
    }

    #[test]
    fn every_document_gains_the_head_and_other_files_are_copied_unchanged() {
        let (rendered, _rendered) = directory("rendered");
        let (staging, _staging) = directory("staging");
        fs::create_dir(rendered.join("first-post")).unwrap();
        fs::write(
            rendered.join("index.html"),
            "<!doctype html><html><head><title>Posts</title></head></html>",
        )
        .unwrap();
        fs::write(
            rendered.join("first-post/index.html"),
            "<html><HEAD lang=en><title>First</title></HEAD></html>",
        )
        .unwrap();
        fs::write(rendered.join("feed.xml"), "<feed><head></head></feed>").unwrap();
        assert_eq!(
            publish(&rendered, &staging, "<link rel=stylesheet>").unwrap(),
            2
        );
        assert_eq!(
            fs::read_to_string(staging.join("index.html")).unwrap(),
            "<!doctype html><html><head><link rel=stylesheet><title>Posts</title></head></html>"
        );
        assert_eq!(
            fs::read_to_string(staging.join("first-post/index.html")).unwrap(),
            "<html><HEAD lang=en><link rel=stylesheet><title>First</title></HEAD></html>"
        );
        assert_eq!(
            fs::read_to_string(staging.join("feed.xml")).unwrap(),
            "<feed><head></head></feed>"
        );
    }

    #[test]
    fn rendered_files_cannot_replace_assets_reserved_paths_or_lack_a_head() {
        for (path, contents, staged, message) in [
            (
                "brand.svg",
                "<svg/>",
                Some("brand.svg"),
                "brand.svg, which a published asset",
            ),
            (
                "__fusor/app.js",
                "",
                None,
                "__fusor/app.js, under the directory fusor reserves",
            ),
            (
                "about/index.html",
                "<p>No head</p>",
                None,
                "about/index.html: the native renderer produced a document without a <head>",
            ),
        ] {
            let (rendered, _rendered) = directory("rendered");
            let (staging, _staging) = directory("staging");
            let file = rendered.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, contents).unwrap();
            if let Some(asset) = staged {
                fs::write(staging.join(asset), "asset").unwrap();
            }
            let error = publish(&rendered, &staging, "").unwrap_err().to_string();
            assert!(error.contains(message), "{path}: {error}");
        }
    }
}
