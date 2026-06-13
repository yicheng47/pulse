use std::{path::PathBuf, thread, time::Duration};

use anyhow::Result;
use clap::Parser;

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
    /// Play a file bit-perfect (hog + integer mode)
    Play {
        file: PathBuf,
        /// Core Audio output device ID (default: system default output)
        #[arg(long)]
        device: Option<pulse_engine::device::DeviceId>,
    },
}

fn main() -> Result<()> {
    match Cmd::parse() {
        Cmd::Devices => {
            let default = pulse_engine::device::default_output_device()
                .ok()
                .map(|device| device.id);
            for device in pulse_engine::device::list_output_devices()? {
                let marker = if Some(device.id) == default { "*" } else { " " };
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
            let device_id = match device {
                Some(device_id) => device_id,
                None => pulse_engine::device::default_output_device()?.id,
            };
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
            let stream = pulse_engine::decode::open(&file)?;
            let device_id = match device {
                Some(device_id) => device_id,
                None => pulse_engine::device::default_output_device()?.id,
            };
            let mut engine = pulse_engine::Engine::open(device_id)?;
            engine.set_format(stream.format)?;
            engine.play()?;

            let bytes_per_frame = stream.format.bytes_per_frame();
            let mut fed_frames = 0_u64;
            pulse_engine::decode::stream_pcm(&file, stream.format, |mut pcm| {
                while !pcm.is_empty() {
                    let accepted_frames = engine.feed(pcm);
                    if accepted_frames == 0 {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }

                    let accepted_bytes = accepted_frames * bytes_per_frame;
                    fed_frames += accepted_frames as u64;
                    pcm = &pcm[accepted_bytes..];
                }
                Ok(())
            })?;

            while engine.position() < fed_frames {
                thread::sleep(Duration::from_millis(10));
            }
            engine.pause()?;
        }
    }
    Ok(())
}
