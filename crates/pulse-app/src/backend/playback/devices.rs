use super::*;

impl Playback {
    pub(super) fn migrate_legacy_exclusive_mode(
        &mut self,
        active_device_uid: &str,
    ) -> io::Result<bool> {
        let Some(disabled) = self.settings.legacy_exclusive_mode_disabled else {
            return Ok(false);
        };
        self.update_settings(|settings| {
            settings.legacy_exclusive_mode_disabled = None;
            settings.output_mode_preferences.set_mode(
                active_device_uid,
                if disabled {
                    StoredOutputMode::Shared
                } else {
                    StoredOutputMode::Exclusive
                },
            );
        })
    }

    pub(super) fn set_output_mode_preferences(
        &mut self,
        preferences: OutputModePreferences,
    ) -> io::Result<bool> {
        self.update_settings(|settings| {
            settings.output_mode_preferences = preferences;
            settings.legacy_exclusive_mode_disabled = None;
        })
    }

    pub(super) fn forget_device_settings(&mut self, device_uid: &str) -> io::Result<bool> {
        if !self
            .settings
            .output_mode_preferences
            .devices()
            .any(|(uid, _)| uid == device_uid)
        {
            return Ok(false);
        }
        self.update_settings(|settings| {
            settings.output_mode_preferences.forget(device_uid);
            if settings.saved_output_device_uid.as_deref() == Some(device_uid) {
                settings.saved_output_device_uid = None;
            }
        })
    }

    pub(super) fn initialize_output_inner(&mut self) {
        self.device_message = None;
        self.device_capability_message = None;
        let devices = match device::list_output_devices() {
            Ok(devices) => devices,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not list output devices: {error}"),
                    is_error: true,
                });
                return;
            }
        };
        let system_default = match device::default_output_device() {
            Ok(device) => device,
            Err(error) => {
                self.devices = devices;
                self.refresh_devices_snapshot();
                self.device_message = Some(DeviceMessage {
                    text: error.to_string(),
                    is_error: true,
                });
                return;
            }
        };
        let preferred_uid = self.settings.saved_output_device_uid.clone();
        let active_device =
            resolve_output_device(&devices, &system_default, preferred_uid.as_deref());
        self.active_device = Some(active_device.clone());
        if self.settings.legacy_exclusive_mode_disabled.is_some()
            && let Err(error) = self.migrate_legacy_exclusive_mode(&active_device.uid)
        {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not load output-mode preferences: {error}"),
                is_error: true,
            });
            self.device_sightings_writable = false;
        }
        self.record_device_sightings_inner(&devices);
        self.devices = devices;
        self.refresh_devices_snapshot();
        let capabilities = device::output_device_capabilities(active_device.id);
        self.automatic_output_mode = automatic_output_mode(&capabilities);
        self.output_mode = self
            .settings
            .output_mode_preferences
            .effective_mode(&active_device.uid, self.automatic_output_mode);
        let engine_kind = engine_kind_for_output_mode(self.output_mode);
        self.playback_output_mode = output_mode_for_engine_kind(engine_kind);
        self.apply_device_capabilities_result(&active_device, capabilities);
        self.install_controller(active_device.id, engine_kind);
    }

    pub(super) fn initialize_output(&mut self) {
        self.initialize_output_inner();
    }

    pub(crate) fn update_device_capabilities(&mut self, output_device: &device::Device) {
        self.apply_device_capabilities_result(
            output_device,
            device::output_device_capabilities(output_device.id),
        );
    }

    pub(crate) fn apply_device_capabilities_result(
        &mut self,
        output_device: &device::Device,
        result: Result<device::OutputDeviceCapabilities, EngineError>,
    ) {
        self.device_capability_message = None;
        match result {
            Ok(capabilities) => {
                self.device_capabilities = Some(capabilities);
            }
            Err(EngineError::NoOutputCapabilities(_)) => {
                self.device_capabilities = None;
                self.device_capability_message = Some(DeviceMessage {
                    text: format!(
                        "{} does not advertise a signed-integer PCM physical format Pulse can use.",
                        output_device.name
                    ),
                    is_error: false,
                });
            }
            Err(error) => {
                self.device_capabilities = None;
                self.device_capability_message = Some(DeviceMessage {
                    text: format!(
                        "Could not query {} capabilities: {error}",
                        output_device.name
                    ),
                    is_error: true,
                });
            }
        }
    }

    pub(crate) fn record_device_sightings(&mut self, devices: &[device::Device]) {
        self.record_device_sightings_inner(devices);
    }

    pub(super) fn record_device_sightings_inner(&mut self, devices: &[device::Device]) {
        if !self.device_sightings_writable {
            return;
        }
        let seen_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut updated = self.settings.output_mode_preferences.clone();
        for output_device in devices {
            let capabilities =
                capabilities_for_sighting(updated.stored_capabilities(&output_device.uid), || {
                    device::output_device_capabilities(output_device.id)
                });
            updated.record_sighting(
                &output_device.uid,
                &output_device.name,
                capabilities,
                seen_at,
            );
        }
        if let Err(error) = self.set_output_mode_preferences(updated) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save output device details: {error}"),
                is_error: true,
            });
        }
    }

    pub(crate) fn displayed_device_message(&self) -> Option<DeviceMessage> {
        match (&self.device_message, &self.device_capability_message) {
            (Some(message), Some(capability)) => Some(DeviceMessage {
                text: format!("{} {}", message.text, capability.text),
                is_error: message.is_error || capability.is_error,
            }),
            (Some(message), None) => Some(message.clone()),
            (None, Some(capability)) => Some(capability.clone()),
            (None, None) => None,
        }
    }

    pub(crate) fn managed_device_groups(&self) -> ManagedDeviceGroups {
        let mut groups = merge_managed_devices(
            &self.devices,
            self.active_device
                .as_ref()
                .map(|device| device.uid.as_str()),
            self.settings.saved_output_device_uid.as_deref(),
            &self.settings.output_mode_preferences,
        );
        if let Some(active) = groups.connected.iter_mut().find(|device| device.active) {
            if let Some(capabilities) = self.device_capabilities {
                active.capabilities = Some(stored_device_capabilities(capabilities));
            }
            active.output_mode = self.output_mode;
            active.automatic = self.output_mode_is_automatic();
            active.bit_perfect_available = active
                .capabilities
                .is_some_and(StoredDeviceCapabilities::supports_bit_perfect);
            active.hardware_volume_available =
                self.volume_state.domain == pulse_engine::VolumeDomain::Device;
        }
        groups
    }

    pub(crate) fn device_management_messages(&self) -> Vec<(String, bool)> {
        let mut messages = Vec::new();
        if let Some(message) = self.displayed_device_message() {
            messages.push((message.text, message.is_error));
        }
        if let Some(error) = &self.error {
            messages.push((error.clone(), true));
        }
        messages
    }

    pub(crate) fn output_mode_is_automatic(&self) -> bool {
        self.active_device
            .as_ref()
            .is_none_or(|device| !self.settings.output_mode_preferences.is_pinned(&device.uid))
    }

    pub(crate) fn set_device_output_mode(
        &mut self,
        device_uid: String,
        mode: StoredOutputMode,
    ) -> bool {
        let mut updated_preferences = self.settings.output_mode_preferences.clone();
        updated_preferences.set_mode(&device_uid, mode);
        if let Err(error) = self.set_output_mode_preferences(updated_preferences) {
            self.retry_after_output_mode_change = false;
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save the output-mode preference: {error}"),
                is_error: true,
            });

            return false;
        }
        self.device_sightings_writable = true;
        self.apply_output_mode_if_active(&device_uid, mode)
    }

    pub(crate) fn switch_to_bit_perfect_and_retry(&mut self, device_uid: String) {
        if self.retry.is_none()
            || !self
                .active_device
                .as_ref()
                .is_some_and(|device| device.uid == device_uid)
        {
            return;
        }
        self.retry_after_output_mode_change =
            self.set_device_output_mode(device_uid, StoredOutputMode::BitPerfect);
    }

    pub(crate) fn reset_device_output_mode_to_auto(&mut self, device_uid: String) {
        let automatic_mode = self.automatic_mode_for_device(&device_uid);
        let mut updated_preferences = self.settings.output_mode_preferences.clone();
        updated_preferences.clear_mode(&device_uid);
        if let Err(error) = self.set_output_mode_preferences(updated_preferences) {
            self.device_message = Some(DeviceMessage {
                text: format!("Could not save the output-mode preference: {error}"),
                is_error: true,
            });

            return;
        }
        self.device_sightings_writable = true;
        self.apply_output_mode_if_active(&device_uid, automatic_mode);
    }

    fn automatic_mode_for_device(&self, device_uid: &str) -> StoredOutputMode {
        if self
            .active_device
            .as_ref()
            .is_some_and(|device| device.uid == device_uid)
        {
            return self.automatic_output_mode;
        }
        automatic_stored_output_mode(
            self.settings
                .output_mode_preferences
                .devices()
                .find(|(uid, _)| *uid == device_uid)
                .and_then(|(_, preferences)| preferences.capabilities),
        )
    }

    pub(crate) fn apply_output_mode_if_active(
        &mut self,
        device_uid: &str,
        mode: StoredOutputMode,
    ) -> bool {
        let Some(active_device) = self
            .active_device
            .as_ref()
            .filter(|device| device.uid == device_uid)
        else {
            return false;
        };
        let device_id = active_device.id;
        self.stop_before_unsafe_dsd_output_change(mode, self.device_capabilities);
        self.output_mode = mode;
        self.send_command(PlaybackCommand::SetOutputDevice {
            device_id,
            kind: engine_kind_for_output_mode(mode),
        })
    }

    pub(crate) fn forget_managed_device(&mut self, device_uid: &str) -> bool {
        if self.devices.iter().any(|device| device.uid == device_uid) {
            return false;
        }
        match self.forget_device_settings(device_uid) {
            Ok(true) => {
                self.device_message = None;

                true
            }
            Ok(false) => false,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not forget the output device: {error}"),
                    is_error: true,
                });

                false
            }
        }
    }

    pub(crate) fn set_managed_device_as_default(&mut self, device_uid: &str) {
        match self.update_saved_output_device_uid(device_uid) {
            Ok(true) => {
                self.device_message = None;
            }
            Ok(false) => {}
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not save the default output device: {error}"),
                    is_error: true,
                });
            }
        }
    }

    pub(crate) fn update_saved_output_device_uid(
        &mut self,
        device_uid: &str,
    ) -> std::io::Result<bool> {
        if !self.devices.iter().any(|device| device.uid == device_uid) {
            return Ok(false);
        }
        let device_uid = device_uid.to_string();
        self.update_settings(|settings| settings.saved_output_device_uid = Some(device_uid))
    }

    pub(crate) fn check_active_device_presence(&mut self) -> bool {
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Loading
        ) {
            return false;
        }
        let Ok(devices) = device::list_output_devices() else {
            return false;
        };
        // Sightings persist stable uid/name metadata; the live device list also compares Core
        // Audio ids because the same uid can be rebound to a new id.
        let sightings_changed = self.devices.len() != devices.len()
            || self
                .devices
                .iter()
                .zip(&devices)
                .any(|(known, current)| known.uid != current.uid || known.name != current.name);
        if sightings_changed {
            self.record_device_sightings(&devices);
        }
        let devices_changed = self.devices.len() != devices.len()
            || self.devices.iter().zip(&devices).any(|(known, current)| {
                known.id != current.id || known.uid != current.uid || known.name != current.name
            });
        let device_lost = self.note_device_loss(devices);
        if device_lost {
            self.send_command(PlaybackCommand::Stop);
        }
        devices_changed || device_lost
    }

    /// Returns true when the active output device is no longer attached; the
    /// engine has no mid-playback device-loss signal, so the row watches for
    /// it while playing.
    pub(crate) fn note_device_loss(&mut self, devices: Vec<device::Device>) -> bool {
        let Some(active) = self.active_device.clone() else {
            self.devices = devices;
            self.refresh_devices_snapshot();
            return false;
        };
        let attached = devices.iter().any(|device| device.uid == active.uid);
        self.devices = devices;
        self.refresh_devices_snapshot();
        if attached {
            return false;
        }
        self.retry = self
            .current_play
            .as_ref()
            .map(|attempt| attempt.target.clone());
        self.notice = Some(PlaybackNotice::DeviceFailure {
            text: format!(
                "{} was disconnected. Reconnect it or choose another output, then try again.",
                active.name
            ),
        });
        true
    }

    pub(crate) fn refresh_output_devices(&mut self) {
        let devices = match device::list_output_devices() {
            Ok(devices) => devices,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!("Could not refresh output devices: {error}"),
                    is_error: true,
                });
                return;
            }
        };
        self.record_device_sightings(&devices);

        let Some(active_device) = self.active_device.clone() else {
            self.initialize_output();
            return;
        };

        if let Some(current_device) = devices
            .iter()
            .find(|device| device.uid == active_device.uid)
            .cloned()
        {
            self.devices = devices;
            self.refresh_devices_snapshot();
            if current_device.id != active_device.id {
                self.request_output_device_change(current_device, false, None);
            } else {
                self.active_device = Some(current_device.clone());
                self.update_device_capabilities(&current_device);
            }
            return;
        }

        self.devices = devices;
        self.refresh_devices_snapshot();
        let system_default = match device::default_output_device() {
            Ok(device) => device,
            Err(error) => {
                self.device_message = Some(DeviceMessage {
                    text: format!(
                        "{} is no longer available, and no system default could be resolved: {error}",
                        active_device.name
                    ),
                    is_error: true,
                });
                return;
            }
        };
        let message = DeviceMessage {
            text: format!(
                "{} is no longer available. Using system default: {}.",
                active_device.name, system_default.name
            ),
            is_error: false,
        };
        self.request_output_device_change(system_default, false, Some(message));
    }

    pub(crate) fn select_output_device(&mut self, output_device: device::Device) {
        self.request_output_device_change(output_device, true, None);
    }

    pub(crate) fn request_output_device_change(
        &mut self,
        output_device: device::Device,
        persist: bool,
        success_message: Option<DeviceMessage>,
    ) {
        // The popover stays open on selection so the current-device card
        // (capability line, mode control) reflects the switch in place.
        if persist {
            self.notice = None;
            self.retry = None;
        }
        self.error = None;
        self.device_message = None;
        self.device_capability_message = None;
        let capabilities = device::output_device_capabilities(output_device.id);
        let automatic_mode = automatic_output_mode(&capabilities);
        let output_mode = self
            .settings
            .output_mode_preferences
            .effective_mode(&output_device.uid, automatic_mode);
        self.stop_before_unsafe_dsd_output_change(output_mode, capabilities.as_ref().ok().copied());
        self.pending_device_change = Some(PendingDeviceChange {
            device: output_device.clone(),
            persist,
            success_message,
            capabilities,
            automatic_mode,
            output_mode,
        });

        if self.command_tx.is_none() {
            let kind = engine_kind_for_output_mode(output_mode);
            self.install_controller(output_device.id, kind);
            self.complete_output_device_change(output_device.id, kind);
            self.sync_next_source();

            return;
        }
        if !self.send_command(PlaybackCommand::SetOutputDevice {
            device_id: output_device.id,
            kind: engine_kind_for_output_mode(output_mode),
        }) {
            self.pending_device_change = None;
            self.device_message = Some(DeviceMessage {
                text: "Playback engine disconnected while changing output devices.".to_string(),
                is_error: true,
            });
        }
    }
}
