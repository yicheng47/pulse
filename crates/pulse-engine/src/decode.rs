//! Symphonia decode: FLAC / ALAC / AIFF / WAV → interleaved integer PCM.
//! Runs on the decode thread, pushes into the rtrb producer.

use std::path::Path;

use crate::{PcmFormat, error::EngineError};

pub struct DecodedStream {
    pub format: PcmFormat,
}

pub fn open(_path: &Path) -> Result<DecodedStream, EngineError> {
    todo!()
}
