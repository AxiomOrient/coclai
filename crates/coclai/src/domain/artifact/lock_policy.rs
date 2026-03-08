use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LockMetadata {
    pub(super) pid: u32,
    pub(super) created_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LockOwnerStatus {
    Alive,
    Dead,
    #[cfg_attr(unix, allow(dead_code))]
    Unknown,
}

pub(super) fn parse_lock_metadata(raw: &str) -> Option<LockMetadata> {
    let mut parts = raw.trim().splitn(2, ':');
    let pid = parts.next()?.parse::<u32>().ok()?;
    // Legacy compatibility: pre-existing pid-only lock payloads are treated as stale candidates.
    let created_unix_ms = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    Some(LockMetadata {
        pid,
        created_unix_ms,
    })
}

pub(super) fn should_reap_lock(
    owner_status: LockOwnerStatus,
    created_unix_ms: Option<u64>,
    now_unix_ms: u64,
    stale_fallback_age: Duration,
) -> bool {
    match owner_status {
        LockOwnerStatus::Dead => true,
        LockOwnerStatus::Alive => false,
        LockOwnerStatus::Unknown => created_unix_ms
            .map(|created| lock_age_exceeds_threshold(created, now_unix_ms, stale_fallback_age))
            .unwrap_or(false),
    }
}

fn lock_age_exceeds_threshold(created_unix_ms: u64, now_unix_ms: u64, threshold: Duration) -> bool {
    let threshold_ms = u64::try_from(threshold.as_millis()).unwrap_or(u64::MAX);
    now_unix_ms.saturating_sub(created_unix_ms) >= threshold_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: Duration = Duration::from_secs(30);

    #[test]
    fn parse_lock_metadata_supports_pid_and_timestamp() {
        let parsed = parse_lock_metadata("42:1700000000\n").expect("parse metadata");
        assert_eq!(
            parsed,
            LockMetadata {
                pid: 42,
                created_unix_ms: 1_700_000_000,
            }
        );
    }

    #[test]
    fn parse_lock_metadata_supports_legacy_pid_only_format() {
        let parsed = parse_lock_metadata("42\n").expect("parse metadata");
        assert_eq!(
            parsed,
            LockMetadata {
                pid: 42,
                created_unix_ms: 0,
            }
        );
    }

    #[test]
    fn unknown_owner_reap_policy_uses_age_threshold() {
        let threshold_ms = u64::try_from(THRESHOLD.as_millis()).unwrap_or(u64::MAX);
        assert!(!should_reap_lock(
            LockOwnerStatus::Unknown,
            Some(1_000),
            1_000 + threshold_ms.saturating_sub(1),
            THRESHOLD
        ));
        assert!(should_reap_lock(
            LockOwnerStatus::Unknown,
            Some(1_000),
            1_000 + threshold_ms,
            THRESHOLD
        ));
    }

    #[test]
    fn dead_owner_lock_is_reaped_immediately() {
        assert!(should_reap_lock(
            LockOwnerStatus::Dead,
            None,
            1_000,
            THRESHOLD
        ));
    }

    #[test]
    fn unknown_owner_without_timestamp_is_not_reaped_without_age_signal() {
        assert!(!should_reap_lock(
            LockOwnerStatus::Unknown,
            None,
            1_000,
            THRESHOLD
        ));
    }
}
