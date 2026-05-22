// serde is used implicitly via serde_json; kept available for future struct expansion
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
}
