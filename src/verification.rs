use crate::domain::{Command, Scene};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct StabilityRule {
    pub required_consecutive: u8,
    pub min_interval_ms: u64,
}

impl StabilityRule {
    pub const ONCE: Self = Self {
        required_consecutive: 1,
        min_interval_ms: 0,
    };

    pub const fn consecutive(required_consecutive: u8, min_interval_ms: u64) -> Self {
        Self {
            required_consecutive,
            min_interval_ms,
        }
    }

    fn validate(self) -> Result<()> {
        ensure!(
            self.required_consecutive > 0,
            "verification stability requires at least one matching observation"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerificationOrigin {
    pub last_action: Option<Command>,
    pub action_frame: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct VerificationObservation {
    pub frame: u64,
    pub scene: Scene,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationProgress {
    Pending,
    Succeeded,
    TimedOut,
}

/// State for one post-action verification loop.
///
/// This type has no device or vision dependency, so deadlines, stable-frame
/// requirements and bounded transition rules can be tested independently.
#[derive(Clone, Debug, Serialize)]
pub struct ActionVerificationContext {
    pub description: String,
    pub expected_scenes: Vec<Scene>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_action: Option<Command>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_frame: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_from: Option<Scene>,
    pub started_at_ms: u64,
    pub deadline_ms: u64,
    pub retry_count: u32,
    pub stability: StabilityRule,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_observation: Option<VerificationObservation>,
    #[serde(skip)]
    consecutive_matches: u8,
    #[serde(skip)]
    last_match_ms: Option<u64>,
    #[serde(skip)]
    last_match_frame: Option<u64>,
}

impl ActionVerificationContext {
    pub fn new(
        description: impl Into<String>,
        expected_scenes: &[Scene],
        origin: VerificationOrigin,
        transition_from: Option<Scene>,
        now_ms: u64,
        timeout_ms: u64,
        stability: StabilityRule,
    ) -> Result<Self> {
        ensure!(
            !expected_scenes.is_empty(),
            "verification requires at least one expected scene"
        );
        stability.validate()?;
        let mut scenes = Vec::with_capacity(expected_scenes.len());
        for scene in expected_scenes {
            if !scenes.contains(scene) {
                scenes.push(*scene);
            }
        }
        Ok(Self {
            description: description.into(),
            expected_scenes: scenes,
            last_action: origin.last_action,
            action_frame: origin.action_frame,
            transition_from,
            started_at_ms: now_ms,
            deadline_ms: now_ms.saturating_add(timeout_ms),
            retry_count: 0,
            stability,
            current_observation: None,
            consecutive_matches: 0,
            last_match_ms: None,
            last_match_frame: None,
        })
    }

    pub fn observe(
        &mut self,
        frame: u64,
        observed_scene: Scene,
        condition_matches: bool,
        now_ms: u64,
    ) -> VerificationProgress {
        self.retry_count = self.retry_count.saturating_add(1);
        let effective_scene = self.effective_scene(observed_scene, condition_matches);
        self.current_observation = Some(VerificationObservation {
            frame,
            scene: effective_scene,
            elapsed_ms: now_ms,
        });

        let expected = self.expected_scenes.contains(&effective_scene);
        let interval_satisfied = self
            .last_match_ms
            .is_none_or(|last| now_ms.saturating_sub(last) >= self.stability.min_interval_ms);
        let new_frame = self.last_match_frame != Some(frame);
        if condition_matches && expected && interval_satisfied && new_frame {
            self.consecutive_matches = self.consecutive_matches.saturating_add(1);
            self.last_match_ms = Some(now_ms);
            self.last_match_frame = Some(frame);
            if self.consecutive_matches >= self.stability.required_consecutive {
                return VerificationProgress::Succeeded;
            }
        } else if !(condition_matches && expected) {
            self.consecutive_matches = 0;
            self.last_match_ms = None;
            self.last_match_frame = None;
        }

        if now_ms >= self.deadline_ms {
            VerificationProgress::TimedOut
        } else {
            VerificationProgress::Pending
        }
    }

    fn effective_scene(&self, observed: Scene, condition_matches: bool) -> Scene {
        let bounded_battle_animation = condition_matches
            && observed == Scene::Unknown
            && self.expected_scenes.contains(&Scene::BattleAnimation)
            && self.transition_from == Some(Scene::CommandCards)
            && matches!(
                self.last_action,
                Some(Command::Attack { .. } | Command::SelectCards)
            );
        if bounded_battle_animation {
            Scene::BattleAnimation
        } else {
            observed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(stability: StabilityRule) -> ActionVerificationContext {
        ActionVerificationContext::new(
            "test transition",
            &[Scene::BattleTurn],
            VerificationOrigin::default(),
            None,
            100,
            500,
            stability,
        )
        .unwrap()
    }

    #[test]
    fn success_requires_both_the_condition_and_an_expected_scene() {
        let mut context = context(StabilityRule::ONCE);
        assert_eq!(
            context.observe(1, Scene::CommandCards, true, 100),
            VerificationProgress::Pending
        );
        assert_eq!(
            context.observe(2, Scene::BattleTurn, true, 250),
            VerificationProgress::Succeeded
        );
        assert_eq!(context.retry_count, 2);
        assert_eq!(context.current_observation.unwrap().frame, 2);
    }

    #[test]
    fn stability_resets_after_a_non_matching_observation() {
        let mut context = context(StabilityRule::consecutive(2, 100));
        assert_eq!(
            context.observe(1, Scene::BattleTurn, true, 100),
            VerificationProgress::Pending
        );
        assert_eq!(
            context.observe(1, Scene::BattleTurn, true, 200),
            VerificationProgress::Pending,
            "the same frame cannot satisfy a multi-frame rule"
        );
        assert_eq!(
            context.observe(2, Scene::Unknown, false, 250),
            VerificationProgress::Pending
        );
        assert_eq!(
            context.observe(3, Scene::BattleTurn, true, 300),
            VerificationProgress::Pending
        );
        assert_eq!(
            context.observe(4, Scene::BattleTurn, true, 400),
            VerificationProgress::Succeeded
        );
    }

    #[test]
    fn unknown_is_animation_only_for_the_bounded_attack_transition() {
        let mut context = ActionVerificationContext::new(
            "starting attack",
            &[Scene::BattleAnimation],
            VerificationOrigin {
                last_action: Some(Command::Attack { opening: false }),
                action_frame: Some(12),
            },
            Some(Scene::CommandCards),
            0,
            500,
            StabilityRule::ONCE,
        )
        .unwrap();
        assert_eq!(
            context.observe(13, Scene::Unknown, true, 1),
            VerificationProgress::Succeeded
        );
        assert_eq!(
            context.current_observation.unwrap().scene,
            Scene::BattleAnimation
        );

        let mut unrelated = ActionVerificationContext::new(
            "not an attack",
            &[Scene::BattleAnimation],
            VerificationOrigin::default(),
            Some(Scene::CommandCards),
            0,
            1,
            StabilityRule::ONCE,
        )
        .unwrap();
        assert_eq!(
            unrelated.observe(1, Scene::Unknown, true, 1),
            VerificationProgress::TimedOut
        );
        assert_eq!(unrelated.current_observation.unwrap().scene, Scene::Unknown);
    }
}
