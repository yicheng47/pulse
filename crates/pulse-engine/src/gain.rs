use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::{thread, time::Duration};

pub(crate) const UNITY_GAIN: f32 = 1.0;
const MIN_AUDIBLE_GAIN: f32 = 0.001;
const RAMP_DURATION_MS: u64 = 10;
pub(crate) const RAMP_DURATION: Duration = Duration::from_millis(RAMP_DURATION_MS);

pub fn volume_gain_for_level(level: f32) -> f32 {
    let level = level.clamp(0.0, 1.0);
    if level == 0.0 {
        return 0.0;
    }
    (level * level * level).max(MIN_AUDIBLE_GAIN)
}

#[derive(Clone)]
pub(crate) struct GainControl {
    target_gain_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    transition_target_bits: Arc<AtomicU32>,
    transition_generation: Arc<AtomicU32>,
    completed_transition_generation: Arc<AtomicU32>,
}

impl Default for GainControl {
    fn default() -> Self {
        Self {
            target_gain_bits: Arc::new(AtomicU32::new(UNITY_GAIN.to_bits())),
            muted: Arc::new(AtomicBool::new(false)),
            transition_target_bits: Arc::new(AtomicU32::new(UNITY_GAIN.to_bits())),
            transition_generation: Arc::new(AtomicU32::new(0)),
            completed_transition_generation: Arc::new(AtomicU32::new(0)),
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

    pub(crate) fn fade_out(&self) -> u32 {
        self.request_transition(0.0)
    }

    pub(crate) fn fade_in(&self) {
        self.request_transition(UNITY_GAIN);
    }

    pub(crate) fn wait_for_transition(&self, generation: u32, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.completed_transition_generation.load(Ordering::Acquire) == generation {
                return true;
            }

            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            thread::sleep(remaining.min(Duration::from_millis(1)));
        }
    }

    fn request_transition(&self, target: f32) -> u32 {
        self.transition_target_bits
            .store(target.to_bits(), Ordering::Relaxed);
        self.transition_generation
            .fetch_add(1, Ordering::Release)
            .wrapping_add(1)
    }

    fn transition(&self) -> (u32, f32) {
        let generation = self.transition_generation.load(Ordering::Acquire);
        let target = f32::from_bits(self.transition_target_bits.load(Ordering::Relaxed));
        (generation, target)
    }

    fn completed_transition_generation(&self) -> u32 {
        self.completed_transition_generation.load(Ordering::Acquire)
    }

    fn complete_transition(&self, generation: u32) {
        self.completed_transition_generation
            .store(generation, Ordering::Release);
    }
}

pub(crate) struct GainProcessor {
    control: GainControl,
    channels: usize,
    ramp_frames: u32,
    ramp_frames_remaining: u32,
    applied_gain: f32,
    ramp_target: f32,
    transition_generation: u32,
    transition_frames_remaining: u32,
    applied_transition_gain: f32,
    transition_target: f32,
    emit_transition_start: bool,
    transition_pending_completion: bool,
}

impl GainProcessor {
    pub(crate) fn new(control: GainControl, sample_rate: u32, channels: usize) -> Self {
        let applied_gain = control.target_gain();
        let ramp_frames = (u64::from(sample_rate) * RAMP_DURATION_MS / 1_000).max(1) as u32;
        let (transition_generation, transition_target) = control.transition();
        let transition_pending_completion =
            transition_generation != control.completed_transition_generation();
        let applied_transition_gain =
            if transition_pending_completion && transition_target == UNITY_GAIN {
                0.0
            } else {
                transition_target
            };
        Self {
            control,
            channels,
            ramp_frames,
            ramp_frames_remaining: 0,
            applied_gain,
            ramp_target: applied_gain,
            transition_generation,
            transition_frames_remaining: if applied_transition_gain != transition_target {
                ramp_frames
            } else {
                0
            },
            applied_transition_gain,
            transition_target,
            emit_transition_start: applied_transition_gain < transition_target,
            transition_pending_completion,
        }
    }

    pub(crate) fn process(&mut self, buffer: &mut [u8]) {
        let target_gain = self.control.target_gain();
        if target_gain != self.ramp_target {
            self.ramp_target = target_gain;
            self.ramp_frames_remaining = self.ramp_frames;
        }

        let (transition_generation, transition_target) = self.control.transition();
        if transition_generation != self.transition_generation {
            self.transition_generation = transition_generation;
            self.transition_target = transition_target;
            self.emit_transition_start = self.applied_transition_gain < self.transition_target;
            self.transition_pending_completion = true;
            self.transition_frames_remaining =
                if self.applied_transition_gain == self.transition_target {
                    0
                } else {
                    self.ramp_frames
                };
        }
        let transition_settled_before_buffer = self.transition_pending_completion
            && !self.emit_transition_start
            && self.transition_frames_remaining == 0;

        if self.applied_gain == UNITY_GAIN
            && self.ramp_target == UNITY_GAIN
            && self.applied_transition_gain == UNITY_GAIN
            && self.transition_target == UNITY_GAIN
            && !self.transition_pending_completion
        {
            return;
        }

        let bytes_per_frame = self.channels * size_of::<f32>();
        for frame in buffer.chunks_exact_mut(bytes_per_frame) {
            let gain = self.next_gain() * self.next_transition_gain();
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

        if transition_settled_before_buffer && buffer.len() >= bytes_per_frame {
            self.control.complete_transition(self.transition_generation);
            self.transition_pending_completion = false;
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

    fn next_transition_gain(&mut self) -> f32 {
        if self.emit_transition_start {
            self.emit_transition_start = false;
            return self.applied_transition_gain;
        }
        if self.transition_frames_remaining == 0 {
            return self.applied_transition_gain;
        }

        self.applied_transition_gain += (self.transition_target - self.applied_transition_gain)
            / self.transition_frames_remaining as f32;
        self.transition_frames_remaining -= 1;
        if self.transition_frames_remaining == 0 {
            self.applied_transition_gain = self.transition_target;
        }
        self.applied_transition_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_volume_level_to_a_perceptual_gain_curve() {
        assert_eq!(volume_gain_for_level(0.0), 0.0);
        assert_eq!(volume_gain_for_level(0.05), MIN_AUDIBLE_GAIN);
        assert_eq!(volume_gain_for_level(0.5), 0.125);
        assert_eq!(volume_gain_for_level(1.0), 1.0);
        assert!(volume_gain_for_level(0.25) < volume_gain_for_level(0.5));
        assert!(volume_gain_for_level(0.5) < volume_gain_for_level(0.75));
    }

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

    #[test]
    fn stop_transition_is_monotonic_and_reaches_silence_before_completion() {
        let control = GainControl::default();
        let mut processor = GainProcessor::new(control.clone(), 1_000, 1);
        let generation = control.fade_out();
        let mut buffer = unit_buffer(10, 1);

        processor.process(&mut buffer);

        let output = samples(&buffer);
        assert!(output.windows(2).all(|pair| pair[0] > pair[1]));
        assert_eq!(output[9], 0.0);
        assert!(!control.wait_for_transition(generation, Duration::ZERO));

        let mut settled_silence = unit_buffer(10, 1);
        processor.process(&mut settled_silence);

        assert!(settled_silence.iter().all(|byte| *byte == 0));
        assert!(control.wait_for_transition(generation, Duration::ZERO));
    }

    #[test]
    fn restart_transition_ramps_from_silence_to_the_user_target() {
        let control = GainControl::default();
        control.set_volume(0.4, false);
        control.fade_in();
        let mut processor = GainProcessor::new(control, 1_000, 1);
        let mut buffer = unit_buffer(11, 1);

        processor.process(&mut buffer);

        let output = samples(&buffer);
        assert!(output.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(output[0], 0.0);
        assert_eq!(output[10], 0.4);
    }

    #[test]
    fn transport_transitions_do_not_change_user_gain_or_mute() {
        let control = GainControl::default();
        control.set_volume(0.25, true);
        let mut processor = GainProcessor::new(control.clone(), 1_000, 1);
        let generation = control.fade_out();
        let mut fade_out = unit_buffer(10, 1);
        processor.process(&mut fade_out);
        let mut settled_silence = unit_buffer(1, 1);
        processor.process(&mut settled_silence);
        assert!(control.wait_for_transition(generation, Duration::ZERO));

        control.fade_in();
        let mut restarted = GainProcessor::new(control.clone(), 1_000, 1);
        let mut fade_in = unit_buffer(11, 1);
        restarted.process(&mut fade_in);

        assert_eq!(
            control.target_gain_bits.load(Ordering::Relaxed),
            0.25_f32.to_bits()
        );
        assert!(control.muted.load(Ordering::Relaxed));
        assert!(fade_in.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn unity_after_a_transport_transition_is_sample_identical() {
        let control = GainControl::default();
        control.fade_in();
        let mut processor = GainProcessor::new(control, 1_000, 2);
        let mut ramp = unit_buffer(11, 2);
        processor.process(&mut ramp);
        let mut settled = unit_buffer(1, 2);
        processor.process(&mut settled);

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
