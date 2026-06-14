//! AUHAL playback sink backed by `coreaudio-rs`.
//!
//! The engine feeds native-endian interleaved `f32` frames. Core Audio owns the
//! conversion from that client format to the device's physical format.

use std::{
    mem,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use coreaudio::audio_unit::{
    AudioUnit, Element, Scope, StreamFormat,
    audio_format::LinearPcmFlags,
    macos_helpers::audio_unit_from_device_id,
    render_callback::{self, data},
};
use objc2_core_audio::AudioObjectID;
use rtrb::Consumer;

use crate::{EngineError, PcmFormat};

pub(crate) struct AuhalSink {
    audio_unit: AudioUnit,
    position_frames: Arc<AtomicU64>,
    running: bool,
}

impl AuhalSink {
    pub(crate) fn start(
        device_id: AudioObjectID,
        consumer: Consumer<u8>,
        format: PcmFormat,
    ) -> Result<Self, EngineError> {
        let channels = usize::from(format.channels);
        let bytes_per_frame = channels * mem::size_of::<f32>();
        if channels == 0 {
            return Err(EngineError::UnsupportedFormat(
                "zero-channel playback is not supported".to_string(),
            ));
        }

        let mut audio_unit =
            audio_unit_from_device_id(device_id, false).map_err(audio_unit_error)?;
        audio_unit
            .set_stream_format(
                StreamFormat {
                    sample_rate: f64::from(format.sample_rate),
                    sample_format: coreaudio::audio_unit::SampleFormat::F32,
                    flags: LinearPcmFlags::IS_FLOAT | LinearPcmFlags::IS_PACKED,
                    channels: u32::from(format.channels),
                },
                Scope::Input,
                Element::Output,
            )
            .map_err(audio_unit_error)?;

        let position_frames = Arc::new(AtomicU64::new(0));
        let underrun_bytes = Arc::new(AtomicU64::new(0));
        let callback_position_frames = Arc::clone(&position_frames);
        let callback_underrun_bytes = Arc::clone(&underrun_bytes);
        let mut consumer = consumer;

        type Args = render_callback::Args<data::InterleavedBytes<f32>>;
        audio_unit
            .set_render_callback(move |args: Args| {
                let data = args.data;
                if data.channels != channels {
                    data.buffer.fill(0);
                    callback_underrun_bytes.fetch_add(data.buffer.len() as u64, Ordering::Relaxed);
                    return Ok(());
                }

                let (filled, remainder) = consumer.pop_partial_slice(data.buffer);
                let filled_len = filled.len();
                if !remainder.is_empty() {
                    remainder.fill(0);
                    callback_underrun_bytes.fetch_add(remainder.len() as u64, Ordering::Relaxed);
                }
                callback_position_frames
                    .fetch_add((filled_len / bytes_per_frame) as u64, Ordering::Relaxed);
                Ok(())
            })
            .map_err(audio_unit_error)?;

        audio_unit.start().map_err(audio_unit_error)?;

        Ok(Self {
            audio_unit,
            position_frames,
            running: true,
        })
    }

    pub(crate) fn position_frames(&self) -> u64 {
        self.position_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        if self.running {
            let _ = self.audio_unit.stop();
            self.running = false;
        }
    }
}

impl Drop for AuhalSink {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn audio_unit_error(error: coreaudio::Error) -> EngineError {
    EngineError::UnsupportedFormat(format!("audio unit: {error}"))
}
