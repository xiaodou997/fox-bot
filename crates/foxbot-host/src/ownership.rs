use crate::{HostError, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

/// One lock for every compliant host under the current OS user, independent of the
/// selected ledger directory. Not machine-wide across users, devices or old G1 CLIs.
pub struct DeviceOwner {
    file: File,
    path: PathBuf,
}
impl DeviceOwner {
    pub fn acquire() -> Result<Self> {
        let base = dirs::data_local_dir().ok_or(HostError::Ownership)?;
        Self::acquire_at(&base.join("FoxBot").join("execution-v1"))
    }
    pub(crate) fn acquire_at(root: &Path) -> Result<Self> {
        private_directory(root)?;
        let path = root.join("device-owner.lock");
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() || m.file_type().is_symlink() => {
                return Err(HostError::Ownership);
            }
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(HostError::Ownership),
            _ => {}
        }
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&path).map_err(|_| HostError::Ownership)?;
        file.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => HostError::Busy,
            _ => HostError::Ownership,
        })?;
        let owner = Self { file, path };
        owner.verify()?;
        Ok(owner)
    }
    pub fn verify(&self) -> Result<()> {
        let live = fs::symlink_metadata(&self.path).map_err(|_| HostError::Ownership)?;
        if !live.is_file() || live.file_type().is_symlink() {
            return Err(HostError::Ownership);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let held = self.file.metadata().map_err(|_| HostError::Ownership)?;
            if held.ino() != live.ino()
                || held.dev() != live.dev()
                || live.nlink() != 1
                || live.permissions().mode() & 0o077 != 0
            {
                return Err(HostError::Ownership);
            }
        }
        #[cfg(not(unix))]
        {
            self.file.metadata().map_err(|_| HostError::Ownership)?;
        }
        Ok(())
    }
}
// Never unlink a lock on Drop: all later processes must observe the same inode.
pub(crate) fn private_directory(path: &Path) -> Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(path).map_err(|_| HostError::Ownership)?;
    let m = fs::symlink_metadata(path).map_err(|_| HostError::Ownership)?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(HostError::Ownership);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o077 != 0 {
            return Err(HostError::Ownership);
        }
    }
    Ok(())
}
