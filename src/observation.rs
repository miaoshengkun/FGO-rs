use crate::{domain::Scene, vision::Rect};
use serde::Serialize;
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionMethod {
    MaaOcr,
    BinaryGlyphDifference,
    TemplateDifference,
    ColorChromaPixelCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorKind {
    CommandCardColor,
    BattleSpeed,
    NpPercent,
    AllyHp,
    EnemyHp,
    ServantSkillCooldown,
    MasterSkillCooldown,
    SkillIconChroma,
    CriticalRate,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreDirection {
    LowerIsBetter,
    HigherIsBetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RecognitionEvidence {
    pub detector: DetectorKind,
    pub roi: Rect,
    pub method: RecognitionMethod,
    pub score: f64,
    pub threshold: f64,
    pub direction: ScoreDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_floor: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldEvidence {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(flatten)]
    pub evidence: RecognitionEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    NoCandidate,
    IncompleteText,
    Ambiguous,
    ConflictingCandidates,
    NoSupportMarker,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Recognition<T> {
    Known {
        value: T,
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence: Option<RecognitionEvidence>,
    },
    Unknown {
        reason: UnknownReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence: Option<RecognitionEvidence>,
    },
    #[default]
    NotRequested,
}

impl<T> From<T> for Recognition<T> {
    fn from(value: T) -> Self {
        Self::Known {
            value,
            evidence: None,
        }
    }
}

impl<T: Copy> Recognition<T> {
    pub const fn known(value: T, evidence: RecognitionEvidence) -> Self {
        Self::Known {
            value,
            evidence: Some(evidence),
        }
    }

    pub const fn unknown(reason: UnknownReason, evidence: Option<RecognitionEvidence>) -> Self {
        Self::Unknown { reason, evidence }
    }

    pub const fn value(self) -> Option<T> {
        match self {
            Self::Known { value, .. } => Some(value),
            Self::Unknown { .. } | Self::NotRequested => None,
        }
    }

    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown { .. })
    }

    pub const fn evidence(self) -> Option<RecognitionEvidence> {
        match self {
            Self::Known { evidence, .. } | Self::Unknown { evidence, .. } => evidence,
            Self::NotRequested => None,
        }
    }
}

/// Scene-specific facts coupled to one mutually exclusive primary scene.
#[derive(Clone, Debug, Serialize)]
pub struct Observation<T> {
    pub scene: Scene,
    pub facts: T,
}

pub trait SceneFacts {
    /// Return only mutually exclusive foreground candidates, in priority order.
    fn scene_candidates(&self) -> Vec<Scene>;

    fn unknown_fields(&self) -> Vec<String> {
        Vec::new()
    }

    fn recognition_evidence(&self) -> Vec<FieldEvidence> {
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneConflict {
    pub candidates: Vec<Scene>,
}

impl fmt::Display for SceneConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "conflicting primary scenes: ")?;
        for (index, scene) in self.candidates.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{scene:?}")?;
        }
        Ok(())
    }
}

impl Error for SceneConflict {}

impl<T: SceneFacts> Observation<T> {
    pub fn classify(facts: T) -> Result<Self, SceneConflict> {
        let mut candidates = Vec::new();
        for scene in facts.scene_candidates() {
            if !candidates.contains(&scene) {
                candidates.push(scene);
            }
        }
        if candidates.len() > 1 {
            return Err(SceneConflict { candidates });
        }
        Ok(Self {
            scene: candidates.first().copied().unwrap_or(Scene::Unknown),
            facts,
        })
    }
}
