//! Ollama cloud subscription/quota fetcher.
//!
//! Scrapes `https://ollama.com/settings/billing` for plan info
//! and `https://ollama.com/settings` for session/weekly usage meters.
//! Authenticates via `OLLAMA_AUTH_COOKIE` env var (full Cookie header value).

use super::types::*;
use crate::pricing;
use chrono::DateTime;
use reqwest::Client;
use scraper::{Html, Selector};
use tracing::{info, warn};

// ─── Constants ───────────────────────────────────────────────────────────────

const OLLAMA_BASE_URL: &str = "https://ollama.com";
const OLLAMA_TIMEOUT_SECS: u64 = 15;

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Read `OLLAMA_AUTH_COOKIE` from environment.
/// Should contain the full Cookie header value, e.g.
/// "aid=...; __Secure-session=..."
pub fn get_auth_cookie() -> Option<String> {
    std::env::var("OLLAMA_AUTH_COOKIE")
        .ok()
        .filter(|c| !c.is_empty())
}

// ─── Fetch functions ─────────────────────────────────────────────────────────

/// Fetch Ollama subscription and usage info.
pub async fn fetch_ollama_quota(client: &Client) -> OllamaQuotaStatus {
    let cookie = match get_auth_cookie() {
        Some(c) => c,
        None => {
            warn!("OLLAMA_AUTH_COOKIE not set");
            return OllamaQuotaStatus {
                available: false,
                data: None,
                error: Some("OLLAMA_AUTH_COOKIE not set".to_string()),
            };
        }
    };

    // Fetch billing and settings pages in parallel
    let (billing_result, settings_result) = tokio::join!(
        fetch_billing_page(client, &cookie),
        fetch_settings_page(client, &cookie),
    );

    let billing_data = match billing_result {
        Ok(data) => data,
        Err(e) => {
            warn!("Ollama billing fetch failed: {e}");
            return OllamaQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to fetch billing info: {e}")),
            };
        }
    };

    let usage_entries = match settings_result {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Ollama settings fetch failed: {e}");
            billing_data.usage_entries // at least show billing info
        }
    };

    let price = billing_data.price.clone();

    // Compute cost estimation from weekly usage percentage and empirical pricing
    let pricing_cfg = pricing::get_config();
    let per_token = pricing_cfg.special.ollama_cloud_empirical_per_token;
    let weekly_quota = pricing_cfg.special.ollama_cloud_empirical_weekly_quota;

    let (estimated_tokens_used, estimated_cost_cny) = if per_token > 0.0 && weekly_quota > 0 {
        // Find the weekly usage entry to get the current percentage
        let weekly_pct = usage_entries
            .iter()
            .find(|e| e.usage_type == "Weekly")
            .map(|e| e.percentage)
            .unwrap_or(0.0);

        let tokens = (weekly_quota as f64 * weekly_pct / 100.0).round() as i64;
        let cost = tokens as f64 * per_token;
        (Some(tokens), Some(cost))
    } else {
        (None, None)
    };

    info!("Ollama quota fetched");

    OllamaQuotaStatus {
        available: true,
        data: Some(OllamaQuotaData {
            plan_name: billing_data.plan_name,
            renews_on: billing_data.renews_on,
            price,
            usage_entries,
            has_annual_option: billing_data.has_annual_option,
            has_max_upgrade: billing_data.has_max_upgrade,
            estimated_tokens_used,
            estimated_cost_cny,
        }),
        error: None,
    }
}

// ─── Billing page parser ─────────────────────────────────────────────────────

struct BillingData {
    plan_name: String,
    renews_on: Option<String>,
    price: Option<String>,
    has_annual_option: bool,
    has_max_upgrade: bool,
    usage_entries: Vec<OllamaUsageEntry>,
}

async fn fetch_billing_page(client: &Client, cookie: &str) -> Result<BillingData, String> {
    let html = fetch_page(client, cookie, "/settings/billing").await?;
    parse_billing_page(&html)
}

fn parse_billing_page(html: &str) -> Result<BillingData, String> {
    let document = Html::parse_document(html);

    // Extract plan name — look for "Current Plan: Pro" or "Current Plan: Max"
    let plan_name =
        extract_text_after(&document, "Current Plan:").unwrap_or_else(|| "Unknown".to_string());

    // Extract renewal date — "Your subscription renews on\nJuly 26, 2026."
    let renews_on = extract_text_after(&document, "renews on")
        .map(|s| s.trim().trim_end_matches('.').trim().to_string())
        .filter(|s| !s.is_empty());

    // Price: "Paid\n$20.00" in the invoices table
    let price = extract_invoice_price(&document);

    // Check for "Change to annual billing" link
    let has_annual = html.contains("Change to annual billing");

    // Check for "Upgrade to Max" link
    let has_max = html.contains("Upgrade to Max");

    Ok(BillingData {
        plan_name,
        renews_on,
        price,
        has_annual_option: has_annual,
        has_max_upgrade: has_max,
        usage_entries: Vec::new(),
    })
}

// ─── Settings page parser ────────────────────────────────────────────────────

async fn fetch_settings_page(
    client: &Client,
    cookie: &str,
) -> Result<Vec<OllamaUsageEntry>, String> {
    let html = fetch_page(client, cookie, "/settings").await?;
    Ok(parse_usage_from_html(&html))
}

/// Parse usage entries from the settings page.
/// Looks for "Session usage" and "Weekly usage" sections with percentage + reset time.
fn parse_usage_from_html(html: &str) -> Vec<OllamaUsageEntry> {
    let document = Html::parse_document(html);
    let mut entries = Vec::new();

    // Collect all .local-time elements in document order
    let local_times: Vec<String> = {
        let selector = Selector::parse(".local-time").expect("hardcoded selector");
        document
            .root_element()
            .select(&selector)
            .filter_map(|el| {
                let t = el.value().attr("data-time")?;
                if DateTime::parse_from_rfc3339(t).is_ok() {
                    Some(t.to_string())
                } else {
                    None
                }
            })
            .collect()
    };

    // Find usage sections in document order
    let root = document.root_element();
    let text: String = root.text().collect();
    let text_lower = text.to_lowercase();

    let usage_types = ["Session", "Weekly"];
    let mut time_idx = 0usize;

    for usage_type in &usage_types {
        let label = format!("{} usage", usage_type);
        let label_lower = label.to_lowercase();

        if !text_lower.contains(&label_lower) {
            continue;
        }

        let pct = extract_usage_percentage(&document, usage_type);

        if let Some(pct) = pct {
            let reset_time = local_times.get(time_idx).cloned();
            if reset_time.is_some() {
                time_idx += 1;
            }

            entries.push(OllamaUsageEntry {
                usage_type: usage_type.to_string(),
                percentage: pct,
                reset_time,
            });
        }
    }

    entries
}

// ─── Extraction helpers ─────────────────────────────────────────────────────

/// Extract text that appears after a label in the page. Uses a simple text search
/// through the flattened document text.
fn extract_text_after(document: &Html, label: &str) -> Option<String> {
    // Walk the document root and collect text nodes
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join("");

    // Find the label position
    let label_lower = label.to_lowercase();
    let text_lower = text.to_lowercase();
    let label_pos = text_lower.find(&label_lower)?;
    let after = &text[label_pos + label.len()..];

    // Take the first meaningful line or phrase after the label
    let result = after.trim().lines().next().unwrap_or("").trim().to_string();

    // Clean up: remove trailing dots and extra whitespace
    let result = result.trim_end_matches('.').trim().to_string();

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Extract the invoice price from the billing page.
/// Looks for a "$" price near "Paid" in the invoice table.
fn extract_invoice_price(document: &Html) -> Option<String> {
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join(" ");

    // Find "Paid" and look backwards for a $XX.XX pattern in the same vicinity
    let paid_pos = text.rfind("Paid")?;

    // Search backwards within ~100 chars before "Paid"
    let search_start = paid_pos.saturating_sub(100);
    let before = &text[search_start..paid_pos];

    // Find the last $XX.XX pattern before "Paid"
    let dollar_pos = before.rfind('$')?;
    let dollar_str = &before[dollar_pos..];
    let end = dollar_str[1..]
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
        .map(|(i, _)| i + 1)
        .unwrap_or(dollar_str.len());

    let price_str = &dollar_str[..end];
    if price_str.len() > 1 {
        Some(price_str.to_string())
    } else {
        None
    }
}

/// Extract usage percentage for a given usage type (Session or Weekly).
/// Handles both integer ("100%") and float ("19.2%") formats.
fn extract_usage_percentage(document: &Html, usage_type: &str) -> Option<f64> {
    let root = document.root_element();
    let text = root.text().collect::<Vec<_>>().join("");

    // Find the usage type label
    let label = format!("{} usage", usage_type);
    let text_lower = text.to_lowercase();
    let label_pos = text_lower.find(&label.to_lowercase())?;

    // Look for "X% used" after the label — match digits, dots, then "%"
    let after = &text[label_pos + label.len()..];
    let pct_start = after.find(|c: char| c.is_ascii_digit())?;
    let pct_str = after[pct_start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>();

    pct_str.parse::<f64>().ok()
}

// ─── Generic helpers ─────────────────────────────────────────────────────────

/// Fetch a page from ollama.com with authentication cookies.
async fn fetch_page(client: &Client, cookie: &str, path: &str) -> Result<String, String> {
    let url = format!("{}{}", OLLAMA_BASE_URL, path);

    let response = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header("Cookie", cookie)
        .timeout(std::time::Duration::from_secs(OLLAMA_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    response
        .text()
        .await
        .map_err(|e| format!("Read error: {e}"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_auth_cookie() {
        temp_env::with_var(
            "OLLAMA_AUTH_COOKIE",
            Some("aid=abc; __Secure-session=xyz"),
            || {
                assert_eq!(
                    get_auth_cookie(),
                    Some("aid=abc; __Secure-session=xyz".to_string())
                );
            },
        );
    }

    #[test]
    fn test_get_auth_cookie_unset() {
        temp_env::with_var("OLLAMA_AUTH_COOKIE", None::<&str>, || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_get_auth_cookie_empty() {
        temp_env::with_var("OLLAMA_AUTH_COOKIE", Some(""), || {
            assert_eq!(get_auth_cookie(), None);
        });
    }

    #[test]
    fn test_parse_billing_page() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
            <div>Your subscription renews on July 26, 2026.</div>
            <a href="/upgrade">Upgrade to Max</a>
            <a href="/billing/annual">Change to annual billing</a>
            <table>
                <tr><td>June 26, 2026</td><td>-</td><td>$20.00</td><td>Paid</td></tr>
            </table>
        </body></html>"#;

        let result = parse_billing_page(html).unwrap();
        assert_eq!(result.plan_name, "Pro");
        assert_eq!(result.renews_on, Some("July 26, 2026".to_string()));
        assert_eq!(result.price, Some("$20.00".to_string()));
        assert!(result.has_annual_option);
        assert!(result.has_max_upgrade);
    }

    #[test]
    fn test_parse_billing_page_minimal() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
        </body></html>"#;

        let result = parse_billing_page(html).unwrap();
        assert_eq!(result.plan_name, "Pro");
        assert!(result.renews_on.is_none());
        assert!(result.price.is_none());
    }

    #[test]
    fn test_parse_usage_from_html() {
        let html = r#"<html><body>
            <div>
                <span>Session usage</span>
                <span>0% used</span>
                <div class="local-time" data-time="2026-06-26T05:00:00Z">Resets in 3 hours.</div>
            </div>
            <div>
                <span>Weekly usage</span>
                <span>10% used</span>
                <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 3 days.</div>
            </div>
        </body></html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[0].percentage, 0.0);
        assert_eq!(
            entries[0].reset_time.as_deref(),
            Some("2026-06-26T05:00:00Z")
        );

        assert_eq!(entries[1].usage_type, "Weekly");
        assert_eq!(entries[1].percentage, 10.0);
        assert_eq!(
            entries[1].reset_time.as_deref(),
            Some("2026-06-29T00:00:00Z")
        );
    }

    #[test]
    fn test_parse_usage_from_html_no_data() {
        let html = r#"<html><body><p>No usage here</p></body></html>"#;
        let entries = parse_usage_from_html(html);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_extract_usage_percentage() {
        let html = r#"<html><body>
            <span>Session usage</span>
            <span>42% used</span>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let pct = extract_usage_percentage(&doc, "Session");
        assert_eq!(pct, Some(42.0));
    }

    #[test]
    fn test_extract_usage_percentage_not_found() {
        let html = r#"<html><body><p>Nothing</p></body></html>"#;
        let doc = Html::parse_document(html);
        let pct = extract_usage_percentage(&doc, "Session");
        assert!(pct.is_none());
    }

    #[test]
    fn test_extract_invoice_price() {
        let html = r#"<html><body>
            <table><tr><td>June 26, 2026</td><td>-</td><td>$20.00</td><td>Paid</td></tr></table>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let price = extract_invoice_price(&doc);
        assert_eq!(price, Some("$20.00".to_string()));
    }

    #[test]
    fn test_extract_text_after() {
        let html = r#"<html><body>
            <div>Current Plan: Pro</div>
            <div>Your subscription renews on July 26, 2026.</div>
        </body></html>"#;
        let doc = Html::parse_document(html);
        let plan = extract_text_after(&doc, "Current Plan:");
        assert_eq!(plan, Some("Pro".to_string()));
        let renew = extract_text_after(&doc, "renews on");
        assert_eq!(renew, Some("July 26, 2026".to_string()));
    }

    // ── Realistic integration-style tests ──────────────────────────────────────

    #[test]
    fn test_parse_usage_from_html_realistic() {
        // Simulates the actual ollama.com/settings HTML structure
        let html = r#"<html>
            <head><title>Usage · Settings</title></head>
            <body>
                <h2><span>Cloud usage</span><span>pro</span></h2>
                <p>Cloud models and capabilities such as web search contribute to session and weekly limits.</p>
                <div>
                    <div><span>Session usage</span><span>0% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-26T05:00:00Z">Resets in 3 hours.</div>
                    </div>
                </div>
                <div>
                    <div><span>Weekly usage</span><span>0% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 3 days.</div>
                    </div>
                </div>
            </body>
        </html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[1].usage_type, "Weekly");
    }

    #[test]
    fn test_parse_usage_from_html_float_percentage() {
        // Test float percentage parsing (e.g. "19.2% used")
        let html = r#"<html>
            <head><title>Usage · Settings</title></head>
            <body>
                <h2><span>Cloud usage</span><span>pro</span></h2>
                <div>
                    <div><span>Session usage</span><span>100% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-26T10:00:00Z">Resets in 6 minutes.</div>
                    </div>
                </div>
                <div>
                    <div><span>Weekly usage</span><span>19.2% used</span></div>
                    <div>
                        <div class="local-time" data-time="2026-06-29T00:00:00Z">Resets in 2 days.</div>
                    </div>
                </div>
            </body>
        </html>"#;

        let entries = parse_usage_from_html(html);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].usage_type, "Session");
        assert_eq!(entries[0].percentage, 100.0);
        assert_eq!(entries[1].usage_type, "Weekly");
        assert!((entries[1].percentage - 19.2).abs() < 0.01);
    }
}
