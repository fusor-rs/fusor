//! Undoing a partial write: [`Staging`] discards work that never became
//! visible, [`OwnedFile`] restores files edited in place.
use crate::error::Result;
use std::{fs, path::PathBuf};

/// Removes the directory on drop, so an early `?` cleans up too. Publishing
/// renames it away first.
pub(crate) struct Staging(pub PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

/// Rollback is refused when the file no longer matches what the operation
/// wrote: someone edited it in the meantime, and losing their work is worse
/// than leaving a half-applied change the error describes.
pub(crate) struct OwnedFile {
    path: PathBuf,
    before: Option<Vec<u8>>,
    written: Option<Vec<u8>>,
}

impl OwnedFile {
    pub fn snapshot(path: PathBuf) -> Result<Self> {
        let before = read_optional(&path)?;
        Ok(Self {
            path,
            written: before.clone(),
            before,
        })
    }

    /// Call after each step that may have written, so rollback knows what was
    /// ours.
    pub fn capture(&mut self) -> Result {
        self.written = read_optional(&self.path)?;
        Ok(())
    }

    pub fn rollback(&self, reporter: &crate::reporter::Reporter) -> Result {
        if self.before == self.written {
            return Ok(());
        }
        if read_optional(&self.path)? != self.written {
            reporter.warn(format!(
                "preserving concurrent changes to {}",
                self.path.display()
            ));
            return Ok(());
        }
        match &self.before {
            Some(bytes) => fs::write(&self.path, bytes)?,
            None if self.path.exists() => fs::remove_file(&self.path)?,
            None => {}
        }
        Ok(())
    }
}

pub(crate) fn read_optional(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::error::Error::from(error).context(path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporter::Reporter;

    #[test]
    fn rollback_restores_our_own_writes_but_never_a_concurrent_edit() {
        let root = std::env::temp_dir().join(format!("fusor-transaction-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let _cleanup = Staging(root.clone());
        let path = root.join("Cargo.toml");
        fs::write(&path, "before").unwrap();

        let mut owned = OwnedFile::snapshot(path.clone()).unwrap();
        fs::write(&path, "operation").unwrap();
        owned.capture().unwrap();

        fs::write(&path, "user edit").unwrap();
        owned.rollback(&Reporter::default()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "user edit");

        fs::write(&path, "operation").unwrap();
        owned.rollback(&Reporter::default()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "before");
    }
}
