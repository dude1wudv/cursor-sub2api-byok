//! Same-directory, flushed replacement; never delete the destination first.
use std::{fs, io::Write, path::Path};

use crate::Result;

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| crate::Error::Config("atomic write requires a parent directory".into()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".cursor-sub2api-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    fn wide(path: &Path) -> Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(crate::Error::Config("file path contains NUL".into()));
        }
        value.push(0);
        Ok(value)
    }
    let source = wide(source)?;
    let target = wide(target)?;
    // Both paths are in the same directory. No copy fallback or reboot delay.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_replaces_unicode_path_without_temp_residue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("设置.json");
        write(&path, b"first").unwrap();
        write(&path, b"second").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"second");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replace_preserves_target_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("directory");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"untouched").unwrap();
        assert!(write(&target, b"replacement").is_err());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"untouched");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
