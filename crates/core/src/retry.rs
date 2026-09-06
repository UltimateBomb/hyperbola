//! When a failed download is worth another attempt.
//!
//! This is a rule, not plumbing, so it lives with the other rules: both
//! shells must agree on it, or the same failure would be retried three times
//! on one platform and reported immediately on the other.

/// Whether a failure is worth another attempt.
///
/// Network trouble is: a reset connection or a timeout usually succeeds on the
/// next try. A video that no longer exists is not — retrying that wastes the
/// user's time and buries the real reason under three identical errors.
pub fn is_retryable(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    const PERMANENT: [&str; 9] = [
        "video unavailable",
        "private video",
        "removed by the uploader",
        "has been terminated",
        "members-only",
        "is not available in your country",
        "requested format is not available",
        "unsupported url",
        "404",
    ];
    !PERMANENT.iter().any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::is_retryable;

    #[test]
    fn network_failures_are_retried() {
        assert!(is_retryable("[download] Connection reset by peer"));
        assert!(is_retryable(
            "Unable to download webpage: The read operation timed out"
        ));
        assert!(is_retryable("HTTP Error 503: Service Unavailable"));
    }

    #[test]
    fn gone_or_impossible_downloads_are_not_retried() {
        assert!(!is_retryable("[youtube] abc: Video unavailable"));
        assert!(!is_retryable(
            "ERROR: Private video. Sign in if you've been granted access"
        ));
        assert!(!is_retryable("Requested format is not available"));
        assert!(!is_retryable("HTTP Error 404: Not Found"));
    }

    #[test]
    fn the_check_is_case_insensitive() {
        assert!(!is_retryable("VIDEO UNAVAILABLE"));
    }
}
