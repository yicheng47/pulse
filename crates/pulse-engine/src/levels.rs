//! VU (RMS/peak) and spectrum from the realtime tap. FFT via realfft, computed
//! off the audio thread from a tapped copy of the buffer.

#[derive(Debug, Clone, Copy, Default)]
pub struct Levels {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}
