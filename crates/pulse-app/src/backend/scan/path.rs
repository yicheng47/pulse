use std::{ffi::CString, fs, io, path::Path};

#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;

use unicode_normalization::UnicodeNormalization;

use super::super::LibraryError;

pub(in crate::backend) struct StorageRootPath {
    pub path_text: String,
    pub path_key: String,
    pub is_case_sensitive: bool,
}

pub(in crate::backend) fn normalize_storage_root(
    path: &Path,
) -> Result<StorageRootPath, LibraryError> {
    let path = fs::canonicalize(path).map_err(|source| LibraryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !path.is_dir() {
        return Err(LibraryError::NotDirectory(path));
    }

    let is_case_sensitive = volume_is_case_sensitive(&path).map_err(|source| LibraryError::Io {
        path: path.clone(),
        source,
    })?;
    let (path_text, path_key) = path_identity(&path, is_case_sensitive)?;

    Ok(StorageRootPath {
        path_text,
        path_key,
        is_case_sensitive,
    })
}

pub(in crate::backend) fn path_identity(
    path: &Path,
    is_case_sensitive: bool,
) -> Result<(String, String), LibraryError> {
    let path_text = path
        .to_str()
        .ok_or_else(|| LibraryError::NonUnicodePath(path.to_path_buf()))?
        .to_owned();
    let path_key = normalized_path_key(&path_text, is_case_sensitive);
    Ok((path_text, path_key))
}

fn normalized_path_key(path: &str, is_case_sensitive: bool) -> String {
    let normalized = path.nfc().collect::<String>();
    if is_case_sensitive {
        normalized
    } else {
        normalized
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .nfc()
            .collect()
    }
}

#[cfg(target_os = "macos")]
fn volume_is_case_sensitive(path: &Path) -> io::Result<bool> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;

    // SAFETY: `__error` returns this thread's errno pointer, and `pathconf` does not retain `path`.
    let result = unsafe {
        *libc::__error() = 0;
        libc::pathconf(path.as_ptr(), libc::_PC_CASE_SENSITIVE)
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(0) {
            Ok(false)
        } else {
            Err(error)
        }
    } else {
        Ok(result != 0)
    }
}

#[cfg(not(target_os = "macos"))]
fn volume_is_case_sensitive(_path: &Path) -> io::Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_unicode_without_merging_case_sensitive_paths() {
        let composed = normalized_path_key("/Music/Café/Track.wav", true);
        let decomposed = normalized_path_key("/Music/Cafe\u{301}/Track.wav", true);

        assert_eq!(composed, decomposed);
        assert_ne!(
            normalized_path_key("/Music/Track.wav", true),
            normalized_path_key("/music/track.wav", true)
        );
    }

    #[test]
    fn folds_case_only_for_case_insensitive_volumes() {
        assert_eq!(
            normalized_path_key("/Music/CAFÉ/Track.WAV", false),
            normalized_path_key("/music/cafe\u{301}/track.wav", false)
        );
    }
}
