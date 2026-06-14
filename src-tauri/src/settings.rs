use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct AppConfig {
    pub default_output: Option<DevicePreference>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DevicePreference {
    pub uid: String,
    pub name: String,
}

impl AppConfig {
    pub fn load() -> Result<Self, String> {
        let path = config_path()?;
        match fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents)
                .map_err(|error| format!("failed to parse {}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("failed to read {}: {error}", path.display())),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let contents = toml::to_string_pretty(self)
            .map_err(|error| format!("failed to serialize app config: {error}"))?;
        fs::write(&path, contents)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))
    }
}

pub fn config_path() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|path| path.join("pulse").join("app.toml"))
        .ok_or_else(|| "could not determine user config directory".to_string())
}
