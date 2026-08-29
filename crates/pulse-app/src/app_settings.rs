use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredDeviceCapabilities {
    pub max_bits_per_channel: Option<u32>,
    pub max_sample_rate: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoredDevicePreferences {
    pub name: Option<String>,
    pub capabilities: Option<StoredDeviceCapabilities>,
    pub last_seen_unix_seconds: Option<u64>,
    pub(crate) exclusive_mode: Option<bool>,
}

impl StoredDevicePreferences {
    pub fn exclusive_mode_override(&self) -> Option<bool> {
        self.exclusive_mode
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExclusiveModePreferences {
    pub(crate) devices: BTreeMap<String, StoredDevicePreferences>,
}

impl ExclusiveModePreferences {
    pub fn effective_mode(&self, device_uid: &str, default: bool) -> bool {
        self.devices
            .get(device_uid)
            .and_then(StoredDevicePreferences::exclusive_mode_override)
            .unwrap_or(default)
    }

    pub fn is_overridden(&self, device_uid: &str) -> bool {
        self.devices
            .get(device_uid)
            .is_some_and(|device| device.exclusive_mode.is_some())
    }

    pub fn set_override(&mut self, device_uid: &str, enabled: bool) {
        self.devices
            .entry(device_uid.to_string())
            .or_default()
            .exclusive_mode = Some(enabled);
    }

    pub fn clear_override(&mut self, device_uid: &str) {
        if let Some(device) = self.devices.get_mut(device_uid) {
            device.exclusive_mode = None;
        }
    }

    pub fn record_sighting(
        &mut self,
        device_uid: &str,
        name: &str,
        capabilities: Option<StoredDeviceCapabilities>,
        seen_at_unix_seconds: u64,
    ) {
        let device = self.devices.entry(device_uid.to_string()).or_default();
        device.name = Some(name.to_string());
        if capabilities.is_some() {
            device.capabilities = capabilities;
        }
        device.last_seen_unix_seconds = Some(seen_at_unix_seconds);
    }

    pub fn devices(&self) -> impl Iterator<Item = (&str, &StoredDevicePreferences)> {
        self.devices
            .iter()
            .map(|(uid, preferences)| (uid.as_str(), preferences))
    }

    pub fn stored_capabilities(&self, device_uid: &str) -> Option<StoredDeviceCapabilities> {
        self.devices
            .get(device_uid)
            .and_then(|device| device.capabilities)
    }

    pub fn forget(&mut self, device_uid: &str) -> bool {
        self.devices.remove(device_uid).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub saved_output_device_uid: Option<String>,
    pub exclusive_mode_preferences: ExclusiveModePreferences,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) legacy_exclusive_mode_disabled: Option<bool>,
    pub volume_level: f32,
    pub volume_muted: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            saved_output_device_uid: None,
            exclusive_mode_preferences: ExclusiveModePreferences::default(),
            legacy_exclusive_mode_disabled: None,
            volume_level: 1.0,
            volume_muted: false,
        }
    }
}

impl AppSettings {
    pub fn load(path: &Path) -> io::Result<Self> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return recover_invalid_settings(path, &error);
            }
            Err(error) => return Err(error),
        };
        let mut settings = match serde_json::from_str::<Self>(&contents) {
            Ok(settings) => settings,
            Err(error) => return recover_invalid_settings(path, &error),
        };
        settings.normalize();
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("settings path has no parent directory"))?;
        fs::create_dir_all(parent)?;

        let mut contents = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        contents.push(b'\n');
        let (mut temporary_file, temporary_path) = create_temporary_file(path)?;
        let result = (|| {
            temporary_file.write_all(&contents)?;
            temporary_file.sync_all()?;
            drop(temporary_file);
            fs::rename(&temporary_path, path)?;
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    pub fn normalize(&mut self) {
        self.saved_output_device_uid = self
            .saved_output_device_uid
            .take()
            .map(|uid| uid.trim().to_string())
            .filter(|uid| !uid.is_empty());
        if !self.volume_level.is_finite() || !(0.0..=1.0).contains(&self.volume_level) {
            self.volume_level = 1.0;
        }
    }
}

pub fn settings_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("settings.json")
}

fn create_temporary_file(path: &Path) -> io::Result<(File, PathBuf)> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("settings path has no file name"))?;
    for index in 0..100 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let temporary_path = path.with_file_name(format!(".{file_name}{suffix}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((file, temporary_path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary settings file",
    ))
}

fn recover_invalid_settings(path: &Path, error: &dyn std::fmt::Display) -> io::Result<AppSettings> {
    let archived_path = archive_invalid_settings(path)?;
    eprintln!(
        "Could not load {}: {error}. Moved it to {} and restored defaults.",
        path.display(),
        archived_path.display()
    );
    Ok(AppSettings::default())
}

fn archive_invalid_settings(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings path has no parent directory"))?;
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("settings path has no file stem"))?;
    let extension = path.extension().and_then(|extension| extension.to_str());
    for index in 0..100 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let file_name = match extension {
            Some(extension) => format!("{stem}.corrupt{suffix}.{extension}"),
            None => format!("{stem}.corrupt{suffix}"),
        };
        let archived_path = parent.join(file_name);
        match fs::symlink_metadata(&archived_path) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(path, &archived_path)?;
        return Ok(archived_path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a corrupt settings backup path",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_settings_round_trip_every_current_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        let mut settings = AppSettings {
            saved_output_device_uid: Some("matrix".to_string()),
            volume_level: 0.42,
            volume_muted: true,
            ..AppSettings::default()
        };
        settings.exclusive_mode_preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            1_777_777_777,
        );
        settings
            .exclusive_mode_preferences
            .set_override("matrix", false);

        settings.save(&path).unwrap();

        assert_eq!(AppSettings::load(&path).unwrap(), settings);
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains("\n  \"savedOutputDeviceUid\""));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn exclusive_mode_preferences_round_trip_two_device_overrides() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        let mut settings = AppSettings::default();
        settings
            .exclusive_mode_preferences
            .set_override("matrix", false);
        settings
            .exclusive_mode_preferences
            .set_override("airpods", true);

        settings.save(&path).unwrap();
        let loaded = AppSettings::load(&path).unwrap().exclusive_mode_preferences;

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
    fn offline_override_edit_persists_and_applies_when_the_device_reconnects() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        let mut settings = AppSettings::default();
        settings.exclusive_mode_preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            100,
        );
        settings
            .exclusive_mode_preferences
            .set_override("matrix", false);

        settings.save(&path).unwrap();
        let loaded = AppSettings::load(&path).unwrap().exclusive_mode_preferences;

        assert!(loaded.is_overridden("matrix"));
        assert!(!loaded.effective_mode("matrix", true));
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
    fn stored_device_details_round_trip_with_escaped_names_and_auto_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        let mut settings = AppSettings::default();
        settings.exclusive_mode_preferences.record_sighting(
            "matrix\\uid",
            "Matrix\tmini-i\nSeries",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            1_777_777_777,
        );

        settings.save(&path).unwrap();
        let loaded = AppSettings::load(&path).unwrap();

        assert_eq!(loaded, settings);
        let stored = loaded
            .exclusive_mode_preferences
            .devices
            .get("matrix\\uid")
            .unwrap();
        assert_eq!(stored.name.as_deref(), Some("Matrix\tmini-i\nSeries"));
        assert_eq!(stored.exclusive_mode_override(), None);
    }

    #[test]
    fn a_forgotten_device_reconnects_with_fresh_probe_defaults() {
        let mut preferences = ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix",
            "Old name",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: None,
                max_sample_rate: 48_000,
            }),
            100,
        );
        preferences.set_override("matrix", false);

        assert!(preferences.forget("matrix"));
        preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            200,
        );

        assert!(!preferences.is_overridden("matrix"));
        assert!(preferences.effective_mode("matrix", true));
        let stored = preferences.devices.get("matrix").unwrap();
        assert_eq!(stored.name.as_deref(), Some("mini-i Series"));
        assert_eq!(stored.last_seen_unix_seconds, Some(200));
    }

    #[test]
    fn missing_and_unknown_json_fields_use_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::write(
            &path,
            r#"{"futureSetting":{"enabled":true},"volumeMuted":true}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();

        assert_eq!(loaded.saved_output_device_uid, None);
        assert_eq!(loaded.volume_level, 1.0);
        assert!(loaded.volume_muted);
    }

    #[test]
    fn corrupt_settings_are_archived_before_defaults_are_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::write(&path, "{not json").unwrap();

        assert_eq!(AppSettings::load(&path).unwrap(), AppSettings::default());

        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(directory.path().join("settings.corrupt.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn non_corruption_read_errors_leave_settings_in_place() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("preserved"), "user data").unwrap();

        assert!(AppSettings::load(&path).is_err());

        assert!(path.is_dir());
        assert_eq!(
            fs::read_to_string(path.join("preserved")).unwrap(),
            "user data"
        );
        assert!(!directory.path().join("settings.corrupt.json").exists());
    }

    #[test]
    fn existing_corrupt_backup_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::write(&path, "{new corrupt").unwrap();
        fs::write(
            directory.path().join("settings.corrupt.json"),
            "old corrupt",
        )
        .unwrap();

        assert_eq!(AppSettings::load(&path).unwrap(), AppSettings::default());

        assert_eq!(
            fs::read_to_string(directory.path().join("settings.corrupt.json")).unwrap(),
            "old corrupt"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("settings.corrupt-1.json")).unwrap(),
            "{new corrupt"
        );
    }

    #[test]
    fn volume_level_normalizes_to_the_shipped_range() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::write(&path, r#"{"volumeLevel":2.0}"#).unwrap();

        assert_eq!(AppSettings::load(&path).unwrap().volume_level, 1.0);
    }

    #[test]
    fn failed_save_leaves_the_existing_destination_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("preserved"), "user data").unwrap();

        assert!(AppSettings::default().save(&path).is_err());

        assert_eq!(
            fs::read_to_string(path.join("preserved")).unwrap(),
            "user data"
        );
        assert!(fs::read_dir(directory.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
}
