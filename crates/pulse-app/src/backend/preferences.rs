use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use super::settings::{
    AppSettings, ExclusiveModePreferences, StoredDeviceCapabilities, StoredDevicePreferences,
    settings_path,
};

const APP_DIRECTORY_NAME: &str = if cfg!(debug_assertions) {
    "pulse-dev"
} else {
    "pulse"
};

const EXCLUSIVE_MODES_VERSION: &str = "version=2";
const LEGACY_EXCLUSIVE_MODES_VERSION: &str = "version=1";

pub fn library_database_path() -> io::Result<PathBuf> {
    Ok(app_data_directory()?.join("library.sqlite"))
}

pub fn cover_cache_directory() -> io::Result<PathBuf> {
    Ok(app_data_directory()?.join("covers"))
}

pub fn app_data_directory() -> io::Result<PathBuf> {
    dirs::data_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME))
        .ok_or_else(|| io::Error::other("could not determine the app data directory"))
}

pub(crate) fn legacy_config_directory() -> io::Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join(APP_DIRECTORY_NAME))
        .ok_or_else(|| io::Error::other("could not determine the app configuration directory"))
}

#[cfg(all(target_os = "macos", feature = "updater"))]
pub fn take_legacy_update_check_preference() -> io::Result<Option<bool>> {
    take_legacy_update_check_preference_from(&check_updates_disabled_path()?)
}

pub(crate) fn load_or_migrate_app_settings() -> io::Result<AppSettings> {
    load_or_migrate_app_settings_from(&app_data_directory()?, &legacy_config_directory()?)
}

pub(crate) fn load_or_migrate_app_settings_from(
    app_data_dir: &Path,
    legacy_config_dir: &Path,
) -> io::Result<AppSettings> {
    let path = settings_path(app_data_dir);
    let legacy_paths = LegacyPreferencePaths::new(legacy_config_dir);
    if path_exists(&path)? {
        let settings = AppSettings::load(&path)?;
        if !path_exists(&path)? {
            settings.save(&path)?;
        }
        return Ok(settings);
    }

    let settings = migrate_legacy_preferences(app_data_dir, &legacy_paths)?;
    settings.save(&path)?;
    remove_legacy_preferences(&legacy_paths)?;
    Ok(settings)
}

struct LegacyPreferencePaths {
    output_device_uid: PathBuf,
    exclusive_mode_disabled: PathBuf,
    exclusive_modes: PathBuf,
    volume_level: PathBuf,
    volume_muted: PathBuf,
}

impl LegacyPreferencePaths {
    fn new(directory: &Path) -> Self {
        Self {
            output_device_uid: directory.join("app-output-device.uid"),
            exclusive_mode_disabled: directory.join("exclusive-mode.disabled"),
            exclusive_modes: directory.join("exclusive-modes.tsv"),
            volume_level: directory.join("volume.level"),
            volume_muted: directory.join("volume.muted"),
        }
    }

    fn all(&self) -> [&Path; 5] {
        [
            &self.output_device_uid,
            &self.exclusive_mode_disabled,
            &self.exclusive_modes,
            &self.volume_level,
            &self.volume_muted,
        ]
    }
}

enum LegacyText {
    Missing,
    Contents(String),
    Corrupt,
}

fn migrate_legacy_preferences(
    app_data_dir: &Path,
    paths: &LegacyPreferencePaths,
) -> io::Result<AppSettings> {
    let saved_output_device_uid = match read_legacy_text(&paths.output_device_uid)? {
        LegacyText::Contents(contents) => parse_output_device_uid(&contents),
        LegacyText::Missing | LegacyText::Corrupt => None,
    };
    let (exclusive_mode_preferences, had_exclusive_modes) =
        match read_legacy_text(&paths.exclusive_modes)? {
            LegacyText::Contents(contents) => match parse_exclusive_mode_preferences(&contents) {
                Ok(preferences) => (preferences, true),
                Err(error) => {
                    archive_corrupt_legacy_file(&paths.exclusive_modes, &error)?;
                    (ExclusiveModePreferences::default(), true)
                }
            },
            LegacyText::Corrupt => (ExclusiveModePreferences::default(), true),
            LegacyText::Missing => (ExclusiveModePreferences::default(), false),
        };
    let legacy_exclusive_mode_disabled = if had_exclusive_modes {
        None
    } else {
        let disabled = path_exists(&paths.exclusive_mode_disabled)?;
        let legacy_install_exists = app_data_dir.join("library.sqlite").exists();
        (disabled || legacy_install_exists).then_some(disabled)
    };
    let volume_level = match read_legacy_text(&paths.volume_level)? {
        LegacyText::Contents(contents) => match parse_volume_level(&contents) {
            Ok(level) => level,
            Err(error) => {
                archive_corrupt_legacy_file(&paths.volume_level, &error)?;
                1.0
            }
        },
        LegacyText::Missing | LegacyText::Corrupt => 1.0,
    };
    let volume_muted = path_exists(&paths.volume_muted)?;

    Ok(AppSettings {
        saved_output_device_uid,
        exclusive_mode_preferences,
        legacy_exclusive_mode_disabled,
        volume_level,
        volume_muted,
        interface_scale: 1.0,
        session: None,
    })
}

fn remove_legacy_preferences(paths: &LegacyPreferencePaths) -> io::Result<()> {
    for path in paths.all() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn read_legacy_text(path: &Path) -> io::Result<LegacyText> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(LegacyText::Contents(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LegacyText::Missing),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            archive_corrupt_legacy_file(path, &error)?;
            Ok(LegacyText::Corrupt)
        }
        Err(error) => Err(error),
    }
}

fn archive_corrupt_legacy_file(path: &Path, error: &dyn std::fmt::Display) -> io::Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("legacy preference path has no file name"))?;
    for index in 0..100 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let archived_path = path.with_file_name(format!("{file_name}.corrupt{suffix}"));
        match fs::symlink_metadata(&archived_path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(path, &archived_path)?;
        eprintln!(
            "Could not migrate {}: {error}. Moved it to {} and used the default value.",
            path.display(),
            archived_path.display()
        );
        return Ok(archived_path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a corrupt legacy preference backup path",
    ))
}

fn path_exists(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn parse_exclusive_mode_preferences(contents: &str) -> io::Result<ExclusiveModePreferences> {
    let mut lines = contents.lines();
    let version = lines.next();
    if version == Some(LEGACY_EXCLUSIVE_MODES_VERSION) {
        return parse_legacy_exclusive_mode_preferences(lines);
    }
    if version != Some(EXCLUSIVE_MODES_VERSION) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported exclusive-mode preference version",
        ));
    }

    let mut devices = BTreeMap::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid exclusive-mode preference entry",
            ));
        }
        let exclusive_mode = parse_exclusive_mode(fields[0])?;
        let uid = unescape_field(fields[1])?;
        if uid.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "exclusive-mode device UID is empty",
            ));
        }
        let name = match unescape_field(fields[2])? {
            name if name.is_empty() => None,
            name => Some(name),
        };
        let capabilities = parse_stored_capabilities(fields[3], fields[4])?;
        let last_seen_unix_seconds = if fields[5].is_empty() {
            None
        } else {
            Some(
                fields[5]
                    .parse::<u64>()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            )
        };
        devices.insert(
            uid,
            StoredDevicePreferences {
                name,
                capabilities,
                last_seen_unix_seconds,
                exclusive_mode,
            },
        );
    }
    Ok(ExclusiveModePreferences { devices })
}

fn parse_legacy_exclusive_mode_preferences<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> io::Result<ExclusiveModePreferences> {
    let mut preferences = ExclusiveModePreferences::default();
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
        let Some(enabled) = parse_exclusive_mode(mode)? else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "legacy exclusive-mode preference cannot be automatic",
            ));
        };
        preferences.set_override(uid, enabled);
    }
    Ok(preferences)
}

fn parse_exclusive_mode(mode: &str) -> io::Result<Option<bool>> {
    match mode {
        "auto" => Ok(None),
        "exclusive" => Ok(Some(true)),
        "shared" => Ok(Some(false)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid exclusive-mode preference value",
        )),
    }
}

fn parse_stored_capabilities(
    bit_depth: &str,
    sample_rate: &str,
) -> io::Result<Option<StoredDeviceCapabilities>> {
    if bit_depth == "unknown" {
        if !sample_rate.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown device capabilities include a sample rate",
            ));
        }
        return Ok(None);
    }
    let max_bits_per_channel = if bit_depth == "float" {
        None
    } else {
        Some(
            bit_depth
                .parse::<u32>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        )
    };
    let max_sample_rate = sample_rate
        .parse::<u32>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(StoredDeviceCapabilities {
        max_bits_per_channel,
        max_sample_rate,
    }))
}

fn unescape_field(value: &str) -> io::Result<String> {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => unescaped.push('\\'),
            Some('t') => unescaped.push('\t'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid escaped preference field",
                ));
            }
        }
    }
    Ok(unescaped)
}

fn parse_output_device_uid(contents: &str) -> Option<String> {
    let uid = contents.trim();
    (!uid.is_empty()).then(|| uid.to_string())
}

fn parse_volume_level(contents: &str) -> io::Result<f32> {
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

#[cfg(all(target_os = "macos", feature = "updater"))]
fn check_updates_disabled_path() -> io::Result<PathBuf> {
    Ok(legacy_config_directory()?.join("check-updates.disabled"))
}

#[cfg(any(all(target_os = "macos", feature = "updater"), test))]
fn take_legacy_update_check_preference_from(path: &Path) -> io::Result<Option<bool>> {
    match fs::remove_file(path) {
        Ok(()) => Ok(Some(false)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
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
    fn migrates_every_flat_preference_after_writing_json() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("app-output-device.uid"), " matrix ").unwrap();
        fs::write(
            legacy.join("exclusive-modes.tsv"),
            "version=2\nshared\tmatrix\\\\uid\tMatrix\\tmini-i\\nSeries\t24\t192000\t1777777777\nauto\tairpods\tAirPods\tfloat\t48000\t1666666666\n",
        )
        .unwrap();
        fs::write(legacy.join("exclusive-mode.disabled"), []).unwrap();
        fs::write(legacy.join("volume.level"), "0.42").unwrap();
        fs::write(legacy.join("volume.muted"), []).unwrap();
        fs::write(legacy.join("check-updates.disabled"), []).unwrap();

        let migrated = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(migrated.saved_output_device_uid.as_deref(), Some("matrix"));
        assert_eq!(migrated.volume_level, 0.42);
        assert!(migrated.volume_muted);
        assert!(
            !migrated
                .exclusive_mode_preferences
                .effective_mode("matrix\\uid", true)
        );
        let stored = migrated
            .exclusive_mode_preferences
            .devices
            .get("matrix\\uid")
            .unwrap();
        assert_eq!(stored.name.as_deref(), Some("Matrix\tmini-i\nSeries"));
        assert_eq!(
            stored.capabilities,
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            })
        );
        assert_eq!(stored.last_seen_unix_seconds, Some(1_777_777_777));
        assert_eq!(
            AppSettings::load(&settings_path(&app_data)).unwrap(),
            migrated
        );
        for path in LegacyPreferencePaths::new(&legacy).all() {
            assert!(!path.exists(), "{} still exists", path.display());
        }
        assert!(legacy.join("check-updates.disabled").exists());
    }

    #[test]
    fn fresh_directory_writes_default_settings() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");

        let loaded = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(loaded, AppSettings::default());
        assert_eq!(
            AppSettings::load(&settings_path(&app_data)).unwrap(),
            AppSettings::default()
        );
        assert!(settings_path(&app_data).exists());
    }

    #[test]
    fn existing_settings_ignore_and_preserve_legacy_files() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("volume.level"), "0.9").unwrap();
        let current = AppSettings {
            volume_level: 0.2,
            ..AppSettings::default()
        };
        current.save(&settings_path(&app_data)).unwrap();

        let loaded = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(loaded.volume_level, 0.2);
        assert_eq!(
            fs::read_to_string(legacy.join("volume.level")).unwrap(),
            "0.9"
        );
    }

    #[test]
    fn corrupt_settings_are_archived_and_replaced_without_using_legacy_values() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&app_data).unwrap();
        fs::create_dir_all(&legacy).unwrap();
        fs::write(settings_path(&app_data), "{broken").unwrap();
        fs::write(legacy.join("volume.level"), "0.3").unwrap();

        let loaded = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(loaded, AppSettings::default());
        assert_eq!(
            fs::read_to_string(app_data.join("settings.corrupt.json")).unwrap(),
            "{broken"
        );
        assert_eq!(
            AppSettings::load(&settings_path(&app_data)).unwrap(),
            AppSettings::default()
        );
        assert_eq!(
            fs::read_to_string(legacy.join("volume.level")).unwrap(),
            "0.3"
        );
    }

    #[test]
    fn malformed_volume_is_archived_without_blocking_other_fields() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("app-output-device.uid"), "matrix").unwrap();
        fs::write(legacy.join("volume.level"), "loud").unwrap();

        let migrated = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(migrated.saved_output_device_uid.as_deref(), Some("matrix"));
        assert_eq!(migrated.volume_level, 1.0);
        assert!(settings_path(&app_data).exists());
        assert_eq!(
            fs::read_to_string(legacy.join("volume.level.corrupt")).unwrap(),
            "loud"
        );
    }

    #[test]
    fn malformed_device_table_is_archived_without_blocking_other_fields() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("app-output-device.uid"), "matrix").unwrap();
        fs::write(legacy.join("exclusive-modes.tsv"), "").unwrap();
        fs::write(legacy.join("volume.level"), "0.42").unwrap();

        let migrated = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(migrated.saved_output_device_uid.as_deref(), Some("matrix"));
        assert!(migrated.exclusive_mode_preferences.devices.is_empty());
        assert_eq!(migrated.legacy_exclusive_mode_disabled, None);
        assert_eq!(migrated.volume_level, 0.42);
        assert!(settings_path(&app_data).exists());
        assert_eq!(
            fs::read_to_string(legacy.join("exclusive-modes.tsv.corrupt")).unwrap(),
            ""
        );
    }

    #[test]
    fn parses_version_one_overrides_for_migration() {
        let preferences =
            parse_exclusive_mode_preferences("version=1\nexclusive\tmatrix\nshared\tairpods\n")
                .unwrap();

        assert!(preferences.effective_mode("matrix", false));
        assert!(!preferences.effective_mode("airpods", true));
        assert_eq!(preferences.devices.len(), 2);
    }

    #[test]
    fn rejects_malformed_device_tables_without_deleting_them() {
        for contents in [
            "version=2\nauto\tmatrix\n",
            "version=2\nauto\tmatrix\\q\tMatrix\tunknown\t\t100\n",
            "version=2\nauto\tmatrix\tMatrix\tunknown\t48000\t100\n",
        ] {
            assert_eq!(
                parse_exclusive_mode_preferences(contents)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn legacy_disabled_marker_is_preserved_until_an_active_device_is_known() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("exclusive-mode.disabled"), []).unwrap();

        let settings = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(settings.legacy_exclusive_mode_disabled, Some(true));
        assert!(!legacy.join("exclusive-mode.disabled").exists());
    }

    #[test]
    fn legacy_install_default_is_preserved_until_an_active_device_is_known() {
        let root = tempfile::tempdir().unwrap();
        let app_data = root.path().join("data");
        let legacy = root.path().join("config");
        fs::create_dir_all(&app_data).unwrap();
        fs::write(app_data.join("library.sqlite"), []).unwrap();

        let settings = load_or_migrate_app_settings_from(&app_data, &legacy).unwrap();

        assert_eq!(settings.legacy_exclusive_mode_disabled, Some(false));
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
    fn debug_build_uses_the_dev_profile_for_settings_and_library() {
        let app_data = app_data_directory().unwrap();
        assert!(app_data.ends_with("pulse-dev"));
        assert_eq!(
            settings_path(&app_data)
                .file_name()
                .and_then(|name| name.to_str()),
            Some("settings.json")
        );
        assert!(legacy_config_directory().unwrap().ends_with("pulse-dev"));
    }
}
