//! Stable, platform-neutral data exchanged by the native RoomPlan capture layer.
//!
//! The native iOS layer creates this document from `CapturedRoom`. The USDZ assets are
//! preserved beside it, but their USDA prim names and archive paths are deliberately not part
//! of this contract.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SCHEMA_VERSION: &str = "roomplan-capture/v1";

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct CaptureDraft {
    pub schema_version: String,
    pub draft_id: String,
    pub created_at: String,
    pub source: CaptureSource,
    pub coordinate_system: CoordinateSystem,
    pub room: CapturedRoom,
    #[serde(default)]
    pub assets: Vec<CaptureAsset>,
    pub provenance: CaptureProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct CaptureSource {
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roomplan_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct CoordinateSystem {
    pub units: String,
    pub up_axis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handedness: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct CapturedRoom {
    pub id: String,
    #[serde(default)]
    pub surfaces: Vec<RoomSurface>,
    #[serde(default)]
    pub openings: Vec<RoomOpening>,
    #[serde(default)]
    pub objects: Vec<RoomObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct RoomSurface {
    pub id: String,
    pub category: String,
    pub dimensions_m: [f32; 3],
    pub transform: [f32; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<CaptureConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct RoomOpening {
    pub id: String,
    pub category: String,
    pub dimensions_m: [f32; 3],
    pub transform: [f32; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<CaptureConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct RoomObject {
    pub id: String,
    pub category: String,
    pub dimensions_m: [f32; 3],
    pub transform: [f32; 16],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<CaptureConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    RefineExisting,
    NewRoom,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaptureConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaptureAssetRole {
    Parametric,
    Mesh,
    Model,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct CaptureAsset {
    pub asset_id: String,
    pub role: CaptureAssetRole,
    pub format: String,
    pub byte_length: u64,
    pub sha256: String,
    /// An opaque native-storage handle. Absolute sandbox paths never cross the bridge.
    pub storage_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct CaptureProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_mode: Option<CaptureMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<CaptureConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

impl CaptureDraft {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError(format!(
                "unsupported schema version: {}",
                self.schema_version
            )));
        }
        required("draft_id", &self.draft_id)?;
        required("created_at", &self.created_at)?;
        if self.source.platform != "ios" {
            return Err(ValidationError(format!(
                "unsupported capture platform: {}",
                self.source.platform
            )));
        }
        if self.coordinate_system.units != "meters" {
            return Err(ValidationError(
                "coordinate_system.units must be meters".into(),
            ));
        }
        if self.coordinate_system.up_axis != "Y" {
            return Err(ValidationError(
                "coordinate_system.up_axis must be Y".into(),
            ));
        }
        required("room.id", &self.room.id)?;

        let mut ids = HashSet::new();
        let mut source_ids = HashSet::new();
        for element in self
            .room
            .surfaces
            .iter()
            .map(|e| {
                (
                    &e.id,
                    &e.category,
                    &e.dimensions_m,
                    &e.transform,
                    &e.confidence,
                    &e.source_id,
                )
            })
            .chain(self.room.openings.iter().map(|e| {
                (
                    &e.id,
                    &e.category,
                    &e.dimensions_m,
                    &e.transform,
                    &e.confidence,
                    &e.source_id,
                )
            }))
            .chain(self.room.objects.iter().map(|e| {
                (
                    &e.id,
                    &e.category,
                    &e.dimensions_m,
                    &e.transform,
                    &e.confidence,
                    &e.source_id,
                )
            }))
        {
            required("room element id", element.0)?;
            required("room element category", element.1)?;
            if !ids.insert(element.0) {
                return Err(ValidationError(format!(
                    "duplicate room element id: {}",
                    element.0
                )));
            }
            if let Some(source_id) = element.5 {
                if !source_ids.insert(source_id) {
                    return Err(ValidationError(format!("duplicate source id: {source_id}")));
                }
            }
            validate_dimensions(element.2)?;
            validate_transform(element.3)?;
        }

        // RoomPlan exposes classification confidence as high/medium/low. Preserve that native
        // vocabulary instead of inventing a numeric conversion that would look more precise.
        let _ = &self.provenance.confidence;
        let mut asset_ids = HashSet::new();
        for asset in &self.assets {
            required("asset.asset_id", &asset.asset_id)?;
            required("asset.storage_token", &asset.storage_token)?;
            if !asset_ids.insert(&asset.asset_id) {
                return Err(ValidationError(format!(
                    "duplicate asset id: {}",
                    asset.asset_id
                )));
            }
            if asset.format != "usdz" {
                return Err(ValidationError(format!(
                    "unsupported asset format: {}",
                    asset.format
                )));
            }
            if asset.byte_length == 0 {
                return Err(ValidationError(format!(
                    "asset has zero bytes: {}",
                    asset.asset_id
                )));
            }
            if asset.sha256.len() != 64 || !asset.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ValidationError(format!(
                    "invalid SHA-256: {}",
                    asset.asset_id
                )));
            }
        }
        Ok(())
    }
}

fn required(name: &str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError(format!("{name} must not be empty")))
    } else {
        Ok(())
    }
}

fn validate_dimensions(dimensions: &[f32; 3]) -> Result<(), ValidationError> {
    if dimensions.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(ValidationError(
            "dimensions must be finite and non-negative".into(),
        ));
    }
    Ok(())
}

fn validate_transform(transform: &[f32; 16]) -> Result<(), ValidationError> {
    if transform.iter().any(|v| !v.is_finite()) {
        return Err(ValidationError(
            "transform must contain only finite values".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> CaptureDraft {
        CaptureDraft {
            schema_version: SCHEMA_VERSION.into(),
            draft_id: "draft-1".into(),
            created_at: "2026-09-23T12:00:00Z".into(),
            source: CaptureSource {
                platform: "ios".into(),
                roomplan_version: Some("17".into()),
                app_version: Some("0.1.0".into()),
            },
            coordinate_system: CoordinateSystem {
                units: "meters".into(),
                up_axis: "Y".into(),
                handedness: None,
            },
            room: CapturedRoom {
                id: "room-1".into(),
                surfaces: vec![RoomSurface {
                    id: "surface-1".into(),
                    category: "wall".into(),
                    dimensions_m: [2.0, 2.5, 0.16],
                    transform: [0.0; 16],
                    confidence: Some(CaptureConfidence::High),
                    source_id: Some("native-wall-1".into()),
                }],
                openings: vec![],
                objects: vec![],
            },
            assets: vec![CaptureAsset {
                asset_id: "asset-1".into(),
                role: CaptureAssetRole::Parametric,
                format: "usdz".into(),
                byte_length: 1,
                sha256: "a".repeat(64),
                storage_token: "asset-token-1".into(),
            }],
            provenance: CaptureProvenance {
                capture_mode: Some(CaptureMode::NewRoom),
                parent_revision_id: None,
                confidence: Some(CaptureConfidence::High),
                capture_started_at: None,
                capture_finished_at: None,
            },
        }
    }

    #[test]
    fn accepts_a_native_draft() {
        assert!(draft().validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_element_ids() {
        let mut value = draft();
        value.room.objects.push(RoomObject {
            id: "surface-1".into(),
            category: "chair".into(),
            dimensions_m: [1.0; 3],
            transform: [0.0; 16],
            confidence: None,
            source_id: None,
        });
        assert!(value
            .validate()
            .unwrap_err()
            .0
            .contains("duplicate room element"));
    }

    #[test]
    fn rejects_non_meter_coordinates() {
        let mut value = draft();
        value.coordinate_system.units = "centimeters".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn round_trips_json() {
        let value = draft();
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: CaptureDraft = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
    }
}
