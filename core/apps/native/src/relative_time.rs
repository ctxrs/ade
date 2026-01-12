use chrono::{DateTime, Utc};

pub(crate) fn format_relative_age_short(when: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(when) = when else {
        return String::new();
    };

    let diff_sec = now
        .signed_duration_since(when)
        .num_seconds()
        .max(0);

    if diff_sec < 45 {
        return "Now".to_string();
    }

    let diff_min = diff_sec / 60;
    if diff_min < 60 {
        return format!("{}m", diff_min.max(1));
    }

    let diff_hr = diff_min / 60;
    if diff_hr < 24 {
        return format!("{}h", diff_hr);
    }

    let diff_day = diff_hr / 24;
    if diff_day < 7 {
        return format!("{}d", diff_day);
    }

    let diff_week = diff_day / 7;
    if diff_week < 52 {
        return format!("{}w", diff_week);
    }

    let diff_year = diff_week / 52;
    format!("{}y", diff_year.max(1))
}
