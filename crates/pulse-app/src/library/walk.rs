use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::{LibraryError, path::path_identity, system_time_ns};

#[derive(Debug)]
pub(super) struct DiscoveredFile {
    pub path: PathBuf,
    pub path_text: String,
    pub path_key: String,
    pub modified_at_ns: i64,
    pub file_size_bytes: u64,
}

#[derive(Debug)]
pub(super) struct WalkError {
    pub path: PathBuf,
    pub message: String,
}

pub(super) struct WalkResult {
    pub files: Vec<DiscoveredFile>,
    pub errors: Vec<WalkError>,
}

pub(super) fn walk_music_files<F>(
    root: &Path,
    is_case_sensitive: bool,
    mut on_discovered: F,
) -> io::Result<WalkResult>
where
    F: FnMut(usize, &Path),
{
    let mut result = WalkResult {
        files: Vec::new(),
        errors: Vec::new(),
    };
    walk_directory(
        root,
        is_case_sensitive,
        true,
        &mut result,
        &mut on_discovered,
    )?;
    result
        .files
        .sort_by(|left, right| left.path_key.cmp(&right.path_key));
    Ok(result)
}

fn walk_directory<F>(
    directory: &Path,
    is_case_sensitive: bool,
    is_root: bool,
    result: &mut WalkResult,
    on_discovered: &mut F,
) -> io::Result<()>
where
    F: FnMut(usize, &Path),
{
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if is_root => return Err(error),
        Err(error) => {
            result.errors.push(WalkError {
                path: directory.to_path_buf(),
                message: error.to_string(),
            });
            return Ok(());
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                result.errors.push(WalkError {
                    path: directory.to_path_buf(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                result.errors.push(WalkError {
                    path,
                    message: error.to_string(),
                });
                continue;
            }
        };

        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk_directory(&path, is_case_sensitive, false, result, on_discovered)?;
            continue;
        }
        if !file_type.is_file() || !is_supported_music_file(&path) {
            continue;
        }

        match discovered_file(path, is_case_sensitive) {
            Ok(file) => {
                result.files.push(file);
                let file = result.files.last().expect("just pushed a discovered file");
                on_discovered(result.files.len(), &file.path);
            }
            Err(error) => result.errors.push(WalkError {
                path: error_path(&error),
                message: error.to_string(),
            }),
        }
    }

    Ok(())
}

fn discovered_file(path: PathBuf, is_case_sensitive: bool) -> Result<DiscoveredFile, LibraryError> {
    let metadata = fs::metadata(&path).map_err(|source| LibraryError::Io {
        path: path.clone(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| LibraryError::Io {
        path: path.clone(),
        source,
    })?;
    let modified_at_ns = system_time_ns(modified).map_err(|error| match error {
        LibraryError::IntegerOutOfRange(_) => LibraryError::FileTimestampOutOfRange(path.clone()),
        error => error,
    })?;
    let (path_text, path_key) = path_identity(&path, is_case_sensitive)?;

    Ok(DiscoveredFile {
        path,
        path_text,
        path_key,
        modified_at_ns,
        file_size_bytes: metadata.len(),
    })
}

fn error_path(error: &LibraryError) -> PathBuf {
    match error {
        LibraryError::Io { path, .. }
        | LibraryError::NonUnicodePath(path)
        | LibraryError::NotDirectory(path)
        | LibraryError::FileTimestampOutOfRange(path) => path.clone(),
        _ => PathBuf::new(),
    }
}

fn is_supported_music_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "flac" | "m4a" | "aif" | "aiff" | "wav"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::{fs::File, os::unix::fs::symlink};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn walks_nested_pcm_files_and_filters_everything_else() {
        let temp = tempdir().unwrap();
        let nested = temp.path().join("Nested");
        fs::create_dir(&nested).unwrap();
        for path in [
            temp.path().join("one.FLAC"),
            temp.path().join("two.m4a"),
            nested.join("three.aif"),
            nested.join("four.aiff"),
            nested.join("five.wav"),
            nested.join("ignore.mp3"),
            nested.join("ignore.dsf"),
            nested.join("ignore.mp4"),
        ] {
            File::create(path).unwrap();
        }
        symlink(temp.path(), nested.join("loop")).unwrap();

        let result = walk_music_files(temp.path(), true, |_, _| {}).unwrap();
        let names = result
            .files
            .iter()
            .map(|file| {
                file.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            ["five.wav", "four.aiff", "three.aif", "one.FLAC", "two.m4a"]
        );
        assert!(result.errors.is_empty());
    }

    #[test]
    fn walks_unicode_filenames_with_normalized_identity() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("Café 音楽.wav");
        File::create(&path).unwrap();

        let result = walk_music_files(temp.path(), false, |_, _| {}).unwrap();

        assert_eq!(result.files.len(), 1);
        assert!(result.files[0].path_key.ends_with("café 音楽.wav"));
    }
}
