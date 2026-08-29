pub(crate) const SIDEBAR_WIDTH: f32 = 236.0;
pub(crate) const SIDEBAR_TOP_PADDING: f32 = 56.0;

#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    Albums,
    Artists,
    Tracks,
    Playlists,
    Storage,
    Devices,
}

impl Destination {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Tracks => "Tracks",
            Self::Playlists => "Playlists",
            Self::Storage => "Storage",
            Self::Devices => "Devices",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Albums => "icons/library.svg",
            Self::Artists => "icons/mic-vocal.svg",
            Self::Tracks => "icons/music.svg",
            Self::Playlists => "icons/list-music.svg",
            Self::Storage => "icons/database.svg",
            Self::Devices => "icons/speaker.svg",
        }
    }
}

pub(crate) const NAV_GROUPS: &[(&str, &[Destination])] = &[
    (
        "LIBRARY",
        &[
            Destination::Albums,
            Destination::Artists,
            Destination::Tracks,
            Destination::Playlists,
        ],
    ),
    ("MANAGE", &[Destination::Storage]),
    ("OUTPUT", &[Destination::Devices]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_destination_is_reachable_from_exactly_one_nav_group() {
        let listed: Vec<Destination> = NAV_GROUPS
            .iter()
            .flat_map(|(_, destinations)| destinations.iter().copied())
            .collect();

        assert_eq!(listed.len(), 6);
        for destination in [
            Destination::Albums,
            Destination::Artists,
            Destination::Tracks,
            Destination::Playlists,
            Destination::Storage,
            Destination::Devices,
        ] {
            assert_eq!(
                listed
                    .iter()
                    .filter(|listed| **listed == destination)
                    .count(),
                1,
                "{}",
                destination.label()
            );
        }
    }
}
