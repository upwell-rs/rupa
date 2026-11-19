use std::time::{SystemTime, UNIX_EPOCH};

/// `now` as a fourteen-digit UTC `YYYYMMDDhhmmss`; a time before 1970 gives
/// an invalid version.
pub(super) fn timestamp(now: SystemTime) -> String {
    let Ok(elapsed) = now.duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    let seconds = elapsed.as_secs();
    let (days, time) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil(days);

    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// The Gregorian date `days` after 1970-01-01.
fn civil(days: u64) -> (u64, u64, u64) {
    let days = days + 719_468;
    let era = days / 146_097;
    let day_of_era = days % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    (year, month, day)
}
