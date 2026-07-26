#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Loading,
    Playing,
    Paused,
    Stopping,
    Ended,
    Error,
}

impl PlaybackState {
    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        use PlaybackState::*;

        matches!(
            (self, next),
            (Idle | Ended | Error, Loading)
                | (Ended | Error, Stopping)
                | (Loading, Playing | Error)
                | (Playing, Loading | Paused | Stopping | Ended | Error)
                | (Paused, Loading | Stopping | Error)
                | (Stopping, Idle | Error)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PlaybackState::*;

    #[test]
    fn permits_product_playback_transitions() {
        assert!(Idle.can_transition_to(Loading));
        assert!(Loading.can_transition_to(Playing));
        assert!(Playing.can_transition_to(Paused));
        assert!(Paused.can_transition_to(Loading));
        assert!(Playing.can_transition_to(Ended));
        assert!(Ended.can_transition_to(Stopping));
        assert!(Stopping.can_transition_to(Idle));
    }

    #[test]
    fn rejects_transitions_that_skip_controller_work() {
        assert!(!Idle.can_transition_to(Playing));
        assert!(!Paused.can_transition_to(Playing));
        assert!(!Playing.can_transition_to(Idle));
        assert!(!Ended.can_transition_to(Playing));
    }
}
