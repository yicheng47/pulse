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

const EXCLUSIVE_MODES_VERSION: &str = "version=2";
const LEGACY_EXCLUSIVE_MODES_VERSION: &str = "version=1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredDeviceCapabilities {
    pub max_bits_per_channel: Option<u32>,
    pub max_sample_rate: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoredDevicePreferences {
    pub name: Option<String>,
    pub capabilities: Option<StoredDeviceCapabilities>,
    pub last_seen_unix_seconds: Option<u64>,
    exclusive_mode: Option<bool>,
}

impl StoredDevicePreferences {
    pub fn exclusive_mode_override(&self) -> Option<bool> {
        self.exclusive_mode
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExclusiveModePreferences {
    devices: BTreeMap<String, StoredDevicePreferences>,
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

pub fn load_output_device_uid() -> io::Result<Option<String>> {
    load_output_device_uid_from(&output_device_uid_path()?)
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

pub fn forget_device(
    preferences: &mut ExclusiveModePreferences,
    device_uid: &str,
) -> io::Result<bool> {
    forget_device_from(
        &exclusive_modes_path()?,
        &output_device_uid_path()?,
        preferences,
        device_uid,
    )
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
        preferences.set_override(active_device_uid, !legacy_disabled);
    }
    save_exclusive_mode_preferences_to(path, &preferences)?;
    if legacy_disabled {
        fs::remove_file(legacy_disabled_path)?;
    }
    Ok(preferences)
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

fn save_exclusive_mode_preferences_to(
    path: &Path,
    preferences: &ExclusiveModePreferences,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut contents = String::from(EXCLUSIVE_MODES_VERSION);
    contents.push('\n');
    for (uid, device) in &preferences.devices {
        contents.push_str(match device.exclusive_mode {
            Some(true) => "exclusive",
            Some(false) => "shared",
            None => "auto",
        });
        contents.push('\t');
        contents.push_str(&escape_field(uid));
        contents.push('\t');
        contents.push_str(&escape_field(device.name.as_deref().unwrap_or_default()));
        contents.push('\t');
        match device.capabilities {
            Some(capabilities) => {
                match capabilities.max_bits_per_channel {
                    Some(bits) => contents.push_str(&bits.to_string()),
                    None => contents.push_str("float"),
                }
                contents.push('\t');
                contents.push_str(&capabilities.max_sample_rate.to_string());
            }
            None => contents.push_str("unknown\t"),
        }
        contents.push('\t');
        if let Some(last_seen) = device.last_seen_unix_seconds {
            contents.push_str(&last_seen.to_string());
        }
        contents.push('\n');
    }
    fs::write(path, contents)
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

fn escape_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
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

fn load_output_device_uid_from(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(parse_output_device_uid(&contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn clear_output_device_uid_from(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn forget_device_from(
    preferences_path: &Path,
    output_device_uid_path: &Path,
    preferences: &mut ExclusiveModePreferences,
    device_uid: &str,
) -> io::Result<bool> {
    let mut updated = preferences.clone();
    if !updated.forget(device_uid) {
        return Ok(false);
    }
    if load_output_device_uid_from(output_device_uid_path)?.as_deref() == Some(device_uid) {
        clear_output_device_uid_from(output_device_uid_path)?;
    }
    save_exclusive_mode_preferences_to(preferences_path, &updated)?;
    *preferences = updated;
    Ok(true)
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
    fn offline_override_edit_persists_and_applies_when_the_device_reconnects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let mut preferences = ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            100,
        );
        preferences.set_override("matrix", false);

        save_exclusive_mode_preferences_to(&path, &preferences).unwrap();
        let loaded = parse_exclusive_mode_preferences(&fs::read_to_string(path).unwrap()).unwrap();

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
    fn migrates_the_legacy_disabled_marker_to_the_active_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let legacy_path = directory.path().join("exclusive-mode.disabled");
        fs::write(&legacy_path, []).unwrap();

        let preferences =
            load_exclusive_mode_preferences_from(&path, &legacy_path, true, "airpods").unwrap();

        assert_eq!(
            preferences
                .devices
                .get("airpods")
                .and_then(StoredDevicePreferences::exclusive_mode_override),
            Some(false)
        );
        assert!(!legacy_path.exists());
    }

    #[test]
    fn migrates_the_legacy_enabled_default_to_the_active_device() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let legacy_path = directory.path().join("exclusive-mode.disabled");

        let preferences =
            load_exclusive_mode_preferences_from(&path, &legacy_path, true, "matrix").unwrap();

        assert_eq!(
            preferences
                .devices
                .get("matrix")
                .and_then(StoredDevicePreferences::exclusive_mode_override),
            Some(true)
        );
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
        assert_eq!(fs::read_to_string(path).unwrap(), "version=2\n");
    }

    #[test]
    fn stored_device_details_round_trip_with_escaped_names_and_auto_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("exclusive-modes.tsv");
        let mut preferences = ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix\\uid",
            "Matrix\tmini-i\nSeries",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            1_777_777_777,
        );

        save_exclusive_mode_preferences_to(&path, &preferences).unwrap();
        let loaded = parse_exclusive_mode_preferences(&fs::read_to_string(path).unwrap()).unwrap();

        assert_eq!(loaded, preferences);
        let stored = loaded.devices.get("matrix\\uid").unwrap();
        assert_eq!(stored.name.as_deref(), Some("Matrix\tmini-i\nSeries"));
        assert_eq!(stored.exclusive_mode_override(), None);
    }

    #[test]
    fn parses_version_one_overrides_for_in_place_migration() {
        let preferences =
            parse_exclusive_mode_preferences("version=1\nexclusive\tmatrix\nshared\tairpods\n")
                .unwrap();

        assert!(preferences.effective_mode("matrix", false));
        assert!(!preferences.effective_mode("airpods", true));
        assert_eq!(preferences.devices.len(), 2);
    }

    #[test]
    fn rejects_malformed_version_two_entries() {
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
    fn forgetting_a_saved_device_removes_its_row_and_saved_output_marker() {
        let directory = tempfile::tempdir().unwrap();
        let preferences_path = directory.path().join("exclusive-modes.tsv");
        let output_path = directory.path().join("app-output-device.uid");
        let mut preferences = ExclusiveModePreferences::default();
        preferences.record_sighting(
            "matrix",
            "mini-i Series",
            Some(StoredDeviceCapabilities {
                max_bits_per_channel: Some(24),
                max_sample_rate: 192_000,
            }),
            100,
        );
        preferences.set_override("matrix", false);
        save_exclusive_mode_preferences_to(&preferences_path, &preferences).unwrap();
        fs::write(&output_path, "matrix").unwrap();

        assert!(
            forget_device_from(&preferences_path, &output_path, &mut preferences, "matrix",)
                .unwrap()
        );

        assert!(!preferences.devices.contains_key("matrix"));
        assert!(!output_path.exists());
        assert!(
            !parse_exclusive_mode_preferences(&fs::read_to_string(preferences_path).unwrap())
                .unwrap()
                .devices
                .contains_key("matrix")
        );
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
