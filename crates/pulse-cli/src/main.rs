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
            for d in pulse_engine::device::list_output_devices()? {
                println!("{:>4}  {}", d.id, d.name);
            }
        }
        Cmd::Probe { file } => {
            let stream = pulse_engine::decode::open(&file)?;
            println!("{:?}", stream.format);
        }
        Cmd::Play { file, device } => {
            let _ = (file, device);
            todo!("decode → feed → IOProc; validate on the Matrix DAC indicator")
        }
    }
    Ok(())
}
