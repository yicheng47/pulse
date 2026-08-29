use std::time::SystemTime;

use chrono::{DateTime, Local};

pub(super) struct LastCheckedCopy {
    pub(super) description: String,
    pub(super) value: String,
}

pub(super) fn format_last_checked(checked_at: Option<SystemTime>) -> LastCheckedCopy {
    format_last_checked_at(checked_at, SystemTime::now())
}

fn format_last_checked_at(checked_at: Option<SystemTime>, now: SystemTime) -> LastCheckedCopy {
    let Some(checked_at) = checked_at else {
        return LastCheckedCopy {
            description: "Refreshes while this page is open".into(),
            value: "Never".into(),
        };
    };

    let checked_at_local = DateTime::<Local>::from(checked_at);
    let now_local = DateTime::<Local>::from(now);
    let checked_at_label = if checked_at_local.date_naive() == now_local.date_naive() {
        checked_at_local.format("Today at %-I:%M %p").to_string()
    } else {
        checked_at_local
            .format("%b %-d, %Y at %-I:%M %p")
            .to_string()
    };
    let elapsed_seconds = now
        .duration_since(checked_at)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let value = match elapsed_seconds {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", elapsed_seconds / 60),
        3600..86400 => format!("{} hr ago", elapsed_seconds / 3600),
        _ => format!("{} d ago", elapsed_seconds / 86400),
    };

    LastCheckedCopy {
        description: format!("{checked_at_label} — refreshes while this page is open"),
        value,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn last_checked_copy_tracks_boundaries_and_local_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let cases = [
            (59, "Just now"),
            (60, "1 min ago"),
            (3_599, "59 min ago"),
            (3_600, "1 hr ago"),
            (86_399, "23 hr ago"),
            (86_400, "1 d ago"),
        ];
        for (elapsed, expected) in cases {
            assert_eq!(
                format_last_checked_at(Some(now - Duration::from_secs(elapsed)), now).value,
                expected
            );
        }

        assert_eq!(format_last_checked_at(None, now).value, "Never");
        assert!(
            format_last_checked_at(Some(now - Duration::from_secs(60)), now)
                .description
                .starts_with("Today at ")
        );
        assert!(
            !format_last_checked_at(Some(now - Duration::from_secs(86_400)), now)
                .description
                .starts_with("Today at ")
        );
    }
}
