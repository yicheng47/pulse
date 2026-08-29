pub(super) fn format_playlist_duration(duration_ms: u64) -> String {
    let minutes = duration_ms / 60_000;
    if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}

pub(super) fn format_updated(timestamp_ms: i64, now_ms: i64) -> String {
    let minutes = now_ms.saturating_sub(timestamp_ms) / 60_000;
    if minutes < 1 {
        "now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 24 * 60 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}d", minutes / (24 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_playlist_durations_for_rows() {
        assert_eq!(format_playlist_duration(0), "0m");
        assert_eq!(format_playlist_duration(55 * 60_000), "55m");
        assert_eq!(format_playlist_duration(192 * 60_000), "3h 12m");
    }
}
