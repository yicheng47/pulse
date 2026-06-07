use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{call} failed (OSStatus {status})")]
    Os { call: &'static str, status: i32 },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("device hogged by pid {0}")]
    Hogged(i32),
    #[error("no physical format matches {0:?}")]
    NoMatchingFormat(crate::PcmFormat),
    #[error("decode: {0}")]
    Decode(String),
}
