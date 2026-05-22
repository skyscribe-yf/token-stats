//! OpenCode-go provider integration.
//!
//! Fetches subscription usage data directly from the OpenCode workspace dashboard
//! using native Rust HTTP + HTML parsing. No Python subprocess dependency.

use super::types::*;
use chrono::{Duration, Utc};
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::{info, warn};

/// Parse a human-readable duration string like "4 days 16 hours", "1 day",
/// "4 hours 4 minutes", "30 minutes" into a `chrono::Duration`.
///
/// Returns `None` if the string cannot be parsed.
fn parse_resets_in(resets_in: &str) -> Option<Duration> {
    let mut total = Duration::zero();
    let mut found = false;

    // Split on whitespace and process pairs of (number, unit)
    let tokens: Vec<&str> = resets_in.split_whitespace().collect();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if let Ok(value) = tokens[i].parse::<i64>() {
            let unit = tokens[i + 1].to_lowercase();
            if unit.starts_with("day") {
                total = total + Duration::days(value);
                found = true;
                i += 2;
            } else if unit.starts_with("hour") {
                total = total + Duration::hours(value);
                found = true;
                i += 2;
            } else if unit.starts_with("minute") {
                total = total + Duration::minutes(value);
                found = true;
                i += 2;
            } else if unit.starts_with("second") {
                total = total + Duration::seconds(value);
                found = true;
                i += 2;
            } else {
                // Unknown unit — skip this token and continue
                i += 1;
            }
        } else {
            i += 1;
        }
    }

    if found { Some(total) } else { None }
}

// ─── Constants ───────────────────────────────────────────────────────────────

const OPENCODE_BASE_URL: &str = "https://opencode.ai";
const OPENCODE_TIMEOUT_SECS: u64 = 15;

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Read `OPENCODE_GO_WORKSPACE_ID` from environment.
pub fn get_workspace_id() -> Option<String> {
    std::env::var("OPENCODE_GO_WORKSPACE_ID")
        .ok()
        .filter(|id| !id.is_empty())
}

/// Read `OPENCODE_GO_WORKSPACE_ID_EX` from environment.
pub fn get_workspace_id_ex() -> Option<String> {
    std::env::var("OPENCODE_GO_WORKSPACE_ID_EX")
        .ok()
        .filter(|id| !id.is_empty())
}

/// Read `OPENCODE_GO_AUTH_COOKIE` from environment.
pub fn get_auth_cookie() -> Option<String> {
    std::env::var("OPENCODE_GO_AUTH_COOKIE")
        .ok()
        .filter(|c| !c.is_empty())
}

/// Read `OPENCODE_GO_AUTH_COOKIE_EX` from environment.
pub fn get_auth_cookie_ex() -> Option<String> {
    std::env::var("OPENCODE_GO_AUTH_COOKIE_EX")
        .ok()
        .filter(|c| !c.is_empty())
}

/// Build the OpenCode-go workspace dashboard URL from the workspace ID env var.
pub fn get_opencode_workspace_url() -> Option<String> {
    get_workspace_id().map(|id| format!("{}/workspace/{}/go", OPENCODE_BASE_URL, id))
}

/// Build the workspace usage URL for a given workspace ID.
fn build_usage_url(workspace_id: &str) -> String {
    format!("{}/workspace/{}/go", OPENCODE_BASE_URL, workspace_id)
}

// ─── OpenCode Quota Fetching ─────────────────────────────────────────────────

/// Fetch OpenCode-go subscription usage for the **primary** workspace.
pub async fn fetch_opencode_quota(client: &Client) -> OpenCodeQuotaStatus {
    let workspace_id = match get_workspace_id() {
        Some(id) => id,
        None => {
            warn!("OPENCODE_GO_WORKSPACE_ID not set");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("OPENCODE_GO_WORKSPACE_ID not set".to_string()),
            };
        }
    };

    let auth_cookie = match get_auth_cookie() {
        Some(c) => c,
        None => {
            warn!("OPENCODE_GO_AUTH_COOKIE not set");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("OPENCODE_GO_AUTH_COOKIE not set".to_string()),
            };
        }
    };

    fetch_opencode_quota_impl(
        client,
        &workspace_id,
        &auth_cookie,
        get_opencode_workspace_url(),
    )
    .await
}

/// Fetch OpenCode-go subscription usage for the **EX** workspace.
pub async fn fetch_opencode_quota_ex(client: &Client) -> OpenCodeQuotaStatus {
    let workspace_id = match get_workspace_id_ex() {
        Some(id) => id,
        None => {
            warn!("OPENCODE_GO_WORKSPACE_ID_EX not set");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("OPENCODE_GO_WORKSPACE_ID_EX not set".to_string()),
            };
        }
    };

    let auth_cookie = match get_auth_cookie_ex() {
        Some(c) => c,
        None => {
            warn!("OPENCODE_GO_AUTH_COOKIE_EX not set");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("OPENCODE_GO_AUTH_COOKIE_EX not set".to_string()),
            };
        }
    };

    let workspace_url = Some(format!(
        "{}/workspace/{}/go",
        OPENCODE_BASE_URL, workspace_id
    ));
    fetch_opencode_quota_impl(client, &workspace_id, &auth_cookie, workspace_url).await
}

/// Core fetch+parse logic shared by both primary and EX workspace fetchers.
async fn fetch_opencode_quota_impl(
    client: &Client,
    workspace_id: &str,
    auth_cookie: &str,
    workspace_url: Option<String>,
) -> OpenCodeQuotaStatus {
    let url = build_usage_url(workspace_id);

    let response = match client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header("Cookie", format!("auth={}; oc_locale=en", auth_cookie))
        .timeout(std::time::Duration::from_secs(OPENCODE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            warn!("OpenCode quota fetch failed: network error");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("Failed to fetch OpenCode page (network error)".to_string()),
            };
        }
    };

    if !response.status().is_success() {
        warn!("OpenCode returned HTTP {}", response.status());
        return OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some(format!("OpenCode returned HTTP {}", response.status())),
        };
    }

    let html = match response.text().await {
        Ok(t) => t,
        Err(_) => {
            warn!("Failed to read OpenCode response body");
            return OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some("Failed to read OpenCode response body".to_string()),
            };
        }
    };

    // Step 3: Parse HTML and extract usage
    let entries = parse_usage_from_html(&html);

    if entries.is_empty() {
        warn!("OpenCode usage data not found — may require authentication");
        return OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some(
                "Could not find usage data — page may require authentication or workspace is invalid"
                    .to_string(),
            ),
        };
    }

    info!("OpenCode quota fetched ({} entries)", entries.len());

    OpenCodeQuotaStatus {
        available: true,
        data: Some(QuotaOpenCode {
            provider: "opencode-go".to_string(),
            entries,
            workspace_url,
        }),
        error: None,
    }
}

// ─── HTML Parsing ────────────────────────────────────────────────────────────

/// Parse usage entries from raw HTML.
///
/// Looks for a `<div data-slot="usage">` element, extracts its text content,
/// and parses Rolling/Weekly/Monthly usage percentages + reset timers.
pub(crate) fn parse_usage_from_html(html: &str) -> Vec<QuotaOpenCodeUsageEntry> {
    let document = Html::parse_document(html);

    let selector = Selector::parse("div[data-slot=\"usage\"]")
        .expect("hardcoded CSS selector is always valid");

    let usage_div = match document.select(&selector).next() {
        Some(el) => el,
        None => return Vec::new(),
    };

    let usage_text = usage_div.text().collect::<Vec<_>>().concat();
    extract_usage_entries(&usage_text)
}

/// Extract usage entries from flattened text using a lightweight hand-rolled parser.
///
/// The Python tool's regex pattern is:
/// `(Rolling|Weekly|Monthly) Usage(\d+%)Resets in(.*?)`
/// This parser mirrors that behaviour without pulling in the `regex` crate.
fn extract_usage_entries(text: &str) -> Vec<QuotaOpenCodeUsageEntry> {
    let mut entries = Vec::new();
    let text = text.trim();

    for keyword in &["Rolling", "Weekly", "Monthly"] {
        if let Some(pos) = text.find(keyword) {
            let remainder = &text[pos + keyword.len()..];

            // Look for the literal "Usage" token
            if let Some(usage_pos) = remainder.find("Usage") {
                let after_usage = remainder[usage_pos + "Usage".len()..].trim_start();

                // Read numeric digits = percentage
                let pct_digits: String = after_usage
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if pct_digits.is_empty() {
                    continue;
                }
                let percentage: i32 = match pct_digits.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                let after_pct = &after_usage[pct_digits.len()..];

                // Look for the literal "Resets in" token
                if let Some(resets_pos) = after_pct.find("Resets in") {
                    let reset_text = &after_pct[resets_pos + "Resets in".len()..];

                    // Read until the next usage keyword or end of string
                    let resets_in = read_until_next_keyword(reset_text);
                    let reset_at = parse_resets_in(&resets_in)
                        .map(|dur| (Utc::now() + dur).to_rfc3339());

                    entries.push(QuotaOpenCodeUsageEntry {
                        usage_type: keyword.to_string(),
                        percentage,
                        resets_in,
                        reset_at,
                    });
                }
            }
        }
    }

    entries
}

/// Read from `text` until we encounter the start of another usage keyword
/// ("Rolling", "Weekly", "Monthly"). Returns the accumulated text trimmed.
fn read_until_next_keyword(text: &str) -> String {
    let mut result = String::new();
    let keywords = &["Rolling", "Weekly", "Monthly"];

    // We scan character-by-character. If at any position we match one of the
    // keywords, we stop before that keyword.
    let mut i = 0;
    while i < text.len() {
        let slice = &text[i..];
        if keywords.iter().any(|k| slice.starts_with(k)) {
            break;
        }
        // Advance one UTF-8 character
        if let Some(ch) = slice.chars().next() {
            result.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }

    result.trim().to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_opencode_workspace_url() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", Some("wrk_TEST123"), || {
            assert_eq!(
                get_opencode_workspace_url(),
                Some("https://opencode.ai/workspace/wrk_TEST123/go".to_string())
            );
        });
    }

    #[test]
    fn test_get_opencode_workspace_url_unset() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", None::<&str>, || {
            assert_eq!(get_opencode_workspace_url(), None);
        });
    }

    #[test]
    fn test_build_usage_url() {
        assert_eq!(
            build_usage_url("wrk_TEST123"),
            "https://opencode.ai/workspace/wrk_TEST123/go"
        );
    }

    #[test]
    fn test_get_workspace_id_from_env() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", Some("wrk_123"), || {
            assert_eq!(get_workspace_id(), Some("wrk_123".to_string()));
        });
    }

    #[test]
    fn test_get_workspace_id_unset() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID", None::<&str>, || {
            assert_eq!(get_workspace_id(), None);
        });
    }

    #[test]
    fn test_get_auth_cookie_from_env() {
        temp_env::with_var("OPENCODE_GO_AUTH_COOKIE", Some("tok_abc"), || {
            assert_eq!(get_auth_cookie(), Some("tok_abc".to_string()));
        });
    }

    #[test]
    fn test_get_auth_cookie_unset() {
        temp_env::with_var("OPENCODE_GO_AUTH_COOKIE", None::<&str>, || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_get_workspace_id_ex_from_env() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID_EX", Some("wrk_ex_123"), || {
            assert_eq!(get_workspace_id_ex(), Some("wrk_ex_123".to_string()));
        });
    }

    #[test]
    fn test_get_workspace_id_ex_unset() {
        temp_env::with_var("OPENCODE_GO_WORKSPACE_ID_EX", None::<&str>, || {
            assert_eq!(get_workspace_id_ex(), None);
        });
    }

    #[test]
    fn test_get_auth_cookie_ex_from_env() {
        temp_env::with_var("OPENCODE_GO_AUTH_COOKIE_EX", Some("tok_ex_abc"), || {
            assert_eq!(get_auth_cookie_ex(), Some("tok_ex_abc".to_string()));
        });
    }

    #[test]
    fn test_get_auth_cookie_ex_unset() {
        temp_env::with_var("OPENCODE_GO_AUTH_COOKIE_EX", None::<&str>, || {
            assert_eq!(get_auth_cookie_ex(), None);
        });
    }

    #[test]
    fn test_parse_usage_from_html_with_data() {
        let html = r#"<html><body><div data-slot="usage">Rolling Usage0%Resets in4 hours 4 minutesWeekly Usage3%Resets in4 days 16 hoursMonthly Usage63%Resets in21 days 1 hour</div></body></html>"#;
        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].usage_type, "Rolling");
        assert_eq!(entries[0].percentage, 0);
        assert_eq!(entries[0].resets_in, "4 hours 4 minutes");

        assert_eq!(entries[1].usage_type, "Weekly");
        assert_eq!(entries[1].percentage, 3);
        assert_eq!(entries[1].resets_in, "4 days 16 hours");

        assert_eq!(entries[2].usage_type, "Monthly");
        assert_eq!(entries[2].percentage, 63);
        assert_eq!(entries[2].resets_in, "21 days 1 hour");
    }

    #[test]
    fn test_parse_usage_from_html_no_div() {
        let html = r#"<html><body><p>No usage here</p></body></html>"#;
        let entries = parse_usage_from_html(html);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_usage_from_html_unauthenticated() {
        // When not authenticated, the page shows a login link instead of usage div
        let html = r#"<html><body><a href="/auth">Continue with Google</a></body></html>"#;
        let entries = parse_usage_from_html(html);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_usage_from_html_realistic() {
        // More realistic HTML structure
        let html = r#"<div data-slot="usage">
            <div class="flex gap-4">
                <div>Rolling Usage</div>
                <div>0%</div>
                <div>Resets in 4 hours 4 minutes</div>
            </div>
            <div class="flex gap-4">
                <div>Weekly Usage</div>
                <div>3%</div>
                <div>Resets in 4 days 16 hours</div>
            </div>
            <div class="flex gap-4">
                <div>Monthly Usage</div>
                <div>63%</div>
                <div>Resets in 21 days 1 hour</div>
            </div>
        </div>"#;
        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].percentage, 0);
        assert_eq!(entries[1].percentage, 3);
        assert_eq!(entries[2].percentage, 63);
    }

    #[test]
    fn test_extract_usage_entries() {
        let text = "Rolling Usage0%Resets in4 hours 4 minutesWeekly Usage3%Resets in4 days 16 hoursMonthly Usage63%Resets in21 days 1 hour";
        let entries = extract_usage_entries(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].usage_type, "Rolling");
        assert_eq!(entries[0].percentage, 0);
        assert_eq!(entries[0].resets_in, "4 hours 4 minutes");
        assert_eq!(entries[1].usage_type, "Weekly");
        assert_eq!(entries[1].percentage, 3);
        assert_eq!(entries[1].resets_in, "4 days 16 hours");
        assert_eq!(entries[2].usage_type, "Monthly");
        assert_eq!(entries[2].percentage, 63);
        assert_eq!(entries[2].resets_in, "21 days 1 hour");
    }

    #[test]
    fn test_extract_usage_entries_empty() {
        let entries = extract_usage_entries("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_extract_usage_entries_no_resets_in() {
        // Missing "Resets in" — parser should skip
        let text = "Rolling Usage50%Unknown text";
        let entries = extract_usage_entries(text);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_extract_usage_entries_single() {
        let text = "Monthly Usage95%Resets in5 days 3 hours";
        let entries = extract_usage_entries(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].usage_type, "Monthly");
        assert_eq!(entries[0].percentage, 95);
        assert_eq!(entries[0].resets_in, "5 days 3 hours");
    }

    #[test]
    fn test_read_until_next_keyword() {
        assert_eq!(
            read_until_next_keyword("4 hours 4 minutes"),
            "4 hours 4 minutes"
        );
        assert_eq!(
            read_until_next_keyword("4 hours 4 minutesWeekly Usage3%"),
            "4 hours 4 minutes"
        );
    }

    // ── parse_resets_in tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_resets_in_days_and_hours() {
        let dur = parse_resets_in("4 days 16 hours").unwrap();
        assert_eq!(dur.num_days(), 4);
        assert_eq!(dur.num_hours(), 4 * 24 + 16);
    }

    #[test]
    fn test_parse_resets_in_hours_and_minutes() {
        let dur = parse_resets_in("4 hours 4 minutes").unwrap();
        assert_eq!(dur.num_minutes(), 4 * 60 + 4);
    }

    #[test]
    fn test_parse_resets_in_single_day() {
        let dur = parse_resets_in("1 day").unwrap();
        assert_eq!(dur.num_days(), 1);
    }

    #[test]
    fn test_parse_resets_in_single_hour() {
        let dur = parse_resets_in("1 hour").unwrap();
        assert_eq!(dur.num_hours(), 1);
    }

    #[test]
    fn test_parse_resets_in_minutes() {
        let dur = parse_resets_in("30 minutes").unwrap();
        assert_eq!(dur.num_minutes(), 30);
    }

    #[test]
    fn test_parse_resets_in_days_and_hours_singular() {
        let dur = parse_resets_in("21 days 1 hour").unwrap();
        assert_eq!(dur.num_hours(), 21 * 24 + 1);
    }

    #[test]
    fn test_parse_resets_in_empty() {
        assert!(parse_resets_in("").is_none());
    }

    #[test]
    fn test_parse_resets_in_garbage() {
        assert!(parse_resets_in("some random text").is_none());
    }

    #[test]
    fn test_extract_usage_entries_has_reset_at() {
        let text = "Monthly Usage95%Resets in5 days 3 hours";
        let entries = extract_usage_entries(text);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].reset_at.is_some());
        // reset_at should be a valid RFC3339 timestamp
        let parsed = chrono::DateTime::parse_from_rfc3339(entries[0].reset_at.as_ref().unwrap());
        assert!(parsed.is_ok());
    }

    // ── Async integration tests ──────────────────────────────────────────────

    /// Mutex to prevent concurrent env var manipulation across async tests.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_env_var(name: &str, old: Option<String>) {
        match old {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_missing_workspace_id() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        let old_cookie = std::env::var("OPENCODE_GO_AUTH_COOKIE").ok();
        std::env::remove_var("OPENCODE_GO_WORKSPACE_ID");
        std::env::remove_var("OPENCODE_GO_AUTH_COOKIE");

        let client = Client::new();
        let result = fetch_opencode_quota(&client).await;
        assert!(!result.available);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("OPENCODE_GO_WORKSPACE_ID"));

        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);
        reset_env_var("OPENCODE_GO_AUTH_COOKIE", old_cookie);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_fetch_opencode_quota_missing_auth_cookie() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let old_ws = std::env::var("OPENCODE_GO_WORKSPACE_ID").ok();
        let old_cookie = std::env::var("OPENCODE_GO_AUTH_COOKIE").ok();
        std::env::set_var("OPENCODE_GO_WORKSPACE_ID", "wrk_test");
        std::env::remove_var("OPENCODE_GO_AUTH_COOKIE");

        let client = Client::new();
        let result = fetch_opencode_quota(&client).await;
        assert!(!result.available);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("OPENCODE_GO_AUTH_COOKIE"));

        reset_env_var("OPENCODE_GO_WORKSPACE_ID", old_ws);
        reset_env_var("OPENCODE_GO_AUTH_COOKIE", old_cookie);
    }

    #[test]
    fn test_parse_resets_in_includes_seconds() {
        let dur = parse_resets_in("5 minutes 30 seconds").unwrap();
        assert_eq!(dur.num_seconds(), 5 * 60 + 30);
    }
}
