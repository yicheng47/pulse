use pulse_engine::PlaybackState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PendingToggle {
    from: PlaybackState,
    target: PlaybackState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TransportPresentation {
    pub(super) previous_enabled: bool,
    pub(super) next_enabled: bool,
    pub(super) show_pause: bool,
}

pub(super) fn transport_presentation(
    playback_state: PlaybackState,
    pending_toggle: Option<PendingToggle>,
    has_loaded_track: bool,
    queue_can_advance: bool,
) -> TransportPresentation {
    let displayed_state = pending_toggle
        .map(|pending| pending.target)
        .unwrap_or(playback_state);
    TransportPresentation {
        previous_enabled: has_loaded_track,
        next_enabled: queue_can_advance,
        show_pause: matches!(
            displayed_state,
            PlaybackState::Playing | PlaybackState::Loading
        ),
    }
}

pub(super) fn begin_pending_toggle(
    playback_state: PlaybackState,
    has_restart_target: bool,
) -> Option<PendingToggle> {
    let target = match playback_state {
        PlaybackState::Playing => PlaybackState::Paused,
        PlaybackState::Paused => PlaybackState::Playing,
        PlaybackState::Idle | PlaybackState::Ended if has_restart_target => PlaybackState::Playing,
        PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error => return None,
        PlaybackState::Loading | PlaybackState::Stopping => return None,
    };
    Some(PendingToggle {
        from: playback_state,
        target,
    })
}

pub(super) fn reconcile_pending_toggle(
    pending_toggle: Option<PendingToggle>,
    playback_state: PlaybackState,
) -> Option<PendingToggle> {
    pending_toggle.filter(|pending| {
        playback_state == pending.from
            || matches!(
                playback_state,
                PlaybackState::Loading | PlaybackState::Stopping
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_availability_is_unchanged_across_resume_states() {
        let availability = [
            PlaybackState::Paused,
            PlaybackState::Loading,
            PlaybackState::Playing,
        ]
        .map(|state| {
            let presentation = transport_presentation(state, None, true, true);
            (presentation.previous_enabled, presentation.next_enabled)
        });

        assert_eq!(availability, [(true, true); 3]);
    }

    #[test]
    fn pending_toggle_state_flips_the_icon_optimistically() {
        let resume = begin_pending_toggle(PlaybackState::Paused, true);
        assert!(transport_presentation(PlaybackState::Paused, resume, true, true).show_pause);
        let pause = begin_pending_toggle(PlaybackState::Playing, true);
        assert!(!transport_presentation(PlaybackState::Playing, pause, true, true).show_pause);
        for state in [PlaybackState::Idle, PlaybackState::Ended] {
            assert!(
                transport_presentation(state, begin_pending_toggle(state, true), true, true)
                    .show_pause
            );
        }
    }

    #[test]
    fn settled_non_playing_states_without_a_restart_target_do_not_flip() {
        for state in [
            PlaybackState::Idle,
            PlaybackState::Ended,
            PlaybackState::Error,
        ] {
            assert_eq!(begin_pending_toggle(state, false), None);
            assert!(!transport_presentation(state, None, false, false).show_pause);
        }
    }

    #[test]
    fn error_with_a_restart_target_does_not_flip_optimistically() {
        assert_eq!(begin_pending_toggle(PlaybackState::Error, true), None);
    }

    #[test]
    fn pending_resume_survives_loading_until_playing_confirms() {
        let mut pending = begin_pending_toggle(PlaybackState::Paused, true);
        pending = reconcile_pending_toggle(pending, PlaybackState::Paused);
        assert!(pending.is_some());
        assert!(transport_presentation(PlaybackState::Paused, pending, true, true).show_pause);

        pending = reconcile_pending_toggle(pending, PlaybackState::Loading);
        assert!(pending.is_some());
        assert!(transport_presentation(PlaybackState::Loading, pending, true, true).show_pause);

        pending = reconcile_pending_toggle(pending, PlaybackState::Playing);
        assert!(pending.is_none());
        assert!(transport_presentation(PlaybackState::Playing, pending, true, true).show_pause);
    }

    #[test]
    fn pending_pause_survives_position_updates_until_paused_confirms() {
        let mut pending = begin_pending_toggle(PlaybackState::Playing, true);
        assert!(!transport_presentation(PlaybackState::Playing, pending, true, true).show_pause);
        pending = reconcile_pending_toggle(pending, PlaybackState::Playing);
        assert!(pending.is_some());
        assert!(!transport_presentation(PlaybackState::Playing, pending, true, true).show_pause);

        pending = reconcile_pending_toggle(pending, PlaybackState::Paused);
        assert!(pending.is_none());
        assert!(!transport_presentation(PlaybackState::Paused, pending, true, true).show_pause);
    }

    #[test]
    fn pending_resume_reconciles_to_an_error_snapshot() {
        let mut pending = begin_pending_toggle(PlaybackState::Paused, true);
        pending = reconcile_pending_toggle(pending, PlaybackState::Loading);
        assert!(pending.is_some());
        assert!(transport_presentation(PlaybackState::Loading, pending, true, true).show_pause);

        pending = reconcile_pending_toggle(pending, PlaybackState::Error);
        assert!(pending.is_none());
        assert!(!transport_presentation(PlaybackState::Error, pending, true, true).show_pause);
    }

    #[test]
    fn pending_idle_restart_reconciles_to_an_error_snapshot() {
        let mut pending = begin_pending_toggle(PlaybackState::Idle, true);
        assert!(pending.is_some());
        assert!(transport_presentation(PlaybackState::Idle, pending, true, true).show_pause);

        pending = reconcile_pending_toggle(pending, PlaybackState::Error);
        assert!(pending.is_none());
        assert!(!transport_presentation(PlaybackState::Error, pending, true, true).show_pause);
    }

    #[test]
    fn track_change_loading_keeps_the_pause_icon() {
        for state in [
            PlaybackState::Playing,
            PlaybackState::Loading,
            PlaybackState::Playing,
        ] {
            assert!(transport_presentation(state, None, true, true).show_pause);
        }
    }

    #[test]
    fn next_from_paused_shows_pause_once_loading_starts() {
        assert!(!transport_presentation(PlaybackState::Paused, None, true, true).show_pause);
        assert!(transport_presentation(PlaybackState::Loading, None, true, true).show_pause);
    }

    #[test]
    fn stopping_and_idle_show_the_play_icon() {
        for state in [PlaybackState::Stopping, PlaybackState::Idle] {
            assert!(!transport_presentation(state, None, true, true).show_pause);
        }
    }
}
