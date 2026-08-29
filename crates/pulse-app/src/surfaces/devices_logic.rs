use std::time::{SystemTime, UNIX_EPOCH};

use crate::playback::ManagedDevice;

pub(super) fn device_class(device: &ManagedDevice) -> &'static str {
    match device.capabilities {
        Some(capabilities) if capabilities.max_bits_per_channel.is_some() => "DAC",
        Some(_) => "Bluetooth",
        None => "Unknown",
    }
}

pub(super) fn format_last_seen(last_seen_unix_seconds: Option<u64>) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_last_seen_at(last_seen_unix_seconds, now)
}

fn format_last_seen_at(last_seen_unix_seconds: Option<u64>, now: u64) -> String {
    let Some(last_seen) = last_seen_unix_seconds else {
        return "Last seen before this version of Pulse".to_string();
    };
    let elapsed = now.saturating_sub(last_seen);
    if elapsed < 60 {
        return "Last seen just now".to_string();
    }
    let relative = if elapsed < 3_600 {
        format_unit(elapsed / 60, "minute")
    } else if elapsed < 86_400 {
        format_unit(elapsed / 3_600, "hour")
    } else if elapsed < 604_800 {
        format_unit(elapsed / 86_400, "day")
    } else if elapsed < 2_592_000 {
        format_unit(elapsed / 604_800, "week")
    } else if elapsed < 31_536_000 {
        format_unit(elapsed / 2_592_000, "month")
    } else {
        format_unit(elapsed / 31_536_000, "year")
    };
    format!("Last seen {relative} ago")
}

fn format_unit(value: u64, unit: &str) -> String {
    if value == 1 {
        format!("1 {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_seen_copy_uses_readable_time_units() {
        assert_eq!(format_unit(1, "day"), "1 day");
        assert_eq!(format_unit(3, "day"), "3 days");
        assert_eq!(
            format_last_seen_at(None, 1_000),
            "Last seen before this version of Pulse"
        );
        assert_eq!(format_last_seen_at(Some(990), 1_000), "Last seen just now");
        assert_eq!(
            format_last_seen_at(Some(1_000), 1_000 + 3 * 86_400),
            "Last seen 3 days ago"
        );
    }
}
