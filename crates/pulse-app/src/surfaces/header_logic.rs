#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HeaderState {
    pub(super) settings_active: bool,
    pub(super) update_version: Option<String>,
}

impl HeaderState {
    pub(super) fn new(settings_open: bool, update_version: Option<String>) -> Self {
        Self {
            settings_active: settings_open,
            update_version,
        }
    }

    pub(super) fn update_hint_version(&self) -> Option<&str> {
        self.update_version.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gear_is_active_exactly_when_settings_are_open() {
        assert!(!HeaderState::new(false, None).settings_active);
        assert!(HeaderState::new(true, None).settings_active);
    }

    #[test]
    fn update_hint_visibility_and_version_follow_update_readiness() {
        let unavailable = HeaderState::new(false, None);
        assert_eq!(unavailable.update_hint_version(), None);

        let ready = HeaderState::new(false, Some("1.2.3".to_string()));
        assert_eq!(ready.update_hint_version(), Some("1.2.3"));
    }
}
