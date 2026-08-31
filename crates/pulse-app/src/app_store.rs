use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use gpui::{App, Context, Entity, Global};
use pulse_engine::device;

use crate::backend::{
    AppSettings, ManagedDeviceGroups, Playback, PlaybackAction, PlaybackSnapshot, SessionRoute,
    SessionState, Track, UpdateInfo, Updater,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(16);
const DEVICE_WATCH_POLLS: u32 = 125;

#[derive(Clone)]
pub(crate) struct GlobalAppStore(pub(crate) Entity<AppStore>);

impl Global for GlobalAppStore {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreRevisions {
    pub(crate) settings: u64,
    pub(crate) devices: u64,
    pub(crate) playback: u64,
    pub(crate) queue: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreReactions {
    pub(crate) settings: bool,
    pub(crate) devices: bool,
    pub(crate) playback: bool,
    pub(crate) queue: bool,
    pub(crate) notify: bool,
}

impl StoreRevisions {
    pub(crate) fn reactions_since(self, previous: Self) -> StoreReactions {
        StoreReactions {
            settings: self.settings != previous.settings,
            devices: self.devices != previous.devices,
            playback: self.playback != previous.playback,
            queue: self.queue != previous.queue,
            notify: self != previous,
        }
    }

    fn apply(&mut self, changes: StoreChanges) -> bool {
        if changes.settings {
            self.settings = self.settings.wrapping_add(1);
        }
        if changes.devices {
            self.devices = self.devices.wrapping_add(1);
        }
        if changes.playback {
            self.playback = self.playback.wrapping_add(1);
        }
        if changes.queue {
            self.queue = self.queue.wrapping_add(1);
        }
        changes.any()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StoreChanges {
    settings: bool,
    devices: bool,
    playback: bool,
    queue: bool,
}

impl StoreChanges {
    fn any(self) -> bool {
        self.settings || self.devices || self.playback || self.queue
    }
}

struct RevisionSnapshot {
    settings: AppSettings,
    playback: PlaybackSnapshot,
    device_messages: Vec<(String, bool)>,
}

pub(crate) struct AppStore {
    playback: Playback,
    revision_snapshot: RevisionSnapshot,
    pub(crate) revisions: StoreRevisions,
}

impl AppStore {
    pub(crate) fn new(
        settings_path: PathBuf,
        settings: AppSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.spawn(async move |weak, cx| {
            let mut polls: u32 = 0;
            loop {
                cx.background_executor().timer(EVENT_POLL_INTERVAL).await;
                polls = polls.wrapping_add(1);
                let watch_device = polls.is_multiple_of(DEVICE_WATCH_POLLS);
                if weak
                    .update(cx, |store, cx| store.poll_playback(watch_device, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let playback = Playback::new(settings_path, settings);
        let revision_snapshot = RevisionSnapshot::new(&playback);
        Self {
            playback,
            revision_snapshot,
            revisions: StoreRevisions::default(),
        }
    }

    pub(crate) fn playback_snapshot(&self) -> PlaybackSnapshot {
        self.playback.snapshot()
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        settings_path: PathBuf,
        settings: AppSettings,
        _cx: &mut Context<Self>,
    ) -> Self {
        let playback = Playback::for_test(settings_path, settings);
        let revision_snapshot = RevisionSnapshot::new(&playback);
        Self {
            playback,
            revision_snapshot,
            revisions: StoreRevisions::default(),
        }
    }

    pub(crate) fn launch_session(&self) -> Option<SessionState> {
        self.playback.launch_session()
    }

    pub(crate) fn interface_scale(&self) -> f32 {
        self.playback.settings().interface_scale
    }

    pub(crate) fn set_interface_scale(
        &mut self,
        interface_scale: f32,
        cx: &mut Context<Self>,
    ) -> Option<f32> {
        let changed = self.playback.set_interface_scale(interface_scale);
        let interface_scale = self.interface_scale();
        self.finish_update(cx);
        changed.then_some(interface_scale)
    }

    pub(crate) fn restore_session(
        &mut self,
        session: &SessionState,
        resolved_tracks: Vec<Option<Track>>,
        cx: &mut Context<Self>,
    ) {
        self.playback.restore_session(session, resolved_tracks);
        self.finish_update(cx);
    }

    pub(crate) fn abandon_launch_session_restore(&mut self, cx: &mut Context<Self>) {
        self.playback.abandon_launch_session_restore();
        self.finish_update(cx);
    }

    pub(crate) fn set_session_route(&mut self, route: SessionRoute, cx: &mut Context<Self>) {
        self.playback.set_session_route(route);
        self.finish_update(cx);
    }

    pub(crate) fn managed_device_groups(&self) -> ManagedDeviceGroups {
        self.playback.managed_device_groups()
    }

    pub(crate) fn device_management_messages(&self) -> Vec<(String, bool)> {
        self.playback.device_management_messages()
    }

    pub(crate) fn send_command(&mut self, command: PlaybackAction, cx: &mut Context<Self>) -> bool {
        let result = match command {
            PlaybackAction::ToggleVolumeMute => {
                self.playback.toggle_volume_mute();
                false
            }
            PlaybackAction::SetVolumeLevel(level) => {
                self.playback.set_volume_level(level);
                false
            }
            PlaybackAction::PersistVolume => {
                self.playback.persist_volume();
                false
            }
            PlaybackAction::TogglePlayback => {
                self.playback.toggle_playback();
                false
            }
            PlaybackAction::NextTrack => {
                self.playback.next_track();
                false
            }
            PlaybackAction::PreviousTrack => {
                self.playback.previous_track();
                false
            }
            PlaybackAction::ToggleShuffle => {
                self.playback.toggle_shuffle();
                false
            }
            PlaybackAction::CycleRepeat => {
                self.playback.cycle_repeat();
                false
            }
            PlaybackAction::PlayLibraryTracks {
                tracks,
                start_index,
            } => {
                self.playback.play_library_tracks(&tracks, start_index);
                false
            }
            PlaybackAction::PlayLibraryTracksShuffled(tracks) => {
                self.playback.play_library_tracks_shuffled(&tracks);
                false
            }
            PlaybackAction::SelectLibraryTracks {
                tracks,
                start_index,
            } => {
                self.playback.select_library_tracks(&tracks, start_index);
                false
            }
            PlaybackAction::PlayDroppedPaths(paths) => {
                self.playback.handle_drop(&paths);
                false
            }
            PlaybackAction::JumpToQueueEntry(index) => {
                self.playback.jump_to_queue_entry(index);
                false
            }
            PlaybackAction::RemoveQueueEntry(index) => {
                self.playback.remove_queue_entry(index);
                false
            }
            PlaybackAction::ClearUpcomingQueue => {
                self.playback.clear_upcoming_queue();
                false
            }
            PlaybackAction::RetryPlayback => {
                self.playback.retry_playback();
                false
            }
            PlaybackAction::DismissNotice => {
                self.playback.dismiss_notice();
                false
            }
            PlaybackAction::Seek(position_ms) => {
                self.playback.seek(position_ms);
                false
            }
            PlaybackAction::RefreshOutputDevices => {
                self.playback.refresh_output_devices();
                false
            }
            PlaybackAction::SelectOutputDevice(output_device) => {
                self.playback.select_output_device(output_device);
                false
            }
            PlaybackAction::SetDeviceOutputMode { device_uid, mode } => {
                self.playback.set_device_output_mode(device_uid, mode);
                false
            }
            PlaybackAction::ResetDeviceOutputMode(device_uid) => {
                self.playback.reset_device_output_mode_to_auto(device_uid);
                false
            }
            PlaybackAction::ForgetManagedDevice(device_uid) => {
                self.playback.forget_managed_device(&device_uid)
            }
            PlaybackAction::SetManagedDeviceAsDefault(device_uid) => {
                self.playback.set_managed_device_as_default(&device_uid);
                false
            }
            PlaybackAction::ClearMissingMarks => {
                self.playback.clear_missing_marks();
                false
            }
            PlaybackAction::RemoveMissingMarks(track_ids) => {
                self.playback.remove_missing_marks(&track_ids);
                false
            }
        };
        self.finish_update(cx);
        result
    }

    pub(crate) fn shutdown(&mut self) {
        self.playback.shutdown();
    }

    fn poll_playback(&mut self, watch_device: bool, cx: &mut Context<Self>) {
        let events_changed = self.playback.drain_events();
        let devices_changed = watch_device && self.playback.check_active_device_presence();
        if events_changed || devices_changed {
            self.finish_update(cx);
        }
    }

    fn revision_snapshot(&self) -> RevisionSnapshot {
        RevisionSnapshot::new(&self.playback)
    }

    fn finish_update(&mut self, cx: &mut Context<Self>) {
        let after = self.revision_snapshot();
        let changes = StoreChanges {
            settings: self.revision_snapshot.settings != after.settings,
            devices: devices_changed(&self.revision_snapshot, &after),
            playback: playback_changed(&self.revision_snapshot.playback, &after.playback),
            queue: !Arc::ptr_eq(
                &self.revision_snapshot.playback.queue,
                &after.playback.queue,
            ),
        };
        self.revision_snapshot = after;
        if self.revisions.apply(changes) {
            cx.notify();
        }
    }
}

pub(crate) struct UpdaterBridge {
    updater: Updater,
}

impl UpdaterBridge {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let updater = cx.weak_entity();
        let async_cx = cx.to_async();
        Self {
            updater: Updater::new(move |transition| {
                let updater = updater.clone();
                async_cx
                    .spawn(async move |cx| {
                        let _ = cx.update(|cx| {
                            updater.update(cx, |bridge, cx| {
                                if bridge.updater.apply_transition(transition) {
                                    cx.notify();
                                }
                            })
                        });
                    })
                    .detach();
            }),
        }
    }

    pub(crate) fn is_available(&self) -> bool {
        self.updater.is_available()
    }

    pub(crate) fn start(&self) {
        self.updater.start();
    }

    pub(crate) fn check_for_updates(&self) {
        self.updater.check_for_updates();
    }

    pub(crate) fn available(&self) -> Option<&UpdateInfo> {
        self.updater.available()
    }

    pub(crate) fn automatically_checks_for_updates(&self) -> bool {
        self.updater.automatically_checks_for_updates()
    }

    pub(crate) fn set_automatically_checks_for_updates(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.updater.set_automatically_checks_for_updates(enabled);
        cx.notify();
    }

    pub(crate) fn last_check_at(&self) -> Option<SystemTime> {
        self.updater.last_check_at()
    }
}

impl RevisionSnapshot {
    fn new(playback: &Playback) -> Self {
        Self {
            settings: playback.settings().clone(),
            playback: playback.snapshot(),
            device_messages: playback.device_management_messages(),
        }
    }
}

fn devices_changed(before: &RevisionSnapshot, after: &RevisionSnapshot) -> bool {
    !Arc::ptr_eq(&before.playback.devices, &after.playback.devices)
        || before.settings.saved_output_device_uid != after.settings.saved_output_device_uid
        || before.settings.output_mode_preferences != after.settings.output_mode_preferences
        || before.device_messages != after.device_messages
        || active_device_row(before.playback.active_device.as_ref())
            != active_device_row(after.playback.active_device.as_ref())
        || before.playback.device_capabilities != after.playback.device_capabilities
        || before.playback.automatic_output_mode != after.playback.automatic_output_mode
        || before.playback.output_mode != after.playback.output_mode
        || before.playback.output_mode_automatic != after.playback.output_mode_automatic
        || before.playback.volume_state != after.playback.volume_state
}

fn playback_changed(before: &PlaybackSnapshot, after: &PlaybackSnapshot) -> bool {
    let PlaybackSnapshot {
        playback_state: before_playback_state,
        source_path: before_source_path,
        cover_art_path: before_cover_art_path,
        queue: _before_queue,
        title: before_title,
        secondary: before_secondary,
        format: before_format,
        devices: _before_devices,
        active_device: before_active_device,
        device_capabilities: _before_device_capabilities,
        device_message: _before_device_message,
        automatic_output_mode: _before_automatic_output_mode,
        output_mode: _before_output_mode,
        playback_output_mode: before_playback_output_mode,
        output_mode_automatic: _before_output_mode_automatic,
        bit_perfect_active: before_bit_perfect_active,
        volume_state: before_volume_state,
        volume_level: before_volume_level,
        volume_muted: before_volume_muted,
        position_ms: before_position_ms,
        duration_ms: before_duration_ms,
        dropout_frames: before_dropout_frames,
        error: before_error,
        notice: before_notice,
        missing_track_ids: before_missing_track_ids,
    } = before;
    let PlaybackSnapshot {
        playback_state: after_playback_state,
        source_path: after_source_path,
        cover_art_path: after_cover_art_path,
        queue: _after_queue,
        title: after_title,
        secondary: after_secondary,
        format: after_format,
        devices: _after_devices,
        active_device: after_active_device,
        device_capabilities: _after_device_capabilities,
        device_message: _after_device_message,
        automatic_output_mode: _after_automatic_output_mode,
        output_mode: _after_output_mode,
        playback_output_mode: after_playback_output_mode,
        output_mode_automatic: _after_output_mode_automatic,
        bit_perfect_active: after_bit_perfect_active,
        volume_state: after_volume_state,
        volume_level: after_volume_level,
        volume_muted: after_volume_muted,
        position_ms: after_position_ms,
        duration_ms: after_duration_ms,
        dropout_frames: after_dropout_frames,
        error: after_error,
        notice: after_notice,
        missing_track_ids: after_missing_track_ids,
    } = after;

    before_playback_state != after_playback_state
        || before_source_path != after_source_path
        || before_cover_art_path != after_cover_art_path
        || before_title != after_title
        || before_secondary != after_secondary
        || before_format != after_format
        || active_device_row(before_active_device.as_ref())
            != active_device_row(after_active_device.as_ref())
        || before_playback_output_mode != after_playback_output_mode
        || before_bit_perfect_active != after_bit_perfect_active
        || before_volume_state != after_volume_state
        || before_volume_level != after_volume_level
        || before_volume_muted != after_volume_muted
        || before_position_ms != after_position_ms
        || before_duration_ms != after_duration_ms
        || before_dropout_frames != after_dropout_frames
        || before_error != after_error
        || before_notice != after_notice
        || !Arc::ptr_eq(before_missing_track_ids, after_missing_track_ids)
}

fn active_device_row(device: Option<&device::Device>) -> Option<(device::DeviceId, &str, &str)> {
    device.map(|device| (device.id, device.uid.as_str(), device.name.as_str()))
}

pub(crate) fn global_app_store(cx: &App) -> Entity<AppStore> {
    cx.global::<GlobalAppStore>().0.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reactions_preserve_each_data_dependency() {
        let before = StoreRevisions::default();
        for (after, expected) in [
            (
                StoreRevisions {
                    settings: 1,
                    ..before
                },
                StoreReactions {
                    settings: true,
                    notify: true,
                    ..StoreReactions::default()
                },
            ),
            (
                StoreRevisions {
                    devices: 1,
                    ..before
                },
                StoreReactions {
                    devices: true,
                    notify: true,
                    ..StoreReactions::default()
                },
            ),
            (
                StoreRevisions {
                    playback: 1,
                    ..before
                },
                StoreReactions {
                    playback: true,
                    notify: true,
                    ..StoreReactions::default()
                },
            ),
            (
                StoreRevisions { queue: 1, ..before },
                StoreReactions {
                    queue: true,
                    notify: true,
                    ..StoreReactions::default()
                },
            ),
        ] {
            assert_eq!(after.reactions_since(before), expected);
        }
        assert_eq!(before.reactions_since(before), StoreReactions::default());
    }

    #[test]
    fn each_domain_change_bumps_exactly_its_counter() {
        let cases = [
            StoreChanges {
                settings: true,
                ..StoreChanges::default()
            },
            StoreChanges {
                devices: true,
                ..StoreChanges::default()
            },
            StoreChanges {
                playback: true,
                ..StoreChanges::default()
            },
            StoreChanges {
                queue: true,
                ..StoreChanges::default()
            },
        ];
        for changes in cases {
            let mut revisions = StoreRevisions::default();
            assert!(revisions.apply(changes));
            assert_eq!(
                revisions,
                StoreRevisions {
                    settings: u64::from(changes.settings),
                    devices: u64::from(changes.devices),
                    playback: u64::from(changes.playback),
                    queue: u64::from(changes.queue),
                }
            );
        }
    }

    #[test]
    fn no_op_update_does_not_notify_or_bump() {
        let mut revisions = StoreRevisions::default();
        assert!(!revisions.apply(StoreChanges::default()));
        assert_eq!(revisions, StoreRevisions::default());
    }

    #[test]
    fn coalesced_revisions_cannot_drop_a_domain() {
        let mut after = StoreRevisions::default();
        assert!(after.apply(StoreChanges {
            settings: true,
            devices: true,
            playback: true,
            queue: true,
        }));
        assert_eq!(
            after,
            StoreRevisions {
                settings: 1,
                devices: 1,
                playback: 1,
                queue: 1,
            }
        );
        assert_eq!(
            after.reactions_since(StoreRevisions::default()),
            StoreReactions {
                settings: true,
                devices: true,
                playback: true,
                queue: true,
                notify: true,
            }
        );
    }
}
