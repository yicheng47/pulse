//! App rendering surfaces and the pure helpers each product area owns.

mod devices;
mod devices_logic;
pub(crate) mod library;
mod playback_popovers;
mod playback_row;
mod search;
mod settings;
mod shell;
mod sidebar;
mod sidebar_logic;

pub(crate) use devices::DeviceManagementPage;
pub(crate) use library::{LibraryView, logic::SearchViewModel};
pub(crate) use playback_row::{PlaybackRow, PlaybackSurface};
pub(crate) use shell::{Shell, TOP_BAR_HEIGHT};
pub(crate) use sidebar_logic::{Destination, NAV_GROUPS, SIDEBAR_TOP_PADDING, SIDEBAR_WIDTH};
