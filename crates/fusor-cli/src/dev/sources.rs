//! Hash polling rather than filesystem events behaves the same on every
//! platform and with editors that write through a temporary file. A file is
//! read and hashed again only when its size or modification time changes.
use crate::{
    context::Context,
    error::Result,
    layout,
    pipeline::{assets, manifest::OutputManifest},
    workspace::Project,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(crate) type Snapshot = BTreeMap<PathBuf, Fingerprint>;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Fingerprint {
    len: u64,
    modified: SystemTime,
    contents: u64,
}

/// Only contents count: a build step that rewrites a watched file with the
/// same bytes must not trigger another build.
impl PartialEq for Fingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.contents == other.contents
    }
}

/// Files whose size and modification time match `previous` keep its hash.
pub(crate) fn snapshot(cx: &Context, project: &Project, previous: &Snapshot) -> Result<Snapshot> {
    let mut files = source_snapshot(cx, project, previous)?;
    // Package imports and authored modules outside the Cargo package roots are
    // exact dependencies, even though scanning all node_modules is unnecessary.
    // Missing files disappear from the snapshot and therefore trigger a rebuild.
    let Some(javascript) =
        OutputManifest::read_optional(&project.output(cx))?.and_then(|output| output.javascript)
    else {
        return Ok(files);
    };
    for input in javascript.inputs {
        let path = PathBuf::from(input);
        match fs::metadata(&path) {
            Ok(metadata) => {
                let fingerprint = fingerprint(&path, &metadata, previous)?;
                files.insert(path, fingerprint);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(files)
}

/// These files stay watched across publications. Previous output metadata may
/// add JavaScript inputs that disappear after the next full build.
pub(crate) fn source_snapshot(
    cx: &Context,
    project: &Project,
    previous: &Snapshot,
) -> Result<Snapshot> {
    let generated = [
        project.target.clone(),
        project.output(cx),
        project.root.join(&project.config.output),
    ];
    let mut files = Snapshot::new();
    for root in &project.watch_roots {
        collect(root, &generated, previous, &mut files)?;
    }
    // A dependency or profile change is a source change too.
    for name in ["Cargo.toml", "Cargo.lock", ".cargo"] {
        let path = project.workspace.join(name);
        if path.exists() {
            collect(&path, &generated, previous, &mut files)?;
        }
    }
    files.extend(asset_snapshot(project, previous)?);
    Ok(files)
}

/// Assets are copied after the hook. The earlier snapshot of every other file
/// is kept, so edits made during the hook and afterward still trigger a build.
pub(crate) fn record_assets(
    project: &Project,
    hooked: &BTreeSet<PathBuf>,
    previous: &mut Snapshot,
) -> Result {
    let recorded = asset_snapshot(project, previous)?;
    previous.retain(|path, _| !hooked.contains(path));
    previous.extend(recorded);
    Ok(())
}

/// Asset files can lie outside every watch root, such as `../../install.sh`.
fn asset_snapshot(project: &Project, previous: &Snapshot) -> Result<Snapshot> {
    let mut files = Snapshot::new();
    for path in assets::sources(project)? {
        let fingerprint = fingerprint(&path, &fs::metadata(&path)?, previous)?;
        files.insert(path, fingerprint);
    }
    Ok(files)
}

fn collect(
    path: &Path,
    generated: &[PathBuf],
    previous: &Snapshot,
    out: &mut Snapshot,
) -> io::Result<()> {
    if generated.iter().any(|generated| generated == path) {
        return Ok(());
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };
    if IGNORED.contains(&name) || name.starts_with(".fusor-") {
        return Ok(());
    }
    let kind = fs::symlink_metadata(path)?;
    if kind.is_symlink() {
        return Ok(());
    }
    if kind.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(&entry?.path(), generated, previous, out)?;
        }
    } else if kind.is_file() {
        out.insert(path.to_owned(), fingerprint(path, &kind, previous)?);
    }
    Ok(())
}

/// `metadata` is read before the contents, so a write in between changes the
/// recorded time and the next poll hashes the file again.
fn fingerprint(
    path: &Path,
    metadata: &fs::Metadata,
    previous: &Snapshot,
) -> io::Result<Fingerprint> {
    let len = metadata.len();
    let modified = metadata.modified()?;
    if let Some(known) = previous
        .get(path)
        .filter(|known| known.len == len && known.modified == modified)
    {
        return Ok(*known);
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    fs::read(path)?.hash(&mut hash);
    Ok(Fingerprint {
        len,
        modified,
        contents: hash.finish(),
    })
}

/// Walking these would make every build look like a source change.
const IGNORED: &[&str] = &[
    "target",
    "dist",
    ".git",
    "node_modules",
    ".cache",
    ".codex",
    ".agents",
    layout::STATE,
    "test-results",
];
