use std::{
    fs, io,
    path::{Path, PathBuf},
};

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

pub fn load_exclusive_mode() -> io::Result<bool> {
    load_exclusive_mode_from(&exclusive_mode_disabled_path()?)
}

pub fn save_exclusive_mode(enabled: bool) -> io::Result<()> {
    save_exclusive_mode_to(&exclusive_mode_disabled_path()?, enabled)
}

pub fn load_volume_level() -> io::Result<f32> {
    load_volume_level_from(&volume_level_path()?)
}

pub fn save_volume_level(level: f32) -> io::Result<()> {
    save_volume_level_to(&volume_level_path()?, level)
}

pub fn load_volume_muted() -> io::Result<bool> {
    load_volume_muted_from(&volume_muted_path()?)
}

pub fn save_volume_muted(muted: bool) -> io::Result<()> {
    save_volume_muted_to(&volume_muted_path()?, muted)
}

#[cfg(all(target_os = "macos", feature = "updater"))]
pub fn take_legacy_update_check_preference() -> io::Result<Option<bool>> {
    take_legacy_update_check_preference_from(&check_updates_disabled_path()?)
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

fn exclusive_mode_disabled_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| {
            path.join(APP_DIRECTORY_NAME)
                .join("exclusive-mode.disabled")
        })
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

#[cfg(any(all(target_os = "macos", feature = "updater"), test))]
fn check_updates_disabled_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME).join("check-updates.disabled"))
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

fn volume_level_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME).join("volume.level"))
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

fn volume_muted_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME).join("volume.muted"))
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

fn load_exclusive_mode_from(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn save_exclusive_mode_to(path: &Path, enabled: bool) -> io::Result<()> {
    if enabled {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, [])
}

#[cfg(any(all(target_os = "macos", feature = "updater"), test))]
fn take_legacy_update_check_preference_from(path: &Path) -> io::Result<Option<bool>> {
    match fs::remove_file(path) {
        Ok(()) => Ok(Some(false)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_volume_level_from(path: &Path) -> io::Result<f32> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(1.0),
        Err(error) => return Err(error),
    };
    let level = contents
        .trim()
        .parse::<f32>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !(0.0..=1.0).contains(&level) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "volume level must be between 0 and 1",
        ));
    }
    Ok(level)
}

fn save_volume_level_to(path: &Path, level: f32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, level.to_string())
}

fn load_volume_muted_from(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn save_volume_muted_to(path: &Path, muted: bool) -> io::Result<()> {
    if muted {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        return fs::write(path, []);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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
        assert_eq!(
            exclusive_mode_disabled_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("exclusive-mode.disabled")
        );
        assert_eq!(
            check_updates_disabled_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("check-updates.disabled")
        );
        assert_eq!(
            volume_level_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("volume.level")
        );
        assert_eq!(
            volume_muted_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("volume.muted")
        );
    }

    #[test]
    fn exclusive_mode_round_trips_and_defaults_on_without_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-mode.disabled");

        assert!(load_exclusive_mode_from(&path).unwrap());
        save_exclusive_mode_to(&path, false).unwrap();
        assert!(!load_exclusive_mode_from(&path).unwrap());
        save_exclusive_mode_to(&path, true).unwrap();
        assert!(load_exclusive_mode_from(&path).unwrap());
    }

    #[test]
    fn legacy_update_marker_seeds_disabled_once_then_disappears() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("check-updates.disabled");

        assert_eq!(
            take_legacy_update_check_preference_from(&path).unwrap(),
            None
        );
        fs::write(&path, []).unwrap();
        assert_eq!(
            take_legacy_update_check_preference_from(&path).unwrap(),
            Some(false)
        );
        assert!(!path.exists());
        assert_eq!(
            take_legacy_update_check_preference_from(&path).unwrap(),
            None
        );
    }

    #[test]
    fn volume_level_round_trips_and_defaults_to_unity_without_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("volume.level");

        assert_eq!(load_volume_level_from(&path).unwrap(), 1.0);
        save_volume_level_to(&path, 0.42).unwrap();
        assert_eq!(load_volume_level_from(&path).unwrap(), 0.42);
    }

    #[test]
    fn volume_mute_round_trips_and_defaults_off_without_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("volume.muted");

        assert!(!load_volume_muted_from(&path).unwrap());
        save_volume_muted_to(&path, true).unwrap();
        assert!(load_volume_muted_from(&path).unwrap());
        save_volume_muted_to(&path, false).unwrap();
        assert!(!load_volume_muted_from(&path).unwrap());
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
