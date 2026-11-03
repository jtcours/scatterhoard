// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The LLM's confidence in a memory.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::ValidationError;

/// How much the memory's author trusts the memory, on 0.0..=1.0.
///
/// Validated once here, so nothing downstream rechecks the range. Excluding NaN at construction
/// also buys a total ordering: recall ranks by confidence among other signals, and can sort
/// directly instead of unwrapping a partial comparison at every ranking site.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Confidence(f64);

impl Confidence {
    /// Construct a confidence.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::ConfidenceNotFinite`] for NaN or an infinity, and
    /// [`ValidationError::ConfidenceOutOfRange`] for a finite value outside 0.0..=1.0.
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError::ConfidenceNotFinite);
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(ValidationError::ConfidenceOutOfRange { value });
        }
        Ok(Self(value))
    }

    /// The underlying value, always finite and within 0.0..=1.0.
    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for Confidence {
    type Error = ValidationError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Confidence> for f64 {
    fn from(confidence: Confidence) -> Self {
        confidence.0
    }
}

// `Eq` and `Ord` are sound only because `new` rejects NaN. The two impls and that check have to
// move together.
impl Eq for Confidence {}

impl Ord for Confidence {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .partial_cmp(&other.0)
            .expect("NaN is excluded by Confidence::new")
    }
}

impl PartialOrd for Confidence {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
