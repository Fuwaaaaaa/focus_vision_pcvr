use serde::{Deserialize, Serialize};

/// Total blendshape count (37 lip + 14 eye).
pub const TOTAL_BLENDSHAPES: usize = 51;

/// Face tracking expression profile.
/// Per-blendshape sensitivity weights that scale raw HTC values before OSC output.
/// A weight of 1.0 = unchanged, 2.0 = doubled sensitivity, 0.5 = halved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtProfile {
    pub name: String,
    /// Per-blendshape weights: [lip0..lip36, eye0..eye13] = 51 values.
    /// Missing values default to 1.0.
    #[serde(default = "default_weights")]
    pub weights: Vec<f32>,
    /// Optional smoothing override. None = use global config.
    #[serde(default)]
    pub smoothing_override: Option<f32>,
}

fn default_weights() -> Vec<f32> {
    vec![1.0; TOTAL_BLENDSHAPES]
}

impl Default for FtProfile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            weights: default_weights(),
            smoothing_override: None,
        }
    }
}

impl FtProfile {
    /// Get weight for a blendshape index (0-50). Returns 1.0 if out of range.
    pub fn weight(&self, index: usize) -> f32 {
        self.weights.get(index).copied().unwrap_or(1.0)
    }

    /// Ensure weights vector has exactly TOTAL_BLENDSHAPES entries, padding with 1.0.
    pub fn normalize(&mut self) {
        self.weights.resize(TOTAL_BLENDSHAPES, 1.0);
    }

    /// Replace NaN/Infinity/negative weights with 1.0.
    pub fn sanitize_weights(&mut self) {
        for w in &mut self.weights {
            if w.is_nan() || w.is_infinite() || *w < 0.0 {
                *w = 1.0;
            }
        }
    }

    /// Run all post-deserialization checks: pad to TOTAL_BLENDSHAPES, then
    /// replace invalid values (NaN/Inf/negative) with 1.0.
    pub fn validate(&mut self) {
        self.normalize();
        self.sanitize_weights();
    }
}

/// Profile storage directory: %APPDATA%/FocusVisionPCVR/profiles/
fn profiles_dir() -> Option<std::path::PathBuf> {
    dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join("profiles"))
}

/// Validate profile name: reject path traversal, separators, and null bytes.
fn sanitize_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("Profile name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        return Err("Profile name contains invalid characters");
    }
    if name.len() > 64 {
        return Err("Profile name too long (max 64 chars)");
    }
    Ok(())
}

/// List available profile names (without .json extension).
pub fn list_profiles() -> Vec<String> {
    let dir = match profiles_dir() {
        Some(d) => d,
        None => return vec![],
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(|s| s.to_string())
        })
        .collect()
}

/// Load a profile by name. Returns None if not found, invalid, or name fails sanitization.
pub fn load_profile(name: &str) -> Option<FtProfile> {
    sanitize_name(name).ok()?;
    let dir = profiles_dir()?;
    let path = dir.join(format!("{name}.json"));
    let content = std::fs::read_to_string(&path).ok()?;
    let mut profile: FtProfile = serde_json::from_str(&content).ok()?;
    profile.validate();
    Some(profile)
}

/// Save a profile. Creates the profiles directory if needed.
pub fn save_profile(profile: &FtProfile) -> Result<(), Box<dyn std::error::Error>> {
    sanitize_name(&profile.name)?;
    let dir = profiles_dir().ok_or("Cannot determine app data directory")?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", profile.name));
    let json = serde_json::to_string_pretty(profile)?;
    std::fs::write(&path, json)?;
    log::info!("FT profile saved: {}", profile.name);
    Ok(())
}

/// Delete a profile by name.
pub fn delete_profile(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    sanitize_name(name)?;
    let dir = match profiles_dir() {
        Some(d) => d,
        None => return Ok(()),
    };
    let path = dir.join(format!("{name}.json"));
    if path.exists() {
        std::fs::remove_file(&path)?;
        log::info!("FT profile deleted: {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_profile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("fvp_test_profiles");
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_default_profile() {
        let p = FtProfile::default();
        assert_eq!(p.name, "default");
        assert_eq!(p.weights.len(), TOTAL_BLENDSHAPES);
        assert!(p.weights.iter().all(|&w| (w - 1.0).abs() < f32::EPSILON));
        assert_eq!(p.smoothing_override, None);
    }

    #[test]
    fn test_weight_access() {
        let p = FtProfile::default();
        assert_eq!(p.weight(0), 1.0);
        assert_eq!(p.weight(50), 1.0);
        assert_eq!(p.weight(999), 1.0); // Out of range → default 1.0
    }

    #[test]
    fn test_normalize_short_weights() {
        let mut p = FtProfile {
            name: "test".to_string(),
            weights: vec![2.0, 0.5], // Only 2 values
            smoothing_override: None,
        };
        p.normalize();
        assert_eq!(p.weights.len(), TOTAL_BLENDSHAPES);
        assert_eq!(p.weights[0], 2.0);
        assert_eq!(p.weights[1], 0.5);
        assert_eq!(p.weights[2], 1.0); // Padded
    }

    #[test]
    fn test_profile_serialize_deserialize() {
        let p = FtProfile {
            name: "avatar_test".to_string(),
            weights: vec![1.5; TOTAL_BLENDSHAPES],
            smoothing_override: Some(0.8),
        };
        let json = serde_json::to_string(&p).unwrap();
        let p2: FtProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p2.name, "avatar_test");
        assert_eq!(p2.weights.len(), TOTAL_BLENDSHAPES);
        assert_eq!(p2.smoothing_override, Some(0.8));
    }

    #[test]
    fn test_profile_deserialize_missing_weights() {
        let json = r#"{"name":"minimal"}"#;
        let mut p: FtProfile = serde_json::from_str(json).unwrap();
        p.normalize();
        assert_eq!(p.name, "minimal");
        assert_eq!(p.weights.len(), TOTAL_BLENDSHAPES);
        assert!(p.weights.iter().all(|&w| (w - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn test_save_load_roundtrip() {
        let dir = temp_profile_dir();
        let path = dir.join("roundtrip_test.json");

        let p = FtProfile {
            name: "roundtrip_test".to_string(),
            weights: {
                let mut w = vec![1.0; TOTAL_BLENDSHAPES];
                w[0] = 2.0;
                w[10] = 0.3;
                w
            },
            smoothing_override: Some(0.7),
        };

        let json = serde_json::to_string_pretty(&p).unwrap();
        fs::write(&path, &json).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let mut loaded: FtProfile = serde_json::from_str(&content).unwrap();
        loaded.normalize();

        assert_eq!(loaded.name, "roundtrip_test");
        assert_eq!(loaded.weights[0], 2.0);
        assert_eq!(loaded.weights[10], 0.3);
        assert_eq!(loaded.smoothing_override, Some(0.7));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn test_validate_pads_and_sanitizes() {
        let mut p = FtProfile {
            name: "bad".to_string(),
            weights: vec![2.0, f32::NAN, f32::INFINITY, -1.5],
            smoothing_override: None,
        };
        p.validate();
        assert_eq!(p.weights.len(), TOTAL_BLENDSHAPES);
        assert_eq!(p.weights[0], 2.0);
        assert_eq!(p.weights[1], 1.0); // NaN → 1.0
        assert_eq!(p.weights[2], 1.0); // Infinity → 1.0
        assert_eq!(p.weights[3], 1.0); // Negative → 1.0
        assert_eq!(p.weights[4], 1.0); // Padded
        assert!(p.weights.iter().all(|w| w.is_finite() && *w >= 0.0));
    }
}
