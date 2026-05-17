use crate::models::TokenRecord;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// A single vendor group entry from the TOML config.
#[derive(Debug, Deserialize)]
struct VendorGroup {
    /// The merged (canonical) vendor name.
    name: String,
    /// All original provider names that should map to this vendor.
    providers: Vec<String>,
}

/// Top-level TOML structure.
#[derive(Debug, Deserialize)]
struct VendorMergeConfig {
    vendor_group: Vec<VendorGroup>,
}

/// Build a provider → merged-name lookup table from a TOML config file.
///
/// Returns `None` if the file doesn't exist or is malformed (a warning is logged).
pub fn load_vendor_merge_map(path: &Path) -> Option<HashMap<String, String>> {
    if !path.exists() {
        tracing::info!("Vendor merge config not found at {:?}, skipping", path);
        return None;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read vendor merge config: {}", e);
            return None;
        }
    };

    let config: VendorMergeConfig = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to parse vendor merge config: {}", e);
            return None;
        }
    };

    let mut map: HashMap<String, String> = HashMap::new();
    for group in &config.vendor_group {
        for provider in &group.providers {
            map.insert(provider.clone(), group.name.clone());
        }
    }

    tracing::info!(
        "Loaded vendor merge config: {} mapping(s) from {:?}",
        map.len(),
        path
    );
    Some(map)
}

/// Return the default path for the vendor merge config file.
///
/// Looks for `VENDOR_MERGE_CONFIG` env var first, then falls back to
/// `vendor_merge.toml` in the same directory as the running binary,
/// and finally the current working directory.
pub fn get_vendor_merge_config_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("VENDOR_MERGE_CONFIG") {
        return std::path::PathBuf::from(p);
    }

    // Try next to the running binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("vendor_merge.toml");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    // Fallback: current working directory
    std::path::PathBuf::from("vendor_merge.toml")
}

/// Apply vendor merging to a batch of records in-place.
///
/// For each record whose `provider` matches a key in `merge_map`, the provider
/// is replaced with the mapped value. Records whose provider is not in the map
/// are left unchanged.
pub fn apply_vendor_merge(records: &mut [TokenRecord], merge_map: &HashMap<String, String>) {
    if merge_map.is_empty() {
        return;
    }

    let mut merged_count = 0usize;
    for record in records.iter_mut() {
        if let Some(target) = merge_map.get(&record.provider) {
            record.provider = target.clone();
            merged_count += 1;
        }
    }

    if merged_count > 0 {
        tracing::info!("Vendor merge: {} record(s) remapped", merged_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vendor_merge_toml() {
        let toml = r#"
[[vendor_group]]
name = "kimi"
providers = ["kimi", "kimi-coding"]

[[vendor_group]]
name = "ainaba"
providers = ["openai", "ainaiba"]
"#;
        let config: VendorMergeConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.vendor_group.len(), 2);

        let map = build_map(&config);
        assert_eq!(map.get("kimi"), Some(&"kimi".to_string()));
        assert_eq!(map.get("kimi-coding"), Some(&"kimi".to_string()));
        assert_eq!(map.get("openai"), Some(&"ainaba".to_string()));
        assert_eq!(map.get("ainaiba"), Some(&"ainaba".to_string()));
        assert_eq!(map.get("anthropic"), None);
    }

    #[test]
    fn apply_merge_remaps_providers() {
        let mut map = HashMap::new();
        map.insert("kimi-coding".to_string(), "kimi".to_string());
        map.insert("openai".to_string(), "ainaba".to_string());

        let mut records = vec![
            test_record("kimi"),
            test_record("kimi-coding"),
            test_record("openai"),
            test_record("anthropic"),
        ];

        apply_vendor_merge(&mut records, &map);

        assert_eq!(records[0].provider, "kimi");      // unchanged (already kimi)
        assert_eq!(records[1].provider, "kimi");      // remapped
        assert_eq!(records[2].provider, "ainaba");    // remapped
        assert_eq!(records[3].provider, "anthropic"); // unchanged
    }

    #[test]
    fn empty_map_is_noop() {
        let map = HashMap::new();
        let mut records = vec![test_record("openai")];
        apply_vendor_merge(&mut records, &map);
        assert_eq!(records[0].provider, "openai");
    }

    // helpers
    fn build_map(config: &VendorMergeConfig) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for group in &config.vendor_group {
            for provider in &group.providers {
                map.insert(provider.clone(), group.name.clone());
            }
        }
        map
    }

    fn test_record(provider: &str) -> TokenRecord {
        TokenRecord {
            date: "2026-05-17".to_string(),
            time: "2026-05-17T00:00:00Z".to_string(),
            api_key_prefix: "test".to_string(),
            provider: provider.to_string(),
            model: "test-model".to_string(),
            source: "test".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: 150,
            cost: 0.0,
        }
    }
}
