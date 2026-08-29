//! Classification cache keys (spec §33): the same activity in the same
//! commitment context must not hit the AI twice.

use crate::aggregator::normalize_title;

/// Lowercase, strip "www.".
pub fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_start_matches("www.").to_lowercase()
}

/// Stable cache key for a (commitment, activity) pair.
pub fn cache_key(
    commitment_id: Option<i64>,
    process_name: &str,
    browser_domain: Option<&str>,
    window_title: &str,
) -> String {
    format!(
        "c{}|p{}|d{}|t{}",
        commitment_id.map_or_else(|| "-".into(), |id| id.to_string()),
        process_name.to_lowercase(),
        browser_domain.map(normalize_domain).unwrap_or_default(),
        normalize_title(window_title).to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_across_cosmetic_title_changes() {
        let a = cache_key(Some(1), "chrome.exe", Some("Docs.Google.com"), "(2) Playbook - Google Docs");
        let b = cache_key(Some(1), "CHROME.EXE", Some("docs.google.com"), "Playbook - Google Docs");
        assert_eq!(a, b);
    }

    #[test]
    fn key_differs_across_commitments() {
        let a = cache_key(Some(1), "chrome.exe", None, "Playbook");
        let b = cache_key(Some(2), "chrome.exe", None, "Playbook");
        assert_ne!(a, b);
    }
}
