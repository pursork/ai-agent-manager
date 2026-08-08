//! Small helpers shared across this crate's modules.

/// Current time as an RFC 3339 UTC timestamp (e.g. `2026-08-08T12:00:00Z`),
/// used for `devices.json`'s `added_at` and blob metadata's `updated_at`.
/// Deliberately UTC-only (not the local offset) -- `time`'s local-offset
/// retrieval is unsound on some platforms and is off by default; a blob's
/// `updated_at` is informational metadata, not something conflict
/// resolution depends on (`§4.6` uses the monotonic `version` for that), so
/// there's no reason to take on that risk for a friendlier-looking offset.
pub(crate) fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("Rfc3339 formatting of the current time cannot fail")
}
