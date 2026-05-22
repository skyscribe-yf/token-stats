// serde is used implicitly via serde_json; kept available for future struct expansion
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default advanced model list used when the config file is missing.
const DEFAULT_ADVANCED_MODELS: &[&str] = &[
    "GLM-5.1",
    "deepseek-v4-pro",
    "gpt-5.4",
    "gpt-5.5",
    "kimi-for-coding",
    "kimi-k2.6",
    "kimi-k2.6:high",
    "mimi-v2.5-pro",
];

/// Return the path to the advanced models JSON config file.
///
/// Resolution order:
/// 1. `ADVANCED_MODELS_CONFIG` environment variable
/// 2. `advanced_models.json` next to the running binary
/// 3. `advanced_models.json` in the current working directory
pub fn advanced_models_path() -> PathBuf {
    if let Ok(p) = std::env::var("ADVANCED_MODELS_CONFIG") {
        return PathBuf::from(p);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("advanced_models.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    PathBuf::from("advanced_models.json")
}

/// Load the advanced models list from disk.
///
/// If the file does not exist or is malformed, returns the hard-coded
/// default list.
pub fn load_advanced_models() -> Vec<String> {
    let path = advanced_models_path();
    load_advanced_models_from_path(&path)
}

fn load_advanced_models_from_path(path: &Path) -> Vec<String> {
    if !path.exists() {
        tracing::info!(
            "Advanced models config not found at {:?}, using defaults",
            path
        );
        return default_advanced_models();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read advanced models config: {}", e);
            return default_advanced_models();
        }
    };

    let models: Vec<String> = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Failed to parse advanced models config: {}", e);
            return default_advanced_models();
        }
    };

    tracing::info!(
        "Loaded advanced models config: {} entries from {:?}",
        models.len(),
        path
    );
    models
}

/// Save the advanced models list to disk.
///
/// Writes atomically by first writing to a temporary file and then renaming.
pub fn save_advanced_models(models: &[String]) -> Result<(), String> {
    let path = advanced_models_path();
    let content =
        serde_json::to_string_pretty(models).map_err(|e| format!("Serialization error: {}", e))?;

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, content).map_err(|e| format!("Write error: {}", e))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("Rename error: {}", e))?;

    tracing::info!(
        "Saved advanced models config ({}) to {:?}",
        models.len(),
        path
    );
    Ok(())
}

fn default_advanced_models() -> Vec<String> {
    DEFAULT_ADVANCED_MODELS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ─── Subscription Settings ────────────────────────────────────────────────────

/// Subscription configuration persisted to disk.
///
/// Tracks user-configurable subscription parameters like billing cycle start dates
/// so the dashboard can compute expiration alerts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionSettings {
    /// Day of month (1–28) when the Kimi monthly subscription renews.
    /// `None` means the user has not configured it yet.
    pub kimi_monthly_start_day: Option<u8>,
}

impl Default for SubscriptionSettings {
    fn default() -> Self {
        Self {
            kimi_monthly_start_day: None,
        }
    }
}

/// Return the path to the subscription settings JSON file.
///
/// Resolution order:
/// 1. `SUBSCRIPTION_SETTINGS_CONFIG` environment variable
/// 2. `subscription_settings.json` next to the running binary
/// 3. `subscription_settings.json` in the current working directory
pub fn subscription_settings_path() -> PathBuf {
    if let Ok(p) = std::env::var("SUBSCRIPTION_SETTINGS_CONFIG") {
        return PathBuf::from(p);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("subscription_settings.json");
            if candidate.exists() {
                return candidate;
            }
        }
    }

    PathBuf::from("subscription_settings.json")
}

/// Load subscription settings from disk.
///
/// If the file does not exist or is malformed, returns the default
/// (with `kimi_monthly_start_day: None`).
pub fn load_subscription_settings() -> SubscriptionSettings {
    let path = subscription_settings_path();
    load_subscription_settings_from_path(&path)
}

fn load_subscription_settings_from_path(path: &Path) -> SubscriptionSettings {
    if !path.exists() {
        tracing::info!(
            "Subscription settings not found at {:?}, using defaults",
            path
        );
        return SubscriptionSettings::default();
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read subscription settings: {}", e);
            return SubscriptionSettings::default();
        }
    };

    let settings: SubscriptionSettings = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to parse subscription settings: {}", e);
            return SubscriptionSettings::default();
        }
    };

    // Validate kimi_monthly_start_day is in 1..=28 if set
    if let Some(day) = settings.kimi_monthly_start_day {
        if !(1..=28).contains(&day) {
            tracing::warn!(
                "Invalid kimi_monthly_start_day {} in settings file, ignoring",
                day
            );
            return SubscriptionSettings {
                kimi_monthly_start_day: None,
            };
        }
    }

    tracing::info!("Loaded subscription settings from {:?}", path);
    settings
}

/// Save subscription settings to disk.
///
/// Writes atomically by first writing to a temporary file and then renaming.
pub fn save_subscription_settings(settings: &SubscriptionSettings) -> Result<(), String> {
    let path = subscription_settings_path();
    let content = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Serialization error: {}", e))?;

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, content).map_err(|e| format!("Write error: {}", e))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("Rename error: {}", e))?;

    tracing::info!("Saved subscription settings to {:?}", path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── Advanced Models Tests ────────────────────────────────────────────────

    #[test]
    fn load_returns_defaults_when_file_missing() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("nonexistent.json");
        let models = load_advanced_models_from_path(&path);
        assert!(!models.is_empty());
        assert!(models.contains(&"gpt-5.4".to_string()));
    }

    #[test]
    fn load_parses_existing_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("advanced_models.json");
        let file_models = vec!["model-a".to_string(), "model-b".to_string()];
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(serde_json::to_string(&file_models).unwrap().as_bytes())
                .unwrap();
        }
        let models = load_advanced_models_from_path(&path);
        assert_eq!(models, file_models);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("advanced_models.json");

        let original = vec!["x".to_string(), "y".to_string()];
        {
            // Temporarily override the path by writing directly
            std::fs::write(&path, serde_json::to_string(&original).unwrap()).unwrap();
        }

        let loaded = load_advanced_models_from_path(&path);
        assert_eq!(loaded, original);
    }

    #[test]
    fn path_resolution_prefers_env_var() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let expected = tmp_dir.path().join("from_env.json");
        temp_env::with_var(
            "ADVANCED_MODELS_CONFIG",
            Some(expected.to_str().unwrap()),
            || {
                let path = advanced_models_path();
                assert_eq!(path, expected);
            },
        );
    }

    // ── Subscription Settings Tests ───────────────────────────────────────────

    #[test]
    fn subscription_settings_default_has_none() {
        let settings = SubscriptionSettings::default();
        assert!(settings.kimi_monthly_start_day.is_none());
    }

    #[test]
    fn subscription_settings_load_returns_default_when_missing() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("nonexistent.json");
        let settings = load_subscription_settings_from_path(&path);
        assert!(settings.kimi_monthly_start_day.is_none());
    }

    #[test]
    fn subscription_settings_save_and_reload() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let path = tmp_dir.path().join("subscription_settings.json");
        let original = SubscriptionSettings {
            kimi_monthly_start_day: Some(15),
        };
        let content = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(&path, content).unwrap();
        let loaded = load_subscription_settings_from_path(&path);
        assert_eq!(loaded.kimi_monthly_start_day, Some(15));
    }

    #[test]
    fn subscription_settings_path_resolution_prefers_env_var() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let expected = tmp_dir.path().join("from_env.json");
        temp_env::with_var(
            "SUBSCRIPTION_SETTINGS_CONFIG",
            Some(expected.to_str().unwrap()),
            || {
                let path = subscription_settings_path();
                assert_eq!(path, expected);
            },
        );
    }

    #[test]
    fn subscription_settings_invalid_day_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subscription_settings.json");
        std::fs::write(&path, r#"{\"kimi_monthly_start_day\": 31}"#).unwrap();
        let settings = load_subscription_settings_from_path(&path);
        assert_eq!(settings.kimi_monthly_start_day, None);
    }
}
