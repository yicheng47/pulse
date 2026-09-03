use std::time::Duration;

const AUTO_DISMISS_AFTER: Duration = Duration::from_secs(6);
const MAX_VISIBLE_TOASTS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ToastId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToastKind {
    Error,
    Warning,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToastAction {
    SwitchToExclusive { device_uid: String },
}

impl ToastAction {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::SwitchToExclusive { .. } => "Switch to Exclusive",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Toast {
    pub(crate) kind: ToastKind,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) action: Option<ToastAction>,
}

impl Toast {
    pub(crate) fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: ToastKind::Error,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    pub(crate) fn warning(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: ToastKind::Warning,
            title: title.into(),
            body: body.into(),
            action: None,
        }
    }

    pub(crate) fn error_with_action(
        title: impl Into<String>,
        body: impl Into<String>,
        action: ToastAction,
    ) -> Self {
        Self {
            kind: ToastKind::Error,
            title: title.into(),
            body: body.into(),
            action: Some(action),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ToastEntry {
    pub(crate) id: ToastId,
    pub(crate) toast: Toast,
    timer: Option<ToastTimer>,
}

#[derive(Clone, Copy, Debug)]
struct ToastTimer {
    remaining: Duration,
    started_at: Option<Duration>,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ToastTimerSchedule {
    pub(crate) id: ToastId,
    pub(crate) generation: u64,
    pub(crate) after: Duration,
}

#[derive(Default)]
pub(crate) struct ToastState {
    entries: Vec<ToastEntry>,
    next_id: u64,
}

impl ToastState {
    pub(crate) fn entries(&self) -> &[ToastEntry] {
        &self.entries
    }

    pub(crate) fn push(&mut self, toast: Toast, now: Duration) -> Option<ToastTimerSchedule> {
        let id = ToastId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        let timer = toast.action.is_none().then_some(ToastTimer {
            remaining: AUTO_DISMISS_AFTER,
            started_at: Some(now),
            generation: 0,
        });
        self.entries.insert(0, ToastEntry { id, toast, timer });
        self.entries.truncate(MAX_VISIBLE_TOASTS);
        timer.map(|timer| ToastTimerSchedule {
            id,
            generation: timer.generation,
            after: timer.remaining,
        })
    }

    pub(crate) fn dismiss(&mut self, id: ToastId) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    pub(crate) fn take_action(&mut self, id: ToastId) -> Option<ToastAction> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        self.entries[index].toast.action.as_ref()?;
        self.entries.remove(index).toast.action
    }

    pub(crate) fn set_hovered(
        &mut self,
        id: ToastId,
        hovered: bool,
        now: Duration,
    ) -> Option<ToastTimerSchedule> {
        let entry = self.entries.iter_mut().find(|entry| entry.id == id)?;
        let timer = entry.timer.as_mut()?;
        match (hovered, timer.started_at) {
            (true, Some(started_at)) => {
                timer.remaining = timer
                    .remaining
                    .saturating_sub(now.saturating_sub(started_at));
                timer.started_at = None;
                timer.generation = timer.generation.wrapping_add(1);
                None
            }
            (false, None) => {
                timer.started_at = Some(now);
                timer.generation = timer.generation.wrapping_add(1);
                Some(ToastTimerSchedule {
                    id,
                    generation: timer.generation,
                    after: timer.remaining,
                })
            }
            _ => None,
        }
    }

    pub(crate) fn expire(&mut self, id: ToastId, generation: u64, now: Duration) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };
        let Some(timer) = self.entries[index].timer else {
            return false;
        };
        if timer.generation != generation
            || timer
                .started_at
                .is_none_or(|started_at| now.saturating_sub(started_at) < timer.remaining)
        {
            return false;
        }
        self.entries.remove(index);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(title: &str) -> Toast {
        Toast::error(title, "body")
    }

    #[test]
    fn plain_toast_expires_after_six_seconds() {
        let mut state = ToastState::default();
        let schedule = state.push(plain("one"), Duration::ZERO).unwrap();

        assert!(!state.expire(schedule.id, schedule.generation, Duration::from_secs(5)));
        assert!(state.expire(schedule.id, schedule.generation, Duration::from_secs(6)));
        assert!(state.entries().is_empty());
    }

    #[test]
    fn hover_pauses_and_resumes_the_remaining_timer() {
        let mut state = ToastState::default();
        let first = state.push(plain("one"), Duration::ZERO).unwrap();

        assert_eq!(
            state.set_hovered(first.id, true, Duration::from_secs(2)),
            None
        );
        assert!(!state.expire(first.id, first.generation, Duration::from_secs(6)));
        let resumed = state
            .set_hovered(first.id, false, Duration::from_secs(10))
            .unwrap();
        assert_eq!(resumed.after, Duration::from_secs(4));
        assert!(!state.expire(resumed.id, resumed.generation, Duration::from_secs(13)));
        assert!(state.expire(resumed.id, resumed.generation, Duration::from_secs(14)));
    }

    #[test]
    fn stacking_keeps_three_newest_on_top() {
        let mut state = ToastState::default();
        for title in ["one", "two", "three", "four"] {
            state.push(plain(title), Duration::ZERO);
        }

        assert_eq!(
            state
                .entries()
                .iter()
                .map(|entry| entry.toast.title.as_str())
                .collect::<Vec<_>>(),
            ["four", "three", "two"]
        );
    }

    #[test]
    fn action_toasts_have_no_timer() {
        let mut state = ToastState::default();
        let schedule = state.push(
            Toast::error_with_action(
                "DSD needs Exclusive output",
                "body",
                ToastAction::SwitchToExclusive {
                    device_uid: "matrix".to_string(),
                },
            ),
            Duration::ZERO,
        );
        let id = state.entries()[0].id;

        assert_eq!(schedule, None);
        assert!(!state.expire(id, 0, Duration::from_secs(60)));
        assert_eq!(state.entries().len(), 1);
    }
}
