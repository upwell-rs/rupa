use std::time::Duration;

use super::State;

#[test]
fn lock_waits_round_up_to_whole_seconds_and_saturate() {
    let wait = |lock_timeout| State {
        lock_timeout,
        ..State::default()
    };

    assert_eq!(wait(Duration::ZERO).lock_seconds(), 0);
    assert_eq!(wait(Duration::from_millis(250)).lock_seconds(), 1);
    assert_eq!(wait(Duration::from_secs(2)).lock_seconds(), 2);
    assert_eq!(wait(Duration::MAX).lock_seconds(), i64::MAX);
}
