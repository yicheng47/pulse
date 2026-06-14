mod settings;

use serde::Serialize;

#[tauri::command]
fn engine_status() -> &'static str {
    "engine linked"
}

#[derive(Debug, Serialize)]
struct OutputDevice {
    id: pulse_engine::device::DeviceId,
    uid: String,
    name: String,
    is_system_default: bool,
    is_app_default: bool,
}

#[derive(Debug, Serialize)]
struct OutputDeviceSettings {
    config_path: String,
    devices: Vec<OutputDevice>,
    system_default_uid: Option<String>,
    app_default: Option<settings::DevicePreference>,
    app_default_available: bool,
}

#[tauri::command]
fn output_device_settings() -> Result<OutputDeviceSettings, String> {
    read_output_device_settings()
}

#[tauri::command]
fn set_default_output_device(uid: String) -> Result<OutputDeviceSettings, String> {
    let device = pulse_engine::device::list_output_devices()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|device| device.uid == uid)
        .ok_or_else(|| "output device is not currently available".to_string())?;

    settings::AppConfig {
        default_output: Some(settings::DevicePreference {
            uid: device.uid,
            name: device.name,
        }),
    }
    .save()?;

    read_output_device_settings()
}

#[tauri::command]
fn clear_default_output_device() -> Result<OutputDeviceSettings, String> {
    settings::AppConfig::default().save()?;
    read_output_device_settings()
}

fn read_output_device_settings() -> Result<OutputDeviceSettings, String> {
    let config = settings::AppConfig::load()?;
    let config_path = settings::config_path()?.display().to_string();
    let system_default = pulse_engine::device::default_output_device().ok();
    let system_default_uid = system_default.as_ref().map(|device| device.uid.clone());
    let app_default = config.default_output;
    let app_default_uid = app_default.as_ref().map(|device| device.uid.clone());
    let devices = pulse_engine::device::list_output_devices().map_err(|error| error.to_string())?;
    let app_default_available = app_default_uid
        .as_ref()
        .is_some_and(|uid| devices.iter().any(|device| device.uid == *uid));

    Ok(OutputDeviceSettings {
        config_path,
        system_default_uid: system_default_uid.clone(),
        app_default,
        app_default_available,
        devices: devices
            .into_iter()
            .map(|device| OutputDevice {
                is_system_default: Some(device.uid.as_str()) == system_default_uid.as_deref(),
                is_app_default: Some(device.uid.as_str()) == app_default_uid.as_deref(),
                id: device.id,
                uid: device.uid,
                name: device.name,
            })
            .collect(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            engine_status,
            output_device_settings,
            set_default_output_device,
            clear_default_output_device
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulse");
}
