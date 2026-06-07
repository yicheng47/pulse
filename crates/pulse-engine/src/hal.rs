//! Safe wrapper over the AudioObject property API: hog mode, physical format,
//! nominal sample rate, property listeners.
//!
//! Crib sheet: coreaudio-rs `macos_helpers` (post-PR #128 it uses these same
//! objc2 bindings). Rate/format switches are async — always wait on a property
//! listener before trusting the new state.
