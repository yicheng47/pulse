use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "pulse-cli", about = "CLI harness for the Pulse audio engine")]
enum Cmd {
    /// List Core Audio output devices
    Devices,
    /// Decode a file and print its PCM format
    Probe { file: PathBuf },
    /// Play a file bit-perfect (hog + integer mode)
    Play {
        file: PathBuf,
        /// Output device name (default: system default output)
        #[arg(long)]
        device: Option<String>,
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
        Cmd::Play { file, device } => {
            let _ = (file, device);
            todo!("decode → feed → IOProc; validate on the Matrix DAC indicator")
        }
    }
    Ok(())
}
