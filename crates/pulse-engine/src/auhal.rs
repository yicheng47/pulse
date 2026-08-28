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
    time::Duration,
};

use coreaudio::audio_unit::{
    AudioUnit, Element, Scope, StreamFormat,
    audio_format::LinearPcmFlags,
    macos_helpers::audio_unit_from_device_id,
    render_callback::{self, data},
};
use objc2_core_audio::{AudioObjectID, kAudioDevicePropertyBufferFrameSize};
use rtrb::Consumer;

use crate::{
    EngineError, PcmFormat,
    gain::{GainControl, GainProcessor, RAMP_DURATION},
};

const DEFAULT_BUFFER_FRAMES: u32 = 512;

pub(crate) struct AuhalSink {
    audio_unit: AudioUnit,
    position_frames: Arc<AtomicU64>,
    transition_wait_timeout: Duration,
    running: bool,
}

impl AuhalSink {
    pub(crate) fn start(
        device_id: AudioObjectID,
        consumer: Consumer<u8>,
        format: PcmFormat,
        gain_control: GainControl,
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
        let buffer_frames = audio_unit
            .get_property(
                kAudioDevicePropertyBufferFrameSize,
                Scope::Global,
                Element::Output,
            )
            .unwrap_or(DEFAULT_BUFFER_FRAMES);
        let transition_wait_timeout = transition_wait_timeout(buffer_frames, format.sample_rate);

        let position_frames = Arc::new(AtomicU64::new(0));
        let underrun_bytes = Arc::new(AtomicU64::new(0));
        let callback_position_frames = Arc::clone(&position_frames);
        let callback_underrun_bytes = Arc::clone(&underrun_bytes);
        let mut consumer = consumer;
        let mut gain_processor = GainProcessor::new(gain_control, format.sample_rate, channels);

        type Args = render_callback::Args<data::InterleavedBytes<f32>>;
        audio_unit
            .set_render_callback(move |args: Args| {
                let data = args.data;
                if data.channels != channels {
                    data.buffer.fill(0);
                    gain_processor.process(data.buffer);
                    callback_underrun_bytes.fetch_add(data.buffer.len() as u64, Ordering::Relaxed);
                    return Ok(());
                }

                let (filled, remainder) = consumer.pop_partial_slice(data.buffer);
                let filled_len = filled.len();
                if !remainder.is_empty() {
                    remainder.fill(0);
                    callback_underrun_bytes.fetch_add(remainder.len() as u64, Ordering::Relaxed);
                }
                gain_processor.process(data.buffer);
                callback_position_frames
                    .fetch_add((filled_len / bytes_per_frame) as u64, Ordering::Relaxed);
                Ok(())
            })
            .map_err(audio_unit_error)?;

        audio_unit.start().map_err(audio_unit_error)?;

        Ok(Self {
            audio_unit,
            position_frames,
            transition_wait_timeout,
            running: true,
        })
    }

    pub(crate) fn position_frames(&self) -> u64 {
        self.position_frames.load(Ordering::Relaxed)
    }

    pub(crate) fn transition_wait_timeout(&self) -> Duration {
        self.transition_wait_timeout
    }

    pub(crate) fn stop(mut self) -> Result<(), EngineError> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> Result<(), EngineError> {
        if self.running {
            self.running = false;
            self.audio_unit.stop().map_err(audio_unit_error)?;
        }
        Ok(())
    }
}

fn transition_wait_timeout(buffer_frames: u32, sample_rate: u32) -> Duration {
    let three_buffers =
        Duration::from_secs_f64(f64::from(buffer_frames) * 3.0 / f64::from(sample_rate));
    RAMP_DURATION.saturating_add(three_buffers)
}

impl Drop for AuhalSink {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

fn audio_unit_error(error: coreaudio::Error) -> EngineError {
    EngineError::AudioUnit(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_wait_covers_ramp_pickup_and_settled_silence() {
        assert_eq!(
            transition_wait_timeout(128, 48_000),
            Duration::from_millis(18)
        );
        assert_eq!(
            transition_wait_timeout(512, 48_000),
            Duration::from_millis(42)
        );
    }
}
