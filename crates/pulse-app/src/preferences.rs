use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

const APP_DIRECTORY_NAME: &str = if cfg!(debug_assertions) {
    "pulse-dev"
} else {
    "pulse"
};

const EXCLUSIVE_MODES_VERSION: &str = "version=1";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExclusiveModePreferences {
    overrides: BTreeMap<String, bool>,
}

impl ExclusiveModePreferences {
    pub fn effective_mode(&self, device_uid: &str, default: bool) -> bool {
        self.overrides.get(device_uid).copied().unwrap_or(default)
    }

    pub fn is_overridden(&self, device_uid: &str) -> bool {
        self.overrides.contains_key(device_uid)
    }

    pub fn set_override(&mut self, device_uid: &str, enabled: bool) {
        self.overrides.insert(device_uid.to_string(), enabled);
    }

    pub fn clear_override(&mut self, device_uid: &str) {
        self.overrides.remove(device_uid);
    }
}

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

pub fn load_exclusive_mode_preferences(
    active_device_uid: &str,
) -> io::Result<ExclusiveModePreferences> {
    load_exclusive_mode_preferences_from(
        &exclusive_modes_path()?,
        &exclusive_mode_disabled_path()?,
        library_database_path()?.exists(),
        active_device_uid,
    )
}

pub fn save_exclusive_mode_preferences(preferences: &ExclusiveModePreferences) -> io::Result<()> {
    save_exclusive_mode_preferences_to(&exclusive_modes_path()?, preferences)
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

fn exclusive_modes_path() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME).join("exclusive-modes.tsv"))
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

fn load_exclusive_mode_preferences_from(
    path: &Path,
    legacy_disabled_path: &Path,
    legacy_install_exists: bool,
    active_device_uid: &str,
) -> io::Result<ExclusiveModePreferences> {
    match fs::read_to_string(path) {
        Ok(contents) => return parse_exclusive_mode_preferences(&contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let legacy_disabled = match fs::metadata(legacy_disabled_path) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    let mut preferences = ExclusiveModePreferences::default();
    if legacy_disabled || legacy_install_exists {
        preferences
            .overrides
            .insert(active_device_uid.to_string(), !legacy_disabled);
    }
    save_exclusive_mode_preferences_to(path, &preferences)?;
    if legacy_disabled {
        fs::remove_file(legacy_disabled_path)?;
    }
    Ok(preferences)
}

fn parse_exclusive_mode_preferences(contents: &str) -> io::Result<ExclusiveModePreferences> {
    let mut lines = contents.lines();
    if lines.next() != Some(EXCLUSIVE_MODES_VERSION) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported exclusive-mode preference version",
        ));
    }

    let mut overrides = BTreeMap::new();
    for line in lines {
        let (mode, uid) = line.split_once('\t').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid exclusive-mode preference entry",
            )
        })?;
        if uid.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "exclusive-mode device UID is empty",
            ));
        }
        let enabled = match mode {
            "exclusive" => true,
            "shared" => false,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid exclusive-mode preference value",
                ));
            }
        };
        overrides.insert(uid.to_string(), enabled);
    }
    Ok(ExclusiveModePreferences { overrides })
}

fn save_exclusive_mode_preferences_to(
    path: &Path,
    preferences: &ExclusiveModePreferences,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut contents = String::from(EXCLUSIVE_MODES_VERSION);
    contents.push('\n');
    for (uid, enabled) in &preferences.overrides {
        contents.push_str(if *enabled { "exclusive\t" } else { "shared\t" });
        contents.push_str(uid);
        contents.push('\n');
    }
    fs::write(path, contents)
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
            exclusive_modes_path()
                .unwrap()
                .file_name()
                .and_then(|name| name.to_str()),
            Some("exclusive-modes.tsv")
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
    fn exclusive_mode_preferences_round_trip_two_device_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let mut preferences = ExclusiveModePreferences::default();
        preferences.set_override("matrix", false);
        preferences.set_override("airpods", true);

        save_exclusive_mode_preferences_to(&path, &preferences).unwrap();
        let loaded = load_exclusive_mode_preferences_from(
            &path,
            &directory.path().join("exclusive-mode.disabled"),
            false,
            "matrix",
        )
        .unwrap();

        assert!(!loaded.effective_mode("matrix", true));
        assert!(loaded.effective_mode("airpods", false));
        assert!(loaded.is_overridden("matrix"));
        assert!(loaded.is_overridden("airpods"));
        assert!(loaded.effective_mode("unset-dac", true));
        assert!(!loaded.effective_mode("unset-bluetooth", false));
    }

    #[test]
    fn explicit_mode_equal_to_the_default_remains_pinned() {
        let mut preferences = ExclusiveModePreferences::default();
        preferences.set_override("matrix", true);

        assert!(preferences.is_overridden("matrix"));
        assert!(preferences.effective_mode("matrix", false));
    }

    #[test]
    fn clearing_an_override_returns_the_device_to_its_default() {
        let mut preferences = ExclusiveModePreferences::default();
        preferences.set_override("matrix", false);

        preferences.clear_override("matrix");

        assert!(!preferences.is_overridden("matrix"));
        assert!(preferences.effective_mode("matrix", true));
    }

    #[test]
    fn migrates_the_legacy_disabled_marker_to_the_active_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let legacy_path = directory.path().join("exclusive-mode.disabled");
        fs::write(&legacy_path, []).unwrap();

        let preferences =
            load_exclusive_mode_preferences_from(&path, &legacy_path, true, "airpods").unwrap();

        assert_eq!(preferences.overrides.get("airpods"), Some(&false));
        assert!(!legacy_path.exists());
    }

    #[test]
    fn migrates_the_legacy_enabled_default_to_the_active_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let legacy_path = directory.path().join("exclusive-mode.disabled");

        let preferences =
            load_exclusive_mode_preferences_from(&path, &legacy_path, true, "matrix").unwrap();

        assert_eq!(preferences.overrides.get("matrix"), Some(&true));
    }

    #[test]
    fn fresh_preferences_leave_devices_unset_for_capability_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let legacy_path = directory.path().join("exclusive-mode.disabled");

        let preferences =
            load_exclusive_mode_preferences_from(&path, &legacy_path, false, "airpods").unwrap();

        assert!(!preferences.is_overridden("airpods"));
        assert!(!preferences.effective_mode("airpods", false));
        assert_eq!(fs::read_to_string(path).unwrap(), "version=1\n");
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
