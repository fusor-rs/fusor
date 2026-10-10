//! Every framework package must come from the same source as `fusor-core`. A
//! graph taking `fusor-core` from a checkout and `fusor-router` from the registry compiles,
//! then behaves as though two frameworks were loaded.
use crate::{
    error::{Error, Result},
    workspace::Project,
};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

pub(crate) struct Source {
    checkout: Option<PathBuf>,
    registry: Option<String>,
}

impl Source {
    pub fn resolve(
        project: &Project,
        document: &DocumentMut,
        workspace: &DocumentMut,
    ) -> Result<Self> {
        let (runtime, root) =
            declared(project, document, workspace, "fusor-core")?.ok_or_else(|| {
                Error::project("the application has no direct fusor-core dependency")
                    .remedy("add `fusor-core` to [dependencies] before adding capabilities")
            })?;
        if runtime.get("git").is_some() {
            return Err(
                Error::project("this application takes fusor-core from a git source").remedy(
                    "git-sourced capability additions are not supported; use a registry release or a checkout path",
                ),
            );
        }
        Ok(Self {
            checkout: path_of(runtime, root)?,
            registry: registry_of(runtime),
        })
    }

    pub fn crate_path(&self, name: &str) -> Result<PathBuf> {
        let checkout = self
            .checkout
            .as_ref()
            .ok_or_else(|| Error::internal("no framework checkout to resolve against"))?;
        sibling(checkout, name)
    }

    pub fn registry(&self) -> Option<&str> {
        self.registry.as_deref()
    }

    pub fn is_checkout(&self) -> bool {
        self.checkout.is_some()
    }

    /// Adding features to an existing package from another source would
    /// silently keep two framework versions in the graph.
    pub fn check_existing(
        &self,
        project: &Project,
        document: &DocumentMut,
        workspace: &DocumentMut,
        name: &str,
    ) -> Result {
        let Some((existing, root)) = declared(project, document, workspace, name)? else {
            return Ok(());
        };
        if existing.get("git").is_some() {
            return Err(Error::project(format!(
                "{name} uses a git source but fusor-core does not"
            ))
            .remedy("align the framework dependency family before adding this capability"));
        }
        let expected = self
            .checkout
            .as_ref()
            .map(|checkout| -> Result<PathBuf> { Ok(sibling(checkout, name)?.canonicalize()?) })
            .transpose()?;
        if path_of(existing, root)? != expected
            || registry_of(existing).as_deref() != self.registry()
        {
            return Err(Error::project(format!(
                "{name} comes from a different source than fusor-core"
            ))
            .remedy(
                "no files were changed; align the framework family deliberately before retrying",
            ));
        }
        Ok(())
    }
}

/// Follows workspace inheritance. Returns the directory `path` is relative to.
fn declared<'a>(
    project: &'a Project,
    document: &'a DocumentMut,
    workspace: &'a DocumentMut,
    name: &str,
) -> Result<Option<(&'a Item, &'a Path)>> {
    let Some(member) = document
        .get("dependencies")
        .and_then(|dependencies| dependencies.get(name))
    else {
        return Ok(None);
    };
    if !inherited(document, name) {
        return Ok(Some((member, project.root.as_path())));
    }
    let shared = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(|dependencies| dependencies.get(name))
        .ok_or_else(|| {
            Error::project(format!(
                "{name} is inherited but absent from workspace.dependencies"
            ))
        })?;
    Ok(Some((shared, project.workspace.as_path())))
}

pub(crate) fn inherited(document: &DocumentMut, name: &str) -> bool {
    document
        .get("dependencies")
        .and_then(|dependencies| dependencies.get(name))
        .and_then(|dependency| dependency.get("workspace"))
        .and_then(Item::as_bool)
        == Some(true)
}

fn path_of(dependency: &Item, root: &Path) -> Result<Option<PathBuf>> {
    dependency
        .get("path")
        .and_then(Item::as_str)
        .map(|path| -> Result<PathBuf> { Ok(root.join(path).canonicalize()?) })
        .transpose()
}

fn registry_of(dependency: &Item) -> Option<String> {
    dependency
        .get("registry")
        .and_then(Item::as_str)
        .map(str::to_owned)
}

fn sibling(checkout: &Path, name: &str) -> Result<PathBuf> {
    Ok(checkout
        .parent()
        .ok_or_else(|| Error::internal("the framework checkout path has no parent"))?
        .join(name))
}
