use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::*;

struct RecordingController {
    settings_path: PathBuf,
    settings_before_shutdown: Vec<u8>,
    releases: Arc<AtomicUsize>,
}

impl EngineController for RecordingController {
    fn shutdown(&mut self) -> Result<(), EngineError> {
        assert_eq!(
            fs::read(&self.settings_path).unwrap(),
            self.settings_before_shutdown
        );
        self.releases.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
fn shutdown_releases_before_persisting_the_paused_session_and_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let settings_path = directory.path().join("settings.json");
    let settings = AppSettings::default();
    settings.save(&settings_path).unwrap();
    let settings_before_shutdown = fs::read(&settings_path).unwrap();
    let tracks = [library_track(
        1,
        PathBuf::from("/paused.flac"),
        "Paused track",
    )];
    let releases = Arc::new(AtomicUsize::new(0));
    let mut playback = Playback::for_test(settings_path.clone(), settings);
    playback.launch_session_pending = false;
    playback.seed_queue(QueueState::from_tracks(&tracks, 0));
    playback.playback_state = PlaybackState::Paused;
    playback.position_ms = 12_345;
    playback.controller = Some(Box::new(RecordingController {
        settings_path: settings_path.clone(),
        settings_before_shutdown,
        releases: Arc::clone(&releases),
    }));

    playback.shutdown();

    assert_eq!(releases.load(Ordering::Relaxed), 1);
    let saved = AppSettings::load(&settings_path).unwrap();
    let session = saved.session.unwrap();
    assert_eq!(session.queue_track_ids, [1]);
    assert_eq!(session.position_ms, 12_345);
    let settings_after_shutdown = fs::read(&settings_path).unwrap();

    playback.position_ms = 54_321;
    playback.shutdown();

    assert_eq!(releases.load(Ordering::Relaxed), 1);
    assert_eq!(fs::read(&settings_path).unwrap(), settings_after_shutdown);
}
