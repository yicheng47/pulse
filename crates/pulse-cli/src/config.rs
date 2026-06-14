use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct CliConfig {
    pub default_output: Option<DevicePreference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DevicePreference {
    pub uid: String,
    pub name: String,
}

impl CliConfig {
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents)
                .with_context(|| format!("failed to parse {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let contents = toml::to_string_pretty(self).context("failed to serialize CLI config")?;
        fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))
    }
}

pub fn config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not determine user config directory")?
        .join("pulse")
        .join("cli.toml"))
}

pub fn resolve_output_device(
    explicit_device: Option<pulse_engine::device::DeviceId>,
) -> Result<pulse_engine::device::DeviceId> {
    if let Some(device_id) = explicit_device {
        return Ok(device_id);
    }

    let config = CliConfig::load()?;
    let Some(default_output) = config.default_output else {
        return Ok(pulse_engine::device::default_output_device()?.id);
    };

    resolve_configured_output(&default_output)
}

pub fn configured_output_id() -> Result<Option<pulse_engine::device::DeviceId>> {
    let config = CliConfig::load()?;
    let Some(default_output) = config.default_output else {
        return Ok(None);
    };
    Ok(Some(resolve_configured_output(&default_output)?))
}

pub fn set_default_output(device_id: pulse_engine::device::DeviceId) -> Result<DevicePreference> {
    let device = pulse_engine::device::list_output_devices()?
        .into_iter()
        .find(|device| device.id == device_id)
        .with_context(|| format!("output device {device_id} is not currently available"))?;
    let preference = DevicePreference {
        uid: device.uid,
        name: device.name,
    };
    let config = CliConfig {
        default_output: Some(preference.clone()),
    };
    config.save()?;
    Ok(preference)
}

pub fn clear_default_output() -> Result<()> {
    let config = CliConfig {
        default_output: None,
    };
    config.save()
}

fn resolve_configured_output(
    preference: &DevicePreference,
) -> Result<pulse_engine::device::DeviceId> {
    let devices = pulse_engine::device::list_output_devices()?;
    if let Some(device) = devices.iter().find(|device| device.uid == preference.uid) {
        return Ok(device.id);
    }

    let matching_name: Vec<_> = devices
        .iter()
        .filter(|device| device.name == preference.name)
        .collect();
    match matching_name.as_slice() {
        [device] => Ok(device.id),
        [] => bail!(
            "configured default output device '{}' is not available; pass --device or run `pulse-cli config clear-default-device`",
            preference.name
        ),
        _ => bail!(
            "configured default output device name '{}' is ambiguous; pass --device or run `pulse-cli config set-default-device <id>`",
            preference.name
        ),
    }
}
