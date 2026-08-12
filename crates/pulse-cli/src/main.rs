use std::{path::PathBuf, sync::mpsc::Receiver, thread, time::Duration};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pulse_engine::{PlaybackCommand, PlaybackController, PlaybackEvent, PlaybackState};

mod config;

#[derive(Parser)]
#[command(name = "pulse-cli", about = "CLI harness for the Pulse audio engine")]
enum Cmd {
    /// List Core Audio output devices
    Devices,
    /// Decode a file and print its PCM format
    Probe { file: PathBuf },
    /// Validate hog mode and physical format switching for a file
    ValidateFormat {
        file: PathBuf,
        /// Core Audio output device ID (default: system default output)
        #[arg(long)]
        device: Option<pulse_engine::device::DeviceId>,
    },
    /// Play a file through the AUHAL Core Audio backend
    Play {
        file: PathBuf,
        /// Core Audio output device ID (default: system default output)
        #[arg(long)]
        device: Option<pulse_engine::device::DeviceId>,
    },
    /// Play, pause for two seconds, then resume a file
    SmokePause {
        file: PathBuf,
        /// Core Audio output device ID (default: system default output)
        #[arg(long)]
        device: Option<pulse_engine::device::DeviceId>,
    },
    /// Play a file, then seek to a timestamp
    SmokeSeek {
        file: PathBuf,
        /// Seek target in seconds, for example 90 or 90s
        #[arg(long, value_name = "SECS", value_parser = parse_position_ms)]
        to: u64,
        /// Core Audio output device ID (default: system default output)
        #[arg(long)]
        device: Option<pulse_engine::device::DeviceId>,
    },
    /// Read or update pulse-cli config
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show the active CLI config
    Show,
    /// Remember an output device as the pulse-cli default
    SetDefaultDevice {
        /// Core Audio output device ID from `pulse-cli devices`
        device: pulse_engine::device::DeviceId,
    },
    /// Remove the configured default output device
    ClearDefaultDevice,
}

fn main() -> Result<()> {
    match Cmd::parse() {
        Cmd::Devices => {
            let system_default = pulse_engine::device::default_output_device()
                .ok()
                .map(|device| device.id);
            let configured_default = config::configured_output_id().ok().flatten();
            for device in pulse_engine::device::list_output_devices()? {
                let marker = if Some(device.id) == configured_default {
                    ">"
                } else if Some(device.id) == system_default {
                    "*"
                } else {
                    " "
                };
                println!("{marker} {:>4}  {}", device.id, device.name);
            }
        }
        Cmd::Probe { file } => {
            let stream = pulse_engine::decode::open(&file)?;
            println!("file: {}", file.display());
            println!("codec: {}", stream.codec);
            println!("sample rate: {} Hz", stream.format.sample_rate);
            println!("bit depth: {} bit", stream.format.bits_per_sample);
            println!("channels: {}", stream.format.channels);
            if let Some(frames) = stream.frames {
                println!("frames: {frames}");
            }
        }
        Cmd::ValidateFormat { file, device } => {
            let stream = pulse_engine::decode::open(&file)?;
            let device_id = config::resolve_output_device(device)?;
            let validation =
                pulse_engine::device::validate_output_format(device_id, stream.format)?;

            println!("file: {}", file.display());
            println!(
                "requested: {} Hz / {} bit / {} channels",
                validation.requested.sample_rate,
                validation.requested.bits_per_sample,
                validation.requested.channels
            );
            println!(
                "device: {} ({})",
                validation.device.name, validation.device.id
            );
            println!(
                "nominal sample rate: {} Hz",
                validation.nominal_sample_rate as u32
            );
            println!("stream: {}", validation.physical_format.stream_id);
            println!(
                "physical format: {} Hz / {} bit / {} channels",
                validation.physical_format.sample_rate as u32,
                validation.physical_format.bits_per_channel,
                validation.physical_format.channels_per_frame
            );
            println!(
                "layout: {} bytes/packet, {} frames/packet, {} bytes/frame, flags 0x{:x}",
                validation.physical_format.bytes_per_packet,
                validation.physical_format.frames_per_packet,
                validation.physical_format.bytes_per_frame,
                validation.physical_format.format_flags
            );
        }
        Cmd::Play { file, device } => {
            let device_id = config::resolve_output_device(device)?;
            let (controller, events) = start_playback(device_id, file)?;
            wait_for_completion(&events)?;
            drop(controller);
        }
        Cmd::SmokePause { file, device } => {
            let device_id = config::resolve_output_device(device)?;
            let (controller, events) = start_playback(device_id, file)?;
            wait_for_state(&events, PlaybackState::Playing)?;
            thread::sleep(Duration::from_secs(2));
            controller
                .command_sender()
                .send(PlaybackCommand::Pause)
                .context("playback controller stopped")?;
            wait_for_state(&events, PlaybackState::Paused)?;
            thread::sleep(Duration::from_secs(2));
            controller
                .command_sender()
                .send(PlaybackCommand::Resume)
                .context("playback controller stopped")?;
            wait_for_state(&events, PlaybackState::Playing)?;
            wait_for_completion(&events)?;
        }
        Cmd::SmokeSeek { file, to, device } => {
            let device_id = config::resolve_output_device(device)?;
            let (controller, events) = start_playback(device_id, file)?;
            wait_for_state(&events, PlaybackState::Playing)?;
            controller
                .command_sender()
                .send(PlaybackCommand::Seek { position_ms: to })
                .context("playback controller stopped")?;
            wait_for_state(&events, PlaybackState::Playing)?;
            wait_for_completion(&events)?;
        }
        Cmd::Config { command } => match command {
            ConfigCmd::Show => {
                let path = config::config_path()?;
                let cli_config = config::CliConfig::load()?;
                println!("config: {}", path.display());
                match cli_config.default_output {
                    Some(default_output) => {
                        println!(
                            "default output device: {} (uid: {})",
                            default_output.name, default_output.uid
                        );
                    }
                    None => println!("default output device: <system default>"),
                }
            }
            ConfigCmd::SetDefaultDevice { device } => {
                let preference = config::set_default_output(device)?;
                println!(
                    "default output device: {} (uid: {})",
                    preference.name, preference.uid
                );
            }
            ConfigCmd::ClearDefaultDevice => {
                config::clear_default_output()?;
                println!("default output device: <system default>");
            }
        },
    }
    Ok(())
}

fn start_playback(
    device_id: pulse_engine::device::DeviceId,
    file: PathBuf,
) -> Result<(PlaybackController, Receiver<PlaybackEvent>)> {
    let controller = PlaybackController::spawn(device_id, true);
    let events = controller.subscribe();
    controller
        .command_sender()
        .send(PlaybackCommand::PlayFile { path: file })
        .context("playback controller stopped")?;
    Ok((controller, events))
}

fn wait_for_state(events: &Receiver<PlaybackEvent>, expected: PlaybackState) -> Result<()> {
    loop {
        match events.recv().context("playback controller stopped")? {
            PlaybackEvent::StateChanged(state) if state == expected => return Ok(()),
            PlaybackEvent::Ended { .. } => bail!("playback ended before reaching {expected:?}"),
            PlaybackEvent::CommandRejected { command, state } => {
                bail!("{command} rejected while {state:?}")
            }
            PlaybackEvent::Error { message, .. } => bail!(message),
            _ => {}
        }
    }
}

fn wait_for_completion(events: &Receiver<PlaybackEvent>) -> Result<()> {
    loop {
        match events.recv().context("playback controller stopped")? {
            PlaybackEvent::Ended { .. } => return Ok(()),
            PlaybackEvent::CommandRejected { command, state } => {
                bail!("{command} rejected while {state:?}")
            }
            PlaybackEvent::Error { message, .. } => bail!(message),
            _ => {}
        }
    }
}

fn parse_position_ms(value: &str) -> Result<u64, String> {
    let seconds = value
        .strip_suffix('s')
        .unwrap_or(value)
        .parse::<f64>()
        .map_err(|_| format!("invalid seconds value '{value}'"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("seconds must be a finite non-negative number".to_string());
    }
    Ok((seconds * 1_000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn parses_seek_seconds_with_optional_suffix() {
        assert_eq!(parse_position_ms("90"), Ok(90_000));
        assert_eq!(parse_position_ms("1.5s"), Ok(1_500));
        assert!(parse_position_ms("-1").is_err());
    }

    #[test]
    fn wait_helpers_surface_command_rejections() {
        let rejection = PlaybackEvent::CommandRejected {
            command: "Pause",
            state: PlaybackState::Idle,
        };

        let (state_tx, state_rx) = mpsc::channel();
        state_tx.send(rejection.clone()).unwrap();
        assert_eq!(
            wait_for_state(&state_rx, PlaybackState::Paused)
                .unwrap_err()
                .to_string(),
            "Pause rejected while Idle"
        );

        let (completion_tx, completion_rx) = mpsc::channel();
        completion_tx.send(rejection).unwrap();
        assert_eq!(
            wait_for_completion(&completion_rx).unwrap_err().to_string(),
            "Pause rejected while Idle"
        );
    }
}
