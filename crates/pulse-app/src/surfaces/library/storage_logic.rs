use crate::backend::ScanOutcome;

pub(super) fn scan_outcome_label(outcome: ScanOutcome) -> &'static str {
    match outcome {
        ScanOutcome::Completed => "Complete",
        ScanOutcome::CompletedWithErrors => "Issues",
        ScanOutcome::Offline => "Offline",
        ScanOutcome::Failed => "Failed",
    }
}

pub(super) fn format_relative_time(timestamp_ms: i64, now_ms: i64) -> String {
    let age_ms = now_ms.saturating_sub(timestamp_ms);
    let minutes = age_ms / 60_000;
    if minutes < 1 {
        "Just now".to_string()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 24 * 60 {
        format!("{}h ago", minutes / 60)
    } else {
        format!("{}d ago", minutes / (24 * 60))
    }
}
