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

pub(crate) const INTERFACE_SCALE_STEPS: [f32; 6] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5];
const DEFAULT_INTERFACE_SCALE_INDEX: usize = 2;
pub(crate) const DEFAULT_INTERFACE_SCALE: f32 =
    INTERFACE_SCALE_STEPS[DEFAULT_INTERFACE_SCALE_INDEX];

fn interface_scale_index(scale: f32) -> usize {
    if !scale.is_finite() {
        return DEFAULT_INTERFACE_SCALE_INDEX;
    }

    INTERFACE_SCALE_STEPS
        .iter()
        .enumerate()
        .fold(0, |nearest, (index, step)| {
            if (*step - scale).abs() < (INTERFACE_SCALE_STEPS[nearest] - scale).abs() {
                index
            } else {
                nearest
            }
        })
}

pub(crate) fn snap_interface_scale(scale: f32) -> f32 {
    INTERFACE_SCALE_STEPS[interface_scale_index(scale)]
}

pub(crate) fn next_interface_scale(scale: f32) -> f32 {
    let index = (interface_scale_index(scale) + 1).min(INTERFACE_SCALE_STEPS.len() - 1);
    INTERFACE_SCALE_STEPS[index]
}

pub(crate) fn previous_interface_scale(scale: f32) -> f32 {
    INTERFACE_SCALE_STEPS[interface_scale_index(scale).saturating_sub(1)]
}

pub(crate) fn interface_scale_label(scale: f32) -> String {
    format!("{:.0}%", snap_interface_scale(scale) * 100.)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SettingsViewModel {
    pub(crate) section: SettingsSection,
}

impl SettingsViewModel {
    pub(crate) fn new(section: SettingsSection) -> Self {
        Self { section }
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
        let model = SettingsViewModel::new(SettingsSection::Update);

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
    fn settings_view_model_only_tracks_the_selected_section() {
        assert_eq!(
            SettingsViewModel::new(SettingsSection::General),
            SettingsViewModel {
                section: SettingsSection::General,
            }
        );
    }

    #[test]
    fn interface_scale_steps_move_without_wrapping() {
        assert_eq!(
            INTERFACE_SCALE_STEPS.map(previous_interface_scale),
            [0.8, 0.8, 0.9, 1.0, 1.1, 1.25]
        );
        assert_eq!(
            INTERFACE_SCALE_STEPS.map(next_interface_scale),
            [0.9, 1.0, 1.1, 1.25, 1.5, 1.5]
        );
    }

    #[test]
    fn interface_scale_snaps_to_the_nearest_step() {
        assert_eq!(snap_interface_scale(1.2), 1.25);
        assert_eq!(snap_interface_scale(0.1), 0.8);
        assert_eq!(snap_interface_scale(2.0), 1.5);
        assert_eq!(snap_interface_scale(f32::NAN), 1.0);
        assert_eq!(snap_interface_scale(1.375), 1.25);
    }

    #[test]
    fn interface_scale_labels_use_percentages() {
        assert_eq!(
            INTERFACE_SCALE_STEPS.map(interface_scale_label),
            ["80%", "90%", "100%", "110%", "125%", "150%"]
        );
    }
}
