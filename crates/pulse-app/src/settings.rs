use pulse_engine::device::Device;

pub(crate) const GITHUB_URL: &str = "https://github.com/yicheng47/pulse";
pub(crate) const LICENSE_URL: &str = "https://github.com/yicheng47/pulse/blob/main/LICENSE";
pub(crate) const ACKNOWLEDGEMENTS_URL: &str =
    "https://github.com/yicheng47/pulse/network/dependencies";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsSection {
    General,
    Output,
    Update,
    About,
}

impl SettingsSection {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Output => "Output",
            Self::Update => "Update",
            Self::About => "About",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::General => "icons/sliders-horizontal.svg",
            Self::Output => "icons/speaker.svg",
            Self::Update => "icons/refresh-cw.svg",
            Self::About => "icons/info.svg",
        }
    }
}

pub(crate) const SETTINGS_GROUPS: &[(&str, &[SettingsSection])] = &[
    (
        "SETTINGS",
        &[SettingsSection::General, SettingsSection::Output],
    ),
    ("APP", &[SettingsSection::Update, SettingsSection::About]),
];

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SettingsViewModel {
    pub(crate) section: SettingsSection,
    pub(crate) output_device_name: String,
}

impl SettingsViewModel {
    pub(crate) fn new(section: SettingsSection, active_device: Option<&Device>) -> Self {
        Self {
            section,
            output_device_name: active_device
                .map(|device| device.name.clone())
                .unwrap_or_else(|| "No active output".to_string()),
        }
    }

    pub(crate) fn is_selected(&self, section: SettingsSection) -> bool {
        self.section == section
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AboutLink {
    GitHub,
    License,
    Acknowledgements,
}

impl AboutLink {
    pub(crate) const ALL: [Self; 3] = [Self::GitHub, Self::License, Self::Acknowledgements];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::License => "License",
            Self::Acknowledgements => "Acknowledgements",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::GitHub => "Source, issues, and releases.",
            Self::License => "MIT.",
            Self::Acknowledgements => "GPUI, Symphonia, Lofty, and the Rust audio ecosystem.",
        }
    }

    pub(crate) fn url(self) -> &'static str {
        match self {
            Self::GitHub => GITHUB_URL,
            Self::License => LICENSE_URL,
            Self::Acknowledgements => ACKNOWLEDGEMENTS_URL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_selection_marks_only_the_current_section() {
        let model = SettingsViewModel::new(SettingsSection::Update, None);

        let selected: Vec<bool> = SETTINGS_GROUPS
            .iter()
            .flat_map(|(_, sections)| sections.iter())
            .map(|section| model.is_selected(*section))
            .collect();

        assert_eq!(selected, [false, false, true, false]);
    }

    #[test]
    fn every_section_is_reachable_once_in_design_order() {
        assert_eq!(
            SETTINGS_GROUPS,
            &[
                (
                    "SETTINGS",
                    &[SettingsSection::General, SettingsSection::Output][..],
                ),
                (
                    "APP",
                    &[SettingsSection::Update, SettingsSection::About][..],
                ),
            ]
        );

        let listed: Vec<SettingsSection> = SETTINGS_GROUPS
            .iter()
            .flat_map(|(_, sections)| sections.iter().copied())
            .collect();
        assert_eq!(listed.len(), 4);
        for section in [
            SettingsSection::General,
            SettingsSection::Output,
            SettingsSection::Update,
            SettingsSection::About,
        ] {
            assert_eq!(
                listed.iter().filter(|listed| **listed == section).count(),
                1,
                "{}",
                section.label()
            );
        }
    }

    #[test]
    fn output_row_resolves_the_current_device_name() {
        let device = Device {
            id: 7,
            uid: "matrix-mini-i".to_string(),
            name: "Matrix mini-i Pro 4".to_string(),
        };

        assert_eq!(
            SettingsViewModel::new(SettingsSection::General, Some(&device)).output_device_name,
            "Matrix mini-i Pro 4"
        );
        assert_eq!(
            SettingsViewModel::new(SettingsSection::General, None).output_device_name,
            "No active output"
        );
    }
}
