//! Device profiles (schema v1, `docs/profiles/schema-v1.md`). Profiles are
//! data, never code: strict keys, no expressions beyond `raw × scale + offset`.

pub mod json;
pub mod model;
pub mod validate;

pub use model::*;
pub use validate::{Issue, Report};

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ProfileError {
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("profile has {} error(s): {}", .0.len(), .0.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "))]
    Invalid(Vec<Issue>),
    #[error("internal: validated profile did not match the typed model: {0}")]
    Model(String),
}

/// A validated, typed profile plus the validator's warnings.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedProfile {
    pub profile: Profile,
    pub warnings: Vec<Issue>,
}

/// Validate a document without building the typed model.
pub fn check(bytes: &[u8]) -> Result<Report, ProfileError> {
    let doc = json::parse(bytes).map_err(ProfileError::Json)?;
    Ok(validate::validate(&doc))
}

/// Size check → strict parse → validate → typed model.
pub fn load(bytes: &[u8]) -> Result<LoadedProfile, ProfileError> {
    let doc = json::parse(bytes).map_err(ProfileError::Json)?;
    let report = validate::validate(&doc);
    if !report.errors.is_empty() {
        return Err(ProfileError::Invalid(report.errors));
    }
    let profile: Profile = serde_json::from_value(doc.to_value()).map_err(|e| ProfileError::Model(e.to_string()))?;
    Ok(LoadedProfile { profile, warnings: report.warnings })
}

pub fn load_str(text: &str) -> Result<LoadedProfile, ProfileError> {
    load(text.as_bytes())
}
