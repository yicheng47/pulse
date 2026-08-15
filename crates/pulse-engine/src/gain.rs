use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

const UNITY_GAIN: f32 = 1.0;
const RAMP_DURATION_DIVISOR: u32 = 100;

#[derive(Clone)]
pub(crate) struct GainControl {
    target_gain_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
}

impl Default for GainControl {
    fn default() -> Self {
        Self {
            target_gain_bits: Arc::new(AtomicU32::new(UNITY_GAIN.to_bits())),
            muted: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl GainControl {
    pub(crate) fn set_volume(&self, gain: f32, muted: bool) {
        self.target_gain_bits
            .store(gain.to_bits(), Ordering::Relaxed);
        self.muted.store(muted, Ordering::Relaxed);
    }

    fn target_gain(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.target_gain_bits.load(Ordering::Relaxed))
        }
    }
}

pub(crate) struct GainProcessor {
    control: GainControl,
    channels: usize,
    ramp_frames: u32,
    ramp_frames_remaining: u32,
    applied_gain: f32,
    ramp_target: f32,
}

impl GainProcessor {
    pub(crate) fn new(control: GainControl, sample_rate: u32, channels: usize) -> Self {
        let applied_gain = control.target_gain();
        Self {
            control,
            channels,
            ramp_frames: (sample_rate / RAMP_DURATION_DIVISOR).max(1),
            ramp_frames_remaining: 0,
            applied_gain,
            ramp_target: applied_gain,
        }
    }

    pub(crate) fn process(&mut self, buffer: &mut [u8]) {
        let target_gain = self.control.target_gain();
        if target_gain != self.ramp_target {
            self.ramp_target = target_gain;
            self.ramp_frames_remaining = self.ramp_frames;
        }

        if self.applied_gain == UNITY_GAIN && self.ramp_target == UNITY_GAIN {
            return;
        }

        let bytes_per_frame = self.channels * size_of::<f32>();
        for frame in buffer.chunks_exact_mut(bytes_per_frame) {
            let gain = self.next_gain();
            if gain == UNITY_GAIN {
                continue;
            }
            if gain == 0.0 {
                frame.fill(0);
                continue;
            }
            for sample in frame.chunks_exact_mut(size_of::<f32>()) {
                let value = f32::from_ne_bytes([sample[0], sample[1], sample[2], sample[3]]);
                sample.copy_from_slice(&(value * gain).to_ne_bytes());
            }
        }
    }

    fn next_gain(&mut self) -> f32 {
        if self.ramp_frames_remaining == 0 {
            return self.applied_gain;
        }

        self.applied_gain +=
            (self.ramp_target - self.applied_gain) / self.ramp_frames_remaining as f32;
        self.ramp_frames_remaining -= 1;
        if self.ramp_frames_remaining == 0 {
            self.applied_gain = self.ramp_target;
        }
        self.applied_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_is_sample_identical() {
        let control = GainControl::default();
        let mut processor = GainProcessor::new(control, 48_000, 2);
        let mut buffer = [
            0.25_f32.to_bits(),
            (-0.0_f32).to_bits(),
            f32::from_bits(0x7fc0_1234).to_bits(),
            f32::INFINITY.to_bits(),
        ]
        .into_iter()
        .flat_map(u32::to_ne_bytes)
        .collect::<Vec<_>>();
        let original = buffer.clone();

        processor.process(&mut buffer);

        assert_eq!(buffer, original);
    }

    #[test]
    fn ramp_is_monotonic_and_bounded_within_each_buffer() {
        let control = GainControl::default();
        let mut processor = GainProcessor::new(control.clone(), 1_000, 1);
        control.set_volume(0.2, false);
        let mut first_buffer = unit_buffer(4, 1);

        processor.process(&mut first_buffer);

        let first = samples(&first_buffer);
        assert!(first.windows(2).all(|pair| pair[0] > pair[1]));
        assert!(first.iter().all(|gain| (0.2..=1.0).contains(gain)));

        let mut second_buffer = unit_buffer(6, 1);
        processor.process(&mut second_buffer);
        let second = samples(&second_buffer);
        assert!(second.windows(2).all(|pair| pair[0] > pair[1]));
        assert!(second.iter().all(|gain| (0.2..=first[3]).contains(gain)));
        assert_eq!(second[5], 0.2);

        control.set_volume(0.8, false);
        let mut rising_buffer = unit_buffer(10, 1);
        processor.process(&mut rising_buffer);
        let rising = samples(&rising_buffer);
        assert!(rising.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(rising.iter().all(|gain| (0.2..=0.8).contains(gain)));
        assert_eq!(rising[9], 0.8);
    }

    #[test]
    fn mute_reaches_exact_silence_and_unmute_restores_the_level() {
        let control = GainControl::default();
        control.set_volume(0.25, false);
        let mut processor = GainProcessor::new(control.clone(), 1_000, 2);
        control.set_volume(0.25, true);
        let mut muting_buffer = unit_buffer(10, 2);

        processor.process(&mut muting_buffer);

        assert_eq!(&muting_buffer[18 * size_of::<f32>()..], &[0; 8]);
        let mut silent_buffer = f32::INFINITY.to_bits().to_ne_bytes().repeat(4);
        processor.process(&mut silent_buffer);
        assert!(silent_buffer.iter().all(|byte| *byte == 0));

        control.set_volume(0.25, false);
        let mut unmuting_buffer = unit_buffer(10, 2);
        processor.process(&mut unmuting_buffer);
        assert_eq!(samples(&unmuting_buffer)[19], 0.25);
    }

    #[test]
    fn gain_survives_a_simulated_track_change() {
        let control = GainControl::default();
        control.set_volume(0.4, false);
        let mut first_track = GainProcessor::new(control.clone(), 48_000, 2);
        let mut first_buffer = unit_buffer(1, 2);
        first_track.process(&mut first_buffer);

        let mut second_track = GainProcessor::new(control, 96_000, 2);
        let mut second_buffer = unit_buffer(1, 2);
        second_track.process(&mut second_buffer);

        assert_eq!(first_buffer, second_buffer);
        assert_eq!(samples(&second_buffer), [0.4, 0.4]);
    }

    fn unit_buffer(frames: usize, channels: usize) -> Vec<u8> {
        1.0_f32.to_ne_bytes().repeat(frames * channels)
    }

    fn samples(buffer: &[u8]) -> Vec<f32> {
        buffer
            .chunks_exact(size_of::<f32>())
            .map(|sample| f32::from_ne_bytes([sample[0], sample[1], sample[2], sample[3]]))
            .collect()
    }
}
