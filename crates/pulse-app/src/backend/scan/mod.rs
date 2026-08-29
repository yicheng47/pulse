pub(crate) mod metadata;
pub(super) mod path;
pub(super) mod walk;

use std::time::{SystemTime, UNIX_EPOCH};

use super::LibraryError;

pub(in crate::backend) fn system_time_ms(time: SystemTime) -> Result<i64, LibraryError> {
    system_time_units(time, 1_000)
}

pub(in crate::backend) fn system_time_ns(time: SystemTime) -> Result<i64, LibraryError> {
    system_time_units(time, 1_000_000_000)
}

fn system_time_units(time: SystemTime, units_per_second: u128) -> Result<i64, LibraryError> {
    let units = |duration: std::time::Duration| {
        u128::from(duration.as_secs())
            .saturating_mul(units_per_second)
            .saturating_add(
                u128::from(duration.subsec_nanos()).saturating_mul(units_per_second)
                    / 1_000_000_000,
            )
    };
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            i64::try_from(units(duration)).map_err(|_| LibraryError::IntegerOutOfRange("timestamp"))
        }
        Err(error) => i64::try_from(units(error.duration()))
            .map(|value| -value)
            .map_err(|_| LibraryError::IntegerOutOfRange("timestamp")),
    }
}
