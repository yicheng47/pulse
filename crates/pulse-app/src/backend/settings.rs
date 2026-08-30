use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread::{self, JoinHandle},
};

use serde::{Deserialize, Serialize};

use super::{PlaylistId, TrackId, queue::RepeatMode};

pub(crate) const SESSION_STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionAlbumKey {
    pub(crate) artist: String,
    pub(crate) title: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "destination",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum SessionRoute {
    Albums {
        album: Option<SessionAlbumKey>,
    },
    Artists {
        artist: Option<String>,
        album: Option<SessionAlbumKey>,
    },
    Tracks,
    Playlists {
        playlist_id: Option<PlaylistId>,
    },
    Storage,
    Devices,
}

impl Default for SessionRoute {
    fn default() -> Self {
        Self::Albums { album: None }
    }
}

impl SessionRoute {
    fn is_valid(&self) -> bool {
        let valid_text = |text: &str| !text.trim().is_empty();
        let valid_album =
            |album: &SessionAlbumKey| valid_text(&album.artist) && valid_text(&album.title);
        match self {
            Self::Albums { album } => album.as_ref().is_none_or(valid_album),
            Self::Artists { artist, album } => {
                artist.as_deref().is_none_or(valid_text)
                    && album.as_ref().is_none_or(valid_album)
                    && match (artist, album) {
                        (None, None) => true,
                        (Some(_), None) => true,
                        (Some(artist), Some(album)) => artist == &album.artist,
                        (None, Some(_)) => false,
                    }
            }
            Self::Playlists { playlist_id } => playlist_id.is_none_or(|id| id > 0),
            Self::Tracks | Self::Storage | Self::Devices => true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionState {
    pub(crate) version: u32,
    pub(crate) queue_track_ids: Vec<TrackId>,
    pub(crate) queue_original_positions: Vec<usize>,
    pub(crate) current_index: Option<usize>,
    pub(crate) position_ms: u64,
    pub(crate) shuffle_enabled: bool,
    pub(crate) repeat_mode: RepeatMode,
    pub(crate) route: SessionRoute,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            version: SESSION_STATE_VERSION,
            queue_track_ids: Vec::new(),
            queue_original_positions: Vec::new(),
            current_index: None,
            position_ms: 0,
            shuffle_enabled: false,
            repeat_mode: RepeatMode::Off,
            route: SessionRoute::default(),
        }
    }
}

impl SessionState {
    fn is_valid(&self) -> bool {
        let mut original_positions = HashSet::new();
        self.version == SESSION_STATE_VERSION
            && self.queue_track_ids.iter().all(|id| *id > 0)
            && self.queue_original_positions.len() == self.queue_track_ids.len()
            && self
                .queue_original_positions
                .iter()
                .all(|position| original_positions.insert(*position))
            && match (self.queue_track_ids.is_empty(), self.current_index) {
                (true, None) => self.position_ms == 0,
                (false, Some(index)) => index < self.queue_track_ids.len(),
                _ => false,
            }
            && self.route.is_valid()
    }
}

fn deserialize_session<'de, D>(deserializer: D) -> Result<Option<SessionState>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value
        .and_then(|value| serde_json::from_value::<SessionState>(value).ok())
        .filter(SessionState::is_valid))
}

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
    #[serde(
        default,
        deserialize_with = "deserialize_session",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) session: Option<SessionState>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            saved_output_device_uid: None,
            exclusive_mode_preferences: ExclusiveModePreferences::default(),
            legacy_exclusive_mode_disabled: None,
            volume_level: 1.0,
            volume_muted: false,
            session: None,
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
        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.is_valid())
        {
            self.session = None;
        }
    }
}

enum SettingsWrite {
    Save {
        settings: Box<AppSettings>,
        generation: Option<u64>,
        completion: Option<SyncSender<io::Result<()>>>,
    },
    Flush(SyncSender<()>),
    Stop,
}

pub(crate) struct SettingsWriteResult {
    pub(crate) generation: u64,
    pub(crate) settings: AppSettings,
    pub(crate) result: io::Result<()>,
}

pub(crate) struct SettingsWriter {
    write_tx: Sender<SettingsWrite>,
    result_rx: Receiver<SettingsWriteResult>,
    worker: Option<JoinHandle<()>>,
}

impl SettingsWriter {
    pub(crate) fn spawn(path: PathBuf) -> io::Result<Self> {
        let (write_tx, write_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("pulse-settings".to_string())
            .spawn(move || settings_writer_loop(path, write_rx, result_tx))?;
        Ok(Self {
            write_tx,
            result_rx,
            worker: Some(worker),
        })
    }

    pub(crate) fn save(&self, settings: AppSettings) -> io::Result<()> {
        let (completion_tx, completion_rx) = mpsc::sync_channel(1);
        self.send(SettingsWrite::Save {
            settings: Box::new(settings),
            generation: None,
            completion: Some(completion_tx),
        })?;
        completion_rx
            .recv()
            .map_err(|_| settings_writer_stopped())?
    }

    pub(crate) fn save_in_background(
        &self,
        generation: u64,
        settings: AppSettings,
    ) -> io::Result<()> {
        self.send(SettingsWrite::Save {
            settings: Box::new(settings),
            generation: Some(generation),
            completion: None,
        })
    }

    pub(crate) fn flush(&self) -> io::Result<()> {
        let (completion_tx, completion_rx) = mpsc::sync_channel(1);
        self.send(SettingsWrite::Flush(completion_tx))?;
        completion_rx
            .recv()
            .map_err(|_| settings_writer_stopped())?;
        Ok(())
    }

    pub(crate) fn take_results(&self) -> Vec<SettingsWriteResult> {
        self.result_rx.try_iter().collect()
    }

    fn send(&self, write: SettingsWrite) -> io::Result<()> {
        self.write_tx
            .send(write)
            .map_err(|_| settings_writer_stopped())
    }
}

impl Drop for SettingsWriter {
    fn drop(&mut self) {
        let _ = self.write_tx.send(SettingsWrite::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn settings_writer_loop(
    path: PathBuf,
    write_rx: Receiver<SettingsWrite>,
    result_tx: Sender<SettingsWriteResult>,
) {
    while let Ok(write) = write_rx.recv() {
        match write {
            SettingsWrite::Save {
                settings,
                generation,
                completion,
            } => {
                let result = settings.save(&path);
                if let Some(completion) = completion {
                    let _ = completion.send(result);
                } else if let Some(generation) = generation {
                    let _ = result_tx.send(SettingsWriteResult {
                        generation,
                        settings: *settings,
                        result,
                    });
                }
            }
            SettingsWrite::Flush(completion) => {
                let _ = completion.send(());
            }
            SettingsWrite::Stop => break,
        }
    }
}

fn settings_writer_stopped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "settings writer stopped")
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
            session: Some(SessionState {
                version: SESSION_STATE_VERSION,
                queue_track_ids: vec![11, 22, 11],
                queue_original_positions: vec![0, 2, 1],
                current_index: Some(1),
                position_ms: 42_500,
                shuffle_enabled: true,
                repeat_mode: RepeatMode::All,
                route: SessionRoute::Playlists {
                    playlist_id: Some(7),
                },
            }),
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
        assert!(contents.contains("\"queueTrackIds\""));
        assert!(contents.contains("\"queueOriginalPositions\""));
        assert!(contents.contains("\"destination\": \"playlists\""));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn old_settings_without_a_session_load_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        fs::write(
            &path,
            r#"{"savedOutputDeviceUid":"matrix","volumeLevel":0.25,"volumeMuted":true}"#,
        )
        .unwrap();

        let loaded = AppSettings::load(&path).unwrap();

        assert_eq!(loaded.saved_output_device_uid.as_deref(), Some("matrix"));
        assert_eq!(loaded.volume_level, 0.25);
        assert!(loaded.volume_muted);
        assert_eq!(loaded.session, None);
    }

    #[test]
    fn unparseable_partial_or_unknown_session_blobs_start_cold_without_losing_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = settings_path(directory.path());
        for session in [
            r#"{"version":1,"queueTrackIds":"broken"}"#,
            r#"{"version":1,"queueTrackIds":[1]}"#,
            r#"{"version":1,"queueTrackIds":[1,2],"queueOriginalPositions":[0,0],"currentIndex":0,"positionMs":0,"shuffleEnabled":false,"repeatMode":"off","route":{"destination":"albums","album":null}}"#,
            r#"{"version":2,"queueTrackIds":[],"queueOriginalPositions":[],"currentIndex":null,"positionMs":0,"shuffleEnabled":false,"repeatMode":"off","route":{"destination":"albums","album":null}}"#,
        ] {
            fs::write(
                &path,
                format!(
                    r#"{{"savedOutputDeviceUid":"matrix","volumeLevel":0.25,"volumeMuted":true,"session":{session}}}"#
                ),
            )
            .unwrap();

            let loaded = AppSettings::load(&path).unwrap();

            assert_eq!(loaded.saved_output_device_uid.as_deref(), Some("matrix"));
            assert_eq!(loaded.volume_level, 0.25);
            assert!(loaded.volume_muted);
            assert_eq!(loaded.session, None);
            assert!(path.exists());
            assert!(!directory.path().join("settings.corrupt.json").exists());
        }
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
