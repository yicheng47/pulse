use std::{fmt, time::Duration};

use serde::Deserialize;
use thiserror::Error;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/yicheng47/pulse/releases/latest";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const USER_AGENT: &str = concat!("Pulse/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AvailableUpdate {
    pub(crate) version: String,
    pub(crate) url: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum UpdateState {
    #[default]
    Idle,
    Checking,
    Available(AvailableUpdate),
    Failed,
}

impl UpdateState {
    pub(crate) fn begin(&mut self) {
        *self = Self::Checking;
    }

    pub(crate) fn finish(
        &mut self,
        kind: UpdateCheckKind,
        result: &Result<Option<AvailableUpdate>, String>,
    ) {
        match result {
            Ok(Some(release)) => *self = Self::Available(release.clone()),
            Ok(None) => *self = Self::Idle,
            Err(_) if kind == UpdateCheckKind::Manual => *self = Self::Failed,
            Err(_) => *self = Self::Idle,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpdateCheckKind {
    Launch,
    Manual,
}

impl UpdateCheckKind {
    pub(crate) fn launch_if_enabled(enabled: bool) -> Option<Self> {
        enabled.then_some(Self::Launch)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, u64, u64);

impl Version {
    fn parse(value: &str) -> Option<Self> {
        let mut components = value.split('.');
        let major = parse_version_component(components.next()?)?;
        let minor = parse_version_component(components.next()?)?;
        let patch = parse_version_component(components.next()?)?;
        if components.next().is_some() {
            return None;
        }
        Some(Self(major, minor, patch))
    }

    fn parse_tag(tag: &str) -> Option<Self> {
        Self::parse(tag.strip_prefix('v')?)
    }
}

fn parse_version_component(component: &str) -> Option<u64> {
    if component.is_empty()
        || !component.bytes().all(|byte| byte.is_ascii_digit())
        || (component.len() > 1 && component.starts_with('0'))
    {
        return None;
    }
    component.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Deserialize)]
struct LatestReleaseResponse {
    tag_name: String,
    html_url: String,
}

#[derive(Debug, Error)]
pub(crate) enum UpdateCheckError {
    #[error("GitHub request failed: {0}")]
    Request(#[from] ureq::Error),
    #[error("GitHub response was invalid: {0}")]
    Response(#[from] serde_json::Error),
    #[error("running version is not MAJOR.MINOR.PATCH: {0}")]
    RunningVersion(String),
}

pub(crate) fn fetch_latest_release() -> Result<Option<AvailableUpdate>, UpdateCheckError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(LATEST_RELEASE_URL)
        .header("User-Agent", USER_AGENT)
        .call()?;
    let body = response.body_mut().read_to_string()?;
    parse_latest_release(&body, env!("CARGO_PKG_VERSION"))
}

fn parse_latest_release(
    body: &str,
    running_version: &str,
) -> Result<Option<AvailableUpdate>, UpdateCheckError> {
    let response: LatestReleaseResponse = serde_json::from_str(body)?;
    let Some(latest) = Version::parse_tag(&response.tag_name) else {
        return Ok(None);
    };
    let running = Version::parse(running_version)
        .ok_or_else(|| UpdateCheckError::RunningVersion(running_version.to_string()))?;
    if latest <= running {
        return Ok(None);
    }
    Ok(Some(AvailableUpdate {
        version: latest.to_string(),
        url: response.html_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LATEST_RELEASE: &str = include_str!("update/fixtures/latest-release.json");
    const MALFORMED_RELEASE: &str = include_str!("update/fixtures/malformed-release.json");
    const MISSING_FIELD_RELEASE: &str = include_str!("update/fixtures/missing-field-release.json");

    fn release_body(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/yicheng47/pulse/releases/tag/{tag}"}}"#
        )
    }

    #[test]
    fn newer_version_is_reported_from_the_captured_release() {
        assert_eq!(
            parse_latest_release(LATEST_RELEASE, "0.1.0").unwrap(),
            Some(AvailableUpdate {
                version: "0.2.0".to_string(),
                url: "https://github.com/yicheng47/pulse/releases/tag/v0.2.0".to_string(),
            })
        );
    }

    #[test]
    fn older_version_is_ignored() {
        assert_eq!(
            parse_latest_release(&release_body("v0.1.9"), "0.2.0").unwrap(),
            None
        );
    }

    #[test]
    fn equal_version_is_ignored() {
        assert_eq!(
            parse_latest_release(&release_body("v0.2.0"), "0.2.0").unwrap(),
            None
        );
    }

    #[test]
    fn malformed_version_tags_are_ignored() {
        for tag in ["0.2.0", "v0.2", "v0.2.0.1", "v0.two.0", "v01.2.3", "v"] {
            assert_eq!(
                parse_latest_release(&release_body(tag), "0.1.0").unwrap(),
                None,
                "tag {tag} should be ignored"
            );
        }
    }

    #[test]
    fn prerelease_and_suffixed_tags_are_ignored() {
        for tag in ["v0.2.0-beta.1", "v0.2.0-rc.1", "v0.2.0+build.1"] {
            assert_eq!(
                parse_latest_release(&release_body(tag), "0.1.0").unwrap(),
                None,
                "tag {tag} should be ignored"
            );
        }
    }

    #[test]
    fn malformed_release_body_is_rejected() {
        assert!(matches!(
            parse_latest_release(MALFORMED_RELEASE, "0.1.0"),
            Err(UpdateCheckError::Response(_))
        ));
    }

    #[test]
    fn release_body_missing_a_required_field_is_rejected() {
        assert!(matches!(
            parse_latest_release(MISSING_FIELD_RELEASE, "0.1.0"),
            Err(UpdateCheckError::Response(_))
        ));
    }

    #[test]
    fn manual_checks_drive_every_visible_state() {
        let release = AvailableUpdate {
            version: "0.2.0".to_string(),
            url: "https://example.com/release".to_string(),
        };
        let mut state = UpdateState::Idle;

        state.begin();
        assert_eq!(state, UpdateState::Checking);
        state.finish(UpdateCheckKind::Manual, &Ok(Some(release.clone())));
        assert_eq!(state, UpdateState::Available(release));
        state.begin();
        state.finish(UpdateCheckKind::Manual, &Ok(None));
        assert_eq!(state, UpdateState::Idle);
        state.begin();
        state.finish(UpdateCheckKind::Manual, &Err("offline".to_string()));
        assert_eq!(state, UpdateState::Failed);
    }

    #[test]
    fn disabled_launch_checks_are_suppressed_and_failures_stay_silent() {
        assert_eq!(UpdateCheckKind::launch_if_enabled(false), None);
        assert_eq!(
            UpdateCheckKind::launch_if_enabled(true),
            Some(UpdateCheckKind::Launch)
        );

        let mut state = UpdateState::Idle;
        state.begin();
        assert_eq!(state, UpdateState::Checking);
        state.finish(UpdateCheckKind::Launch, &Err("offline".to_string()));
        assert_eq!(state, UpdateState::Idle);
    }
}
