use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::{
    EngineError, Levels, PcmFormat, auhal, device,
    gain::{GainControl, UNITY_GAIN, volume_gain_for_level},
    hal,
};

#[derive(Debug, Clone, Copy)]
struct FloatPacker {
    source_bits_per_sample: u8,
    source_bytes_per_frame: usize,
    output_bytes_per_frame: usize,
    channels: usize,
}

impl FloatPacker {
    fn new(source: PcmFormat) -> Result<Self, EngineError> {
        if !matches!(source.bits_per_sample, 16 | 24 | 32) {
            return Err(EngineError::UnsupportedFormat(format!(
                "{}-bit PCM is not supported by the AUHAL packer",
                source.bits_per_sample
            )));
        }
        let channels = usize::from(source.channels);
        if channels == 0 {
            return Err(EngineError::UnsupportedFormat(
                "zero-channel playback is not supported".to_string(),
            ));
        }

        Ok(Self {
            source_bits_per_sample: source.bits_per_sample,
            source_bytes_per_frame: source.bytes_per_frame(),
            output_bytes_per_frame: channels * std::mem::size_of::<f32>(),
            channels,
        })
    }
}

/// Playback engine for one output device.
pub struct Engine {
    device: device::DeviceId,
    exclusive_mode: bool,
    _hog: Option<hal::HogGuard>,
    hardware_volume: Option<hal::HardwareVolume>,
    hardware_volume_event_pending: bool,
    producer: Option<Producer<u8>>,
    consumer: Option<Consumer<u8>>,
    sink: Option<auhal::AuhalSink>,
    format: Option<PcmFormat>,
    packer: Option<FloatPacker>,
    pack_buffer: Vec<u8>,
    gain_control: GainControl,
}

impl Engine {
    pub fn open(device: device::DeviceId, exclusive_mode: bool) -> Result<Self, EngineError> {
        let hog = exclusive_mode
            .then(|| hal::HogGuard::acquire(device))
            .transpose()?;
        let hardware_volume = hog
            .as_ref()
            .filter(|hog| hog.owns())
            .and_then(|_| hal::hardware_volume_control(device));
        let hardware_volume_event_pending = hardware_volume.is_some();
        Ok(Self {
            device,
            exclusive_mode,
            _hog: hog,
            hardware_volume,
            hardware_volume_event_pending,
            producer: None,
            consumer: None,
            sink: None,
            format: None,
            packer: None,
            pack_buffer: Vec::new(),
            gain_control: GainControl::default(),
        })
    }

    /// Configures exclusive devices for native-rate playback and prepares the
    /// AUHAL client format. Shared playback leaves the device format alone.
    pub fn set_format(&mut self, fmt: PcmFormat) -> Result<(), EngineError> {
        self.pause()?;
        configure_device_format(
            self.exclusive_mode,
            || hal::set_nominal_sample_rate(self.device, fmt).map(|_| ()),
            || hal::set_matching_physical_format(self.device, fmt).map(|_| ()),
        )?;
        let packer = FloatPacker::new(fmt)?;

        self.reset_ring(fmt, packer)?;
        self.sink = None;
        self.format = Some(fmt);
        self.packer = Some(packer);
        Ok(())
    }

    pub fn play(&mut self) -> Result<(), EngineError> {
        if self.sink.is_some() {
            return Ok(());
        }
        self.gain_control.fade_in();
        let format = self.format.ok_or_else(|| {
            EngineError::UnsupportedFormat("engine format is not set".to_string())
        })?;
        let consumer = self.consumer.take().ok_or_else(|| {
            EngineError::UnsupportedFormat(
                "playback sink is stopped; call set_format before playing again".to_string(),
            )
        })?;
        match auhal::AuhalSink::start(self.device, consumer, format, self.gain_control.clone()) {
            Ok(sink) => {
                self.sink = Some(sink);
                Ok(())
            }
            Err(error) => {
                if let Some(packer) = self.packer {
                    self.reset_ring(format, packer)?;
                }
                Err(error)
            }
        }
    }

    pub fn pause(&mut self) -> Result<(), EngineError> {
        if let Some(sink) = self.sink.take() {
            let timeout = sink.transition_wait_timeout();
            stop_after_fade(&self.gain_control, timeout, || sink.stop())?;
        }
        Ok(())
    }

    /// Pushes interleaved PCM (in the format from `set_format`) into the AUHAL
    /// float32 ring buffer. Returns source frames accepted — backpressure, never
    /// blocks.
    pub fn feed(&mut self, pcm: &[u8]) -> usize {
        let Some(packer) = self.packer else {
            return 0;
        };
        let Some(producer) = &mut self.producer else {
            return 0;
        };

        let source_frames = pcm.len() / packer.source_bytes_per_frame;
        let writable_frames = producer.slots() / packer.output_bytes_per_frame;
        let frames = source_frames.min(writable_frames);
        if frames == 0 {
            return 0;
        }

        self.pack_buffer.clear();
        self.pack_buffer
            .reserve(frames * packer.output_bytes_per_frame);
        pack_pcm_as_f32(
            packer.source_bits_per_sample,
            packer.channels,
            &pcm[..frames * packer.source_bytes_per_frame],
            &mut self.pack_buffer,
        );

        let (pushed, _) = producer.push_partial_slice(&self.pack_buffer);
        pushed.len() / packer.output_bytes_per_frame
    }

    /// Playback position in frames since `play`.
    pub fn position(&self) -> u64 {
        self.sink
            .as_ref()
            .map_or(0, auhal::AuhalSink::position_frames)
    }

    pub fn set_volume(&mut self, level: f32, muted: bool) -> Result<(), EngineError> {
        if let Some(hardware_volume) = &mut self.hardware_volume {
            self.gain_control.set_volume(UNITY_GAIN, false);
            hardware_volume.set_volume(level, muted)
        } else {
            self.gain_control
                .set_volume(volume_gain_for_level(level), muted);
            Ok(())
        }
    }

    pub(crate) fn take_hardware_volume(&mut self) -> Option<(f32, bool)> {
        if !self.hardware_volume_event_pending {
            return None;
        }
        self.hardware_volume_event_pending = false;
        self.hardware_volume
            .as_ref()
            .map(|volume| (volume.level, volume.muted))
    }

    /// Latest RMS/peak from the realtime tap.
    pub fn levels(&self) -> Levels {
        Levels::default()
    }

    fn reset_ring(&mut self, format: PcmFormat, packer: FloatPacker) -> Result<(), EngineError> {
        let ring_capacity = usize::try_from(format.sample_rate)
            .ok()
            .and_then(|sample_rate| sample_rate.checked_mul(packer.output_bytes_per_frame))
            .and_then(|bytes_per_second| bytes_per_second.checked_mul(4))
            .ok_or_else(|| {
                EngineError::UnsupportedFormat("ring buffer size overflow".to_string())
            })?;
        let (producer, consumer) = RingBuffer::<u8>::new(ring_capacity);
        self.producer = Some(producer);
        self.consumer = Some(consumer);
        self.pack_buffer.clear();
        Ok(())
    }
}

fn stop_after_fade(
    gain_control: &GainControl,
    timeout: Duration,
    stop: impl FnOnce() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let transition = gain_control.fade_out();
    let _ = gain_control.wait_for_transition(transition, timeout);
    stop()
}

fn configure_device_format(
    exclusive_mode: bool,
    set_nominal_sample_rate: impl FnOnce() -> Result<(), EngineError>,
    set_matching_physical_format: impl FnOnce() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    if exclusive_mode {
        set_nominal_sample_rate()?;
        let _ = set_matching_physical_format();
    }
    Ok(())
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.pause();
    }
}

fn pack_pcm_as_f32(source_bits_per_sample: u8, channels: usize, pcm: &[u8], output: &mut Vec<u8>) {
    let source_bytes_per_sample = usize::from(source_bits_per_sample).div_ceil(8);
    for frame in pcm.chunks_exact(source_bytes_per_sample * channels) {
        for channel in 0..channels {
            let sample_offset = channel * source_bytes_per_sample;
            let sample = &frame[sample_offset..sample_offset + source_bytes_per_sample];
            let packed = match source_bits_per_sample {
                16 => f32::from(i16::from_ne_bytes([sample[0], sample[1]])) / 32768.0,
                24 => read_i24_ne(sample) as f32 / 8_388_608.0,
                32 => {
                    i32::from_ne_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32
                        / 2_147_483_648.0
                }
                _ => unreachable!("unsupported source width for float packing"),
            };
            output.extend_from_slice(&packed.to_ne_bytes());
        }
    }
}

fn read_i24_ne(bytes: &[u8]) -> i32 {
    let sign = if cfg!(target_endian = "little") {
        if bytes[2] & 0x80 != 0 { 0xff } else { 0x00 }
    } else if bytes[0] & 0x80 != 0 {
        0xff
    } else {
        0x00
    };

    if cfg!(target_endian = "little") {
        i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], sign])
    } else {
        i32::from_ne_bytes([sign, bytes[0], bytes[1], bytes[2]])
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use super::*;

    #[test]
    fn shared_mode_does_not_reconfigure_the_device() {
        configure_device_format(
            false,
            || panic!("shared mode must not set the nominal sample rate"),
            || panic!("shared mode must not set the physical format"),
        )
        .unwrap();
    }

    #[test]
    fn exclusive_mode_reconfigures_rate_then_physical_format() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let rate_calls = Rc::clone(&calls);
        let format_calls = Rc::clone(&calls);

        configure_device_format(
            true,
            move || {
                rate_calls.borrow_mut().push("nominal-rate");
                Ok(())
            },
            move || {
                format_calls.borrow_mut().push("physical-format");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["nominal-rate", "physical-format"]);
    }

    #[test]
    fn stop_halts_after_the_bounded_wait_when_the_callback_never_runs() {
        let control = GainControl::default();
        let stopped = Cell::new(false);

        stop_after_fade(&control, Duration::ZERO, || {
            stopped.set(true);
            Ok(())
        })
        .unwrap();

        assert!(stopped.get());
    }

    #[test]
    fn stop_halts_after_timeout_when_the_callback_stalls_mid_ramp() {
        let control = GainControl::default();
        let mut processor = crate::gain::GainProcessor::new(control.clone(), 1_000, 1);
        let transition = control.fade_out();
        let mut partial_ramp = 1.0_f32.to_ne_bytes().repeat(5);
        processor.process(&mut partial_ramp);
        let last_gain = f32::from_ne_bytes(partial_ramp[16..20].try_into().unwrap());
        assert!((last_gain - 0.5).abs() < f32::EPSILON);
        assert!(!control.wait_for_transition(transition, Duration::ZERO));
        let stopped = Cell::new(false);

        stop_after_fade(&control, Duration::ZERO, || {
            stopped.set(true);
            Ok(())
        })
        .unwrap();

        assert!(stopped.get());
    }

    #[test]
    fn pack_pcm_as_f32_maps_i16_samples() {
        let samples = [-32768_i16, -1_i16, 0_i16, 32767_i16];
        let mut input = Vec::new();
        for sample in samples {
            input.extend_from_slice(&sample.to_ne_bytes());
        }

        let mut output = Vec::new();
        pack_pcm_as_f32(16, 2, &input, &mut output);

        let mut expected = Vec::new();
        for sample in samples {
            expected.extend_from_slice(&(f32::from(sample) / 32768.0).to_ne_bytes());
        }
        assert_eq!(output, expected);
    }

    #[test]
    fn pack_pcm_as_f32_maps_i24_samples() {
        let samples = [-0x0080_0000_i32, -1_i32, 0_i32, 0x007f_ffff_i32];
        let mut input = Vec::new();
        for sample in samples {
            input.extend_from_slice(&i24_ne_bytes(sample));
        }

        let mut output = Vec::new();
        pack_pcm_as_f32(24, 2, &input, &mut output);

        let mut expected = Vec::new();
        for sample in samples {
            expected.extend_from_slice(&(sample as f32 / 8_388_608.0).to_ne_bytes());
        }
        assert_eq!(output, expected);
    }

    #[test]
    fn pack_pcm_as_f32_maps_i32_samples() {
        let samples = [-2_147_483_648_i32, -1_i32, 0_i32, 2_147_483_647_i32];
        let mut input = Vec::new();
        for sample in samples {
            input.extend_from_slice(&sample.to_ne_bytes());
        }

        let mut output = Vec::new();
        pack_pcm_as_f32(32, 2, &input, &mut output);

        let mut expected = Vec::new();
        for sample in samples {
            expected.extend_from_slice(&(sample as f32 / 2_147_483_648.0).to_ne_bytes());
        }
        assert_eq!(output, expected);
    }

    fn i24_ne_bytes(sample: i32) -> [u8; 3] {
        let bytes = sample.to_ne_bytes();
        if cfg!(target_endian = "little") {
            [bytes[0], bytes[1], bytes[2]]
        } else {
            [bytes[1], bytes[2], bytes[3]]
        }
    }
}
