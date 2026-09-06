use crate::{EngineError, PcmFormat, decode::PcmDecoder, decode_dsd::DsdDopDecoder};
use std::{path::Path, sync::Arc};

pub(super) type DecoderFactory =
    Arc<dyn Fn(&Path) -> Result<Box<dyn SourceDecoder>, EngineError> + Send + Sync>;

/// What the worker needs from a decoder: format and duration up front, `seek` returning the
/// position actually reached, and `next_pcm` filling a chunk of interleaved PCM in `format`
/// (`Ok(None)` at end of stream). PCM files come through symphonia, DSD through the DoP packer.
pub(super) trait SourceDecoder {
    fn format(&self) -> PcmFormat;
    fn duration_ms(&self) -> Option<u64>;
    fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError>;
    fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError>;
}

impl SourceDecoder for PcmDecoder {
    fn format(&self) -> PcmFormat {
        self.format()
    }

    fn duration_ms(&self) -> Option<u64> {
        self.duration_ms()
    }

    fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
        self.seek(position_ms)
    }

    fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
        self.next_pcm(pcm)
    }
}

impl SourceDecoder for DsdDopDecoder {
    fn format(&self) -> PcmFormat {
        self.format()
    }

    fn duration_ms(&self) -> Option<u64> {
        Some(self.duration_ms())
    }

    fn seek(&mut self, position_ms: u64) -> Result<u64, EngineError> {
        self.seek(position_ms)
    }

    fn next_pcm(&mut self, pcm: &mut Vec<u8>) -> Result<Option<u64>, EngineError> {
        self.next_pcm(pcm)
    }
}
