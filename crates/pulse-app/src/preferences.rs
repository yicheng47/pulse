use std::{fs, io, path::PathBuf};

const APP_DIRECTORY_NAME: &str = if cfg!(debug_assertions) {
    "pulse-dev"
} else {
    "pulse"
};

pub fn load_output_device_uid() -> io::Result<Option<String>> {
    let path = output_device_uid_path()?;
    match fs::read_to_string(path) {
        Ok(contents) => Ok(parse_output_device_uid(&contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn save_output_device_uid(uid: &str) -> io::Result<()> {
    let path = output_device_uid_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, uid)
}

pub fn library_database_path() -> io::Result<PathBuf> {
    Ok(app_data_directory()?.join("library.sqlite"))
}

pub fn cover_cache_directory() -> io::Result<PathBuf> {
    Ok(app_data_directory()?.join("covers"))
}

fn app_data_directory() -> io::Result<PathBuf> {
    dirs::data_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME))
        .ok_or_else(|| io::Error::other("could not determine the app data directory"))
}

fn output_device_uid_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME).join("app-output-device.uid"))
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

fn parse_output_device_uid(contents: &str) -> Option<String> {
    let uid = contents.trim();
    (!uid.is_empty()).then(|| uid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_single_uid_preference() {
        assert_eq!(
            parse_output_device_uid("  BuiltInSpeakerDevice\n"),
            Some("BuiltInSpeakerDevice".to_string())
        );
        assert_eq!(parse_output_device_uid(" \n"), None);
    }

    #[test]
    fn app_preference_does_not_share_the_cli_file() {
        assert_eq!(
            output_device_uid_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("app-output-device.uid")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_app_preference_uses_the_dev_profile() {
        assert!(
            output_device_uid_path()
                .unwrap()
                .parent()
                .unwrap()
                .ends_with("pulse-dev")
        );
    }

    #[test]
    fn library_files_share_the_app_data_directory() {
        assert_eq!(
            library_database_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("library.sqlite")
        );
        assert_eq!(
            cover_cache_directory()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("covers")
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_library_uses_the_dev_profile() {
        assert!(app_data_directory().unwrap().ends_with("pulse-dev"));
    }
}
