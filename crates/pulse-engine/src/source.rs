use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayableSource {
    pub path: PathBuf,
    pub duration_ms: Option<u64>,
}
