//! Typed wrappers over Core Audio properties: ownership, formats/rates, volume, and capabilities.
//! Rate and format writes are checked by polling readback until they settle.

mod capabilities;
mod formats;
mod ownership;
mod property;
#[cfg(test)]
mod test_support;
mod volume;

use std::time::Duration;

const FORMAT_SETTLE_TIMEOUT: Duration = Duration::from_secs(2);
const FORMAT_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(crate) use capabilities::{
    audio_buffer_list_channel_count, is_integer_wire_format, output_device_capabilities,
};
pub use formats::{
    available_physical_formats, available_virtual_formats, output_streams, physical_format,
    set_physical_format, set_virtual_format, virtual_format,
};
pub(crate) use formats::{set_matching_physical_format, set_nominal_sample_rate};
pub use ownership::{FormatRestoreGuard, HogGuard, hog_owner, mixing_enabled, set_mixing_enabled};
pub(crate) use property::{address, check_status, get_array, get_bytes, get_cf_string, get_value};
pub(crate) use volume::{HardwareVolume, hardware_volume_control};

#[allow(
    unused_imports,
    reason = "Preserve the existing crate-visible HAL paths"
)]
pub(crate) use capabilities::ProbedOutputDeviceCapabilities;
#[allow(
    unused_imports,
    reason = "Preserve the existing crate-visible HAL paths"
)]
pub(crate) use property::{get_data_size, set_value};
