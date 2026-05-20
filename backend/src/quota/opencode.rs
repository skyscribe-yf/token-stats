//! OpenCode-go provider integration.
//!
//! Uses the `opencode-usage` Python CLI tool to fetch subscription usage data.
//! Falls back gracefully when the tool is not installed or requires interactive auth.

use super::types::*;
use tokio::process::Command;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Timeout for the `opencode-usage` subprocess (seconds).
/// If the tool blocks for interactive input, we kill it after this duration.
const OPENCODE_USAGE_TIMEOUT_SECS: u64 = 10;

/// Name of the CLI tool on PATH.
const OPENCODE_USAGE_BIN: &str = "opencode-usage";

// ─── Auth helpers ────────────────────────────────────────────────────────────

/// Build the OpenCode-go workspace dashboard URL from the workspace ID env var.
pub fn get_opencode_workspace_url() -> Option<String> {
    std::env::var("OPENCODE_GO_WORKSPACE_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| format!("https://opencode.ai/workspace/{}/go", id))
}

// ─── OpenCode Quota Fetching ─────────────────────────────────────────────────

/// Fetch OpenCode-go subscription usage by running the `opencode-usage` CLI tool.
pub async fn fetch_opencode_quota() -> OpenCodeQuotaStatus {
    // Step 1: Check if the tool is installed
    if !is_opencode_usage_installed().await {
        return OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some(format!(
                "{} not installed. Install with: uv tool install opencode-usage",
                OPENCODE_USAGE_BIN
            )),
        };
    }

    // Step 2: Run the tool with a timeout
    let output = match run_opencode_usage().await {
        Ok(o) => o,
        Err(e) => return e,
    };

    // Step 3: Parse the table output
    // Strip any ANSI escape codes that Rich might emit
    let clean_output = strip_ansi_codes(&output);
    let entries = parse_usage_table(&clean_output);

    if entries.is_empty() {
        return OpenCodeQuotaStatus {
            available: false,
            data: None,
            error: Some("Could not parse usage data from opencode-usage output".to_string()),
        };
    }

    OpenCodeQuotaStatus {
        available: true,
        data: Some(QuotaOpenCode {
            provider: "opencode-go".to_string(),
            entries,
            workspace_url: get_opencode_workspace_url(),
        }),
        error: None,
    }
}

/// Check if `opencode-usage` is available on PATH.
async fn is_opencode_usage_installed() -> bool {
    // Try running with --help to see if the tool exists.
    // Using `which` is simpler but may not be available on all systems.
    match Command::new(OPENCODE_USAGE_BIN)
        .arg("--help")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .output()
        .await
    {
        Ok(output) => output.status.success() || !output.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Run `opencode-usage` with a timeout, capturing stdout.
/// Returns the stdout string on success, or an error status on failure.
async fn run_opencode_usage() -> Result<String, OpenCodeQuotaStatus> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(OPENCODE_USAGE_TIMEOUT_SECS),
        Command::new(OPENCODE_USAGE_BIN)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null()) // no stdin — prevents blocking on prompts
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            // Process completed within timeout
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if stdout.trim().is_empty() {
                    // Tool exited successfully but produced no output
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    return Err(OpenCodeQuotaStatus {
                        available: false,
                        data: None,
                        error: Some(if stderr.is_empty() {
                            "opencode-usage produced no output".to_string()
                        } else {
                            format!(
                                "opencode-usage error: {}",
                                super::truncate_error_body(&stderr)
                            )
                        }),
                    });
                }
                Ok(stdout)
            } else {
                // Non-zero exit code — likely auth failure or other error
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let error_msg = if stderr.contains("EOFError")
                    || stderr.contains("EOF when reading")
                {
                    "opencode-usage requires authentication. Run `opencode-usage` in terminal once to save credentials.".to_string()
                } else if stderr.is_empty() {
                    format!(
                        "opencode-usage exited with code {}",
                        output.status.code().unwrap_or(-1)
                    )
                } else {
                    format!(
                        "opencode-usage error: {}",
                        super::truncate_error_body(&stderr)
                    )
                };
                Err(OpenCodeQuotaStatus {
                    available: false,
                    data: None,
                    error: Some(error_msg),
                })
            }
        }
        Ok(Err(e)) => {
            // Failed to spawn the process
            Err(OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some(format!("Failed to run opencode-usage: {}", e)),
            })
        }
        Err(_) => {
            // Timeout — the tool is likely blocking for interactive input
            Err(OpenCodeQuotaStatus {
                available: false,
                data: None,
                error: Some(
                    "opencode-usage requires authentication. Run `opencode-usage` in terminal once to save credentials.".to_string(),
                ),
            })
        }
    }
}

/// Strip ANSI escape sequences from a string.
/// Rich may emit ANSI codes even when stdout is piped in some environments.
fn strip_ansi_codes(s: &str) -> String {
    // Simple regex-free approach: remove sequences like \x1b[...m
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip the escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Consume parameter bytes (0-9, ;, etc.)
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() || next == ';' || next == '?' {
                        chars.next();
                    } else if ('@'..='~').contains(&next) {
                        chars.next(); // final byte
                        break;
                    } else {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse the Rich-rendered table output from `opencode-usage`.
///
/// Expected format (plain text when stdout is not a TTY):
/// ```text
///              Account Usage Limits              
/// ┏━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┓
/// ┃ Usage Type   ┃ Capacity ┃ Reset Timer       ┃
/// ┡━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━┩
/// │ Rolling      │       0% │ 4 hours 4 minutes │
/// │ Weekly       │       3% │ 4 days 16 hours   │
/// │ Monthly      │      63% │ 21 days 1 hour    │
/// └──────────────┴──────────┴───────────────────┘
/// ```
fn parse_usage_table(output: &str) -> Vec<QuotaOpenCodeUsageEntry> {
    let mut entries = Vec::new();

    for line in output.lines() {
        // Data rows are delimited by │ (box-drawing light vertical)
        // and contain a percentage value
        if !line.contains('│') || !line.contains('%') {
            continue;
        }

        // Skip header rows (┃) and border rows
        if line.contains('┃') || line.contains('┏') || line.contains('┡') || line.contains('└')
        {
            continue;
        }

        if let Some(entry) = parse_usage_row(line) {
            entries.push(entry);
        }
    }

    entries
}

/// Parse a single data row like: `│ Rolling      │       0% │ 4 hours 4 minutes │`
fn parse_usage_row(line: &str) -> Option<QuotaOpenCodeUsageEntry> {
    // Split by │ and collect non-empty trimmed segments
    let fields: Vec<&str> = line
        .split('│')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if fields.len() != 3 {
        return None;
    }

    let usage_type = fields[0].to_string();
    let capacity_str = fields[1];
    let resets_in = fields[2].to_string();

    // Parse percentage: "0%", "3%", "63%"
    let percentage_str = capacity_str.trim_end_matches('%');
    let percentage: i32 = percentage_str.trim().parse().ok()?;

    Some(QuotaOpenCodeUsageEntry {
        usage_type,
        percentage,
        resets_in,
    })
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
    fn test_parse_usage_table_success() {
        let output = "\
             Account Usage Limits              
┏━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┓
┃ Usage Type   ┃ Capacity ┃ Reset Timer       ┃
┡━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━┩
│ Rolling      │       0% │ 4 hours 4 minutes │
│ Weekly       │       3% │ 4 days 16 hours   │
│ Monthly      │      63% │ 21 days 1 hour    │
└──────────────┴──────────┴───────────────────┘";

        let entries = parse_usage_table(output);
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
    fn test_parse_usage_table_empty() {
        let entries = parse_usage_table("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_usage_table_no_data_rows() {
        let output = "\
             Account Usage Limits              
┏━━━━━━━━━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━┓
┃ Usage Type   ┃ Capacity ┃ Reset Timer       ┃
┡━━━━━━━━━━━━━━╇━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━┩
└──────────────┴──────────┴───────────────────┘";
        let entries = parse_usage_table(output);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_usage_row() {
        let line = "│ Rolling      │       0% │ 4 hours 4 minutes │";
        let entry = parse_usage_row(line).unwrap();
        assert_eq!(entry.usage_type, "Rolling");
        assert_eq!(entry.percentage, 0);
        assert_eq!(entry.resets_in, "4 hours 4 minutes");
    }

    #[test]
    fn test_parse_usage_row_high_percentage() {
        let line = "│ Monthly      │      95% │ 5 days 3 hours    │";
        let entry = parse_usage_row(line).unwrap();
        assert_eq!(entry.usage_type, "Monthly");
        assert_eq!(entry.percentage, 95);
        assert_eq!(entry.resets_in, "5 days 3 hours");
    }

    #[test]
    fn test_parse_usage_row_invalid() {
        // Header row (uses ┃ not │)
        assert!(parse_usage_row("┃ Usage Type   ┃ Capacity ┃ Reset Timer       ┃").is_none());
        // Border row
        assert!(parse_usage_row("└──────────────┴──────────┴───────────────────┘").is_none());
        // Wrong number of fields
        assert!(parse_usage_row("│ Rolling      │       0%").is_none());
    }

    #[test]
    fn test_strip_ansi_codes() {
        // No ANSI codes
        assert_eq!(strip_ansi_codes("hello world"), "hello world");
        // Simple color code
        assert_eq!(strip_ansi_codes("\x1b[31mred text\x1b[0m"), "red text");
        // Multiple codes
        assert_eq!(
            strip_ansi_codes("\x1b[1;32mgreen\x1b[0m and \x1b[33myellow\x1b[0m"),
            "green and yellow"
        );
        // Empty string
        assert_eq!(strip_ansi_codes(""), "");
    }
}
