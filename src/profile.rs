use crate::domain::{FieldSlot, PartySlot, SkillSlot};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::LazyLock};

pub const PROFILE_SCHEMA_V1: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnemyTargetStrategy {
    #[default]
    Current,
    HighestHp,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceCardPriority {
    SupportCount,
    BraveChain,
    ColorChain,
    Damage,
}

impl FaceCardPriority {
    const fn index(self) -> usize {
        match self {
            Self::SupportCount => 0,
            Self::BraveChain => 1,
            Self::ColorChain => 2,
            Self::Damage => 3,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CardStrategyProfile {
    /// Original party-slot identities, highest NP priority first. `None` keeps
    /// legacy screen order for profiles created before identity-aware sorting.
    pub noble_phantasm_order: Option<[PartySlot; 6]>,
    /// Lexicographic priorities used when selecting face-card permutations.
    pub face_card_priority: [FaceCardPriority; 4],
    /// Arts, Quick and Buster base scores in hundredths.
    pub color_score: [u16; 3],
    /// First, second and third card position multipliers in hundredths.
    pub position_multiplier: [u16; 3],
    /// Bonus added to every face-card score when the first overall card is Buster.
    pub first_buster_bonus: u16,
    /// Treat unresolved face-card owners as the configured support servant
    /// when the support-card badge is not visible on a calibrated screen.
    /// Kept opt-in because unknown owners must remain conservative elsewhere.
    pub infer_unknown_support_cards: bool,
}

impl Default for CardStrategyProfile {
    fn default() -> Self {
        Self {
            noble_phantasm_order: None,
            face_card_priority: [
                FaceCardPriority::SupportCount,
                FaceCardPriority::BraveChain,
                FaceCardPriority::ColorChain,
                FaceCardPriority::Damage,
            ],
            color_score: [80, 100, 110],
            position_multiplier: [100, 120, 140],
            first_buster_bonus: 30,
            infer_unknown_support_cards: false,
        }
    }
}

impl CardStrategyProfile {
    pub(crate) fn validate(&self, profile_id: &str) -> Result<()> {
        if let Some(order) = self.noble_phantasm_order {
            let mut party_slots = [false; 6];
            for slot in order {
                ensure!(
                    !party_slots[slot.index()],
                    "party profile {profile_id} repeats NP party slot {}",
                    slot.index() + 1
                );
                party_slots[slot.index()] = true;
            }
        }

        let mut priorities = [false; 4];
        for priority in self.face_card_priority {
            ensure!(
                !priorities[priority.index()],
                "party profile {profile_id} repeats face-card priority {priority:?}"
            );
            priorities[priority.index()] = true;
        }
        ensure!(
            self.color_score.iter().all(|value| *value > 0)
                && self.position_multiplier.iter().all(|value| *value > 0),
            "party profile {profile_id} card scores and multipliers must be positive"
        );
        Ok(())
    }
}

/// Earliest wave/turn pair, compared lexicographically like FGO-py's skillInfo.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTiming {
    pub wave: u8,
    pub turn: u32,
}

/// Absolute current HP threshold; maximum HP is not inferred from Atlas stats.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HpBelowCondition {
    pub servant: PartySlot,
    pub hp: u32,
}

/// Evaluate the current NP of an original party member immediately before a skill.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NpBelowCondition {
    pub servant: PartySlot,
    /// Strict upper bound; 100 permits 0..99%, never 100%.
    pub percent: u16,
}

/// One alternative in a composable skill condition. Fields inside one
/// alternative are combined with AND; alternatives in `any_of` are ORed.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRuleConditionAlternative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub np_below: Option<NpBelowCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp_below: Option<HpBelowCondition>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enemy_charge_full: bool,
}

impl SkillRuleConditionAlternative {
    pub fn is_empty(&self) -> bool {
        self.np_below.is_none() && self.hp_below.is_none() && !self.enemy_charge_full
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LowestNpTarget {
    /// Strict upper bound; the selected current-frontline member must be below it.
    pub below: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LowestHpTarget {
    /// Strict upper bound for the selected current-frontline member's absolute HP.
    pub below: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServantSkillRule {
    /// Optional exact Atlas version; validates metadata, not account unlocks.
    #[serde(default)]
    pub atlas_skill_id: Option<u32>,
    pub servant: PartySlot,
    pub skill: SkillSlot,
    /// Original ally identity, a dynamic frontline target, or an enemy layout slot.
    pub target: SkillRuleTarget,
    /// Omission means every turn when available. Array order is cast priority.
    #[serde(default)]
    pub earliest: Option<SkillTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub np_below: Option<NpBelowCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp_below: Option<HpBelowCondition>,
    /// Cast only when at least one visible enemy has a confirmed full charge gauge.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enemy_charge_full: bool,
    /// Optional OR-composed alternatives for complex FGO-py skill branches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<SkillRuleConditionAlternative>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum SkillRuleTarget {
    Ally(PartySlot),
    Enemy { enemy: crate::domain::EnemySlot },
    LowestNp { lowest_np: LowestNpTarget },
    LowestHp { lowest_hp: LowestHpTarget },
}

impl From<PartySlot> for SkillRuleTarget {
    fn from(slot: PartySlot) -> Self {
        Self::Ally(slot)
    }
}

pub type MasterSkillTarget = SkillRuleTarget;

/// Ordinary master skills; order change retains its separately verified API.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MasterSkillRule {
    pub skill: SkillSlot,
    pub target: MasterSkillTarget,
    #[serde(default)]
    pub earliest: Option<SkillTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub np_below: Option<NpBelowCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp_below: Option<HpBelowCondition>,
    /// Cast only when at least one visible enemy has a confirmed full charge gauge.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enemy_charge_full: bool,
    /// Optional OR-composed alternatives for complex FGO-py skill branches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<SkillRuleConditionAlternative>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRequirement {
    pub servant: FieldSlot,
    pub skill: SkillSlot,
}

/// User-declared account facts. Missing values are unknown, never inferred.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServantAccountState {
    pub level: Option<u32>,
    pub ascension: Option<u32>,
    /// Highest completed phase for each explicitly reported quest; 0 means
    /// not completed. An absent quest is unknown, not implicitly completed.
    pub quest_phases: std::collections::BTreeMap<u32, u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServantReference {
    #[serde(default)]
    pub account_state: Option<ServantAccountState>,
    pub slot: PartySlot,
    pub atlas_id: u32,
    #[serde(default)]
    pub noble_phantasm_id: Option<u32>,
}

/// Exact metadata version for one skill of a dynamic support candidate.
/// This does not establish the selected friend's identity or account unlocks.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportSkillVersion {
    pub skill: SkillSlot,
    pub atlas_skill_id: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupportServantReference {
    pub atlas_id: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skill_versions: Vec<SupportSkillVersion>,
    #[serde(default)]
    pub noble_phantasm_id: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrderChangeProfile {
    pub master_skill: SkillSlot,
    pub field: PartySlot,
    pub reserve: PartySlot,
    pub before_markers: Vec<String>,
    pub after_markers: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleSpeedPolicy {
    #[default]
    Double,
    Unchanged,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartyProfile {
    pub schema: u32,
    pub id: String,
    /// Optional quest/support templates for a non-Rider workflow.
    #[serde(default)]
    pub farm_templates: Option<FarmTemplateProfile>,
    pub opening_markers: Vec<String>,
    pub support_marker: String,
    pub support_field: FieldSlot,
    /// Explicit opt-in for the recognized CN skill-use confirmation dialog.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm_skill_use: bool,
    #[serde(default)]
    pub battle_speed: BattleSpeedPolicy,
    #[serde(default)]
    pub enemy_target: EnemyTargetStrategy,
    #[serde(default)]
    pub cards: CardStrategyProfile,
    #[serde(default)]
    pub servants: Vec<ServantReference>,
    /// Allowed dynamic supports. Their card art identifies the selected support
    /// without requiring a fixed Atlas ID for the support party slot.
    #[serde(default)]
    pub support_candidates: Vec<SupportServantReference>,
    #[serde(default)]
    pub skill_rules: Vec<ServantSkillRule>,
    #[serde(default)]
    pub master_skill_rules: Vec<MasterSkillRule>,
    pub opening_skills_ready: Vec<SkillRequirement>,
    #[serde(default)]
    pub order_change: Option<OrderChangeProfile>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FarmTemplateProfile {
    pub event: String,
    pub quest: String,
    pub formation_support: String,
    pub support_filter: String,
    pub support_tab: String,
    /// Name template searched only beside a matching 120/120 badge.
    pub support_name: String,
    /// Optional profile-specific 120/120 badge used on support rows.
    #[serde(default)]
    pub support_level: Option<String>,
}

fn validate_skill_target(profile_id: &str, target: SkillRuleTarget) -> Result<()> {
    match target {
        SkillRuleTarget::LowestNp { lowest_np } => ensure!(
            (1..=300).contains(&lowest_np.below),
            "party profile {profile_id} lowest-NP target threshold must be in 1..300"
        ),
        SkillRuleTarget::LowestHp { lowest_hp } => ensure!(
            (1..=999_999).contains(&lowest_hp.below),
            "party profile {profile_id} lowest-HP target threshold must be in 1..999999"
        ),
        SkillRuleTarget::Ally(_) | SkillRuleTarget::Enemy { .. } => {}
    }
    Ok(())
}

fn validate_skill_condition_alternatives(
    profile_id: &str,
    alternatives: &[SkillRuleConditionAlternative],
) -> Result<()> {
    for (index, alternative) in alternatives.iter().enumerate() {
        ensure!(
            !alternative.is_empty(),
            "party profile {profile_id} skill condition alternative {} is empty",
            index + 1
        );
        if let Some(condition) = &alternative.hp_below {
            ensure!(
                (1..=999_999).contains(&condition.hp),
                "party profile {profile_id} HP threshold must be in 1..999999"
            );
        }
        if let Some(condition) = &alternative.np_below {
            ensure!(
                (1..=300).contains(&condition.percent),
                "party profile {profile_id} NP threshold must be in 1..300"
            );
        }
    }
    Ok(())
}

impl PartyProfile {
    pub fn load(path: &Path) -> Result<Self> {
        let profile: Self = serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("read profile {}", path.display()))?,
        )
        .with_context(|| format!("parse profile {}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == PROFILE_SCHEMA_V1,
            "unsupported party profile schema {}",
            self.schema
        );
        ensure!(!self.id.trim().is_empty(), "party profile id is empty");
        if let Some(farm) = &self.farm_templates {
            ensure!(
                [
                    &farm.event,
                    &farm.quest,
                    &farm.formation_support,
                    &farm.support_filter,
                    &farm.support_tab,
                    &farm.support_name
                ]
                .iter()
                .all(|name| !name.trim().is_empty()),
                "farm template names must not be empty"
            );
        }
        ensure!(
            !self.opening_markers.is_empty(),
            "party profile {} has no opening markers",
            self.id
        );
        ensure!(
            !self.support_marker.trim().is_empty(),
            "party profile {} has an empty support marker",
            self.id
        );
        ensure!(
            !self.opening_skills_ready.is_empty(),
            "party profile {} has no opening skill requirements",
            self.id
        );
        self.cards.validate(&self.id)?;
        let mut skills = std::collections::HashSet::new();
        for rule in &self.skill_rules {
            validate_skill_target(&self.id, rule.target)?;
            if let Some(id) = rule.atlas_skill_id {
                ensure!(
                    id > 0,
                    "party profile {} has an invalid Atlas skill ID",
                    self.id
                );
                ensure!(
                    self.servants
                        .iter()
                        .any(|servant| servant.slot == rule.servant),
                    "party profile {} Atlas skill references require a fixed servant ID for slot {}",
                    self.id,
                    rule.servant.index() + 1
                );
            }

            ensure!(
                skills.insert((rule.servant.index(), rule.skill.index())),
                "party profile {} repeats a servant skill rule",
                self.id
            );
            if let Some(condition) = &rule.hp_below {
                ensure!(
                    (1..=999_999).contains(&condition.hp),
                    "party profile {} HP threshold must be in 1..999999",
                    self.id
                );
            }
            if let Some(condition) = &rule.np_below {
                ensure!(
                    (1..=300).contains(&condition.percent),
                    "party profile {} NP threshold must be in 1..300",
                    self.id
                );
            }
            if let Some(timing) = &rule.earliest {
                ensure!(
                    (1..=3).contains(&timing.wave) && timing.turn > 0,
                    "party profile {} skill timing requires wave 1..3 and positive turn",
                    self.id
                );
            }
            validate_skill_condition_alternatives(&self.id, &rule.any_of)?;
        }
        let mut master_skills = std::collections::HashSet::new();
        for rule in &self.master_skill_rules {
            validate_skill_target(&self.id, rule.target)?;
            ensure!(
                self.order_change
                    .as_ref()
                    .is_none_or(|order| rule.skill != order.master_skill),
                "party profile {} must use order_change for its configured swap skill",
                self.id
            );
            ensure!(
                master_skills.insert(rule.skill.index()),
                "party profile {} repeats a master skill rule",
                self.id
            );
            if let Some(condition) = &rule.hp_below {
                ensure!(
                    (1..=999_999).contains(&condition.hp),
                    "party profile {} HP threshold must be in 1..999999",
                    self.id
                );
            }
            if let Some(condition) = &rule.np_below {
                ensure!(
                    (1..=300).contains(&condition.percent),
                    "party profile {} NP threshold must be in 1..300",
                    self.id
                );
            }
            if let Some(timing) = &rule.earliest {
                ensure!(
                    (1..=3).contains(&timing.wave) && timing.turn > 0,
                    "party profile {} skill timing requires wave 1..3 and positive turn",
                    self.id
                );
            }
            validate_skill_condition_alternatives(&self.id, &rule.any_of)?;
        }
        let mut servant_slots = [false; 6];
        for servant in &self.servants {
            if let Some(state) = &servant.account_state {
                ensure!(
                    state.level.is_none_or(|level| level > 0),
                    "party profile {} account level must be positive",
                    self.id
                );
                ensure!(
                    state.ascension.is_none_or(|ascension| ascension <= 4),
                    "party profile {} account ascension must be 0..4",
                    self.id
                );
                ensure!(
                    state.quest_phases.keys().all(|id| *id > 0),
                    "party profile {} account quest IDs must be positive",
                    self.id
                );
                ensure!(
                    self.skill_rules
                        .iter()
                        .any(|rule| rule.servant == servant.slot && rule.atlas_skill_id.is_some()),
                    "party profile {} account state requires an explicit Atlas skill reference for slot {}",
                    self.id,
                    servant.slot.index() + 1
                );
            }

            ensure!(
                !servant_slots[servant.slot.index()],
                "party profile {} repeats servant slot {}",
                self.id,
                servant.slot.index() + 1
            );
            servant_slots[servant.slot.index()] = true;
            ensure!(
                servant.atlas_id > 0,
                "party profile {} contains an invalid Atlas servant ID",
                self.id
            );
            ensure!(
                servant.noble_phantasm_id.is_none_or(|id| id > 0),
                "party profile {} contains an invalid Atlas NP ID",
                self.id
            );
        }
        let mut support_ids = std::collections::HashSet::new();
        let mut support_skill_slots = None;
        for servant in &self.support_candidates {
            let mut slots = std::collections::BTreeSet::new();
            for version in &servant.skill_versions {
                ensure!(
                    version.atlas_skill_id > 0,
                    "party profile {} support {} has an invalid Atlas skill ID",
                    self.id,
                    servant.atlas_id
                );
                ensure!(
                    slots.insert(version.skill.index()),
                    "party profile {} support {} repeats a skill version slot",
                    self.id,
                    servant.atlas_id
                );
            }
            if let Some(expected) = &support_skill_slots {
                ensure!(
                    *expected == slots,
                    "party profile {} support candidates must declare the same skill version slots",
                    self.id
                );
            } else {
                support_skill_slots = Some(slots);
            }
            ensure!(
                servant.atlas_id > 0,
                "party profile {} contains an invalid Atlas support ID",
                self.id
            );
            ensure!(
                servant.noble_phantasm_id.is_none_or(|id| id > 0),
                "party profile {} contains an invalid Atlas support NP ID",
                self.id
            );
            ensure!(
                support_ids.insert(servant.atlas_id),
                "party profile {} repeats Atlas support ID {}",
                self.id,
                servant.atlas_id
            );
        }
        if let Some(order) = &self.order_change {
            ensure!(
                order.field.index() < 3,
                "party profile {} order-change field member must be in slots 1..3",
                self.id
            );
            ensure!(
                order.reserve.index() >= 3,
                "party profile {} order-change reserve member must be in slots 4..6",
                self.id
            );
            ensure!(
                !order.before_markers.is_empty() && !order.after_markers.is_empty(),
                "party profile {} order-change markers are empty",
                self.id
            );
        }
        ensure!(
            self.required_markers()
                .iter()
                .all(|name| !name.trim().is_empty()),
            "party profile {} contains an empty marker",
            self.id
        );
        Ok(())
    }

    pub fn required_templates(&self) -> Vec<String> {
        let mut names = self.required_markers();
        if let Some(farm) = &self.farm_templates {
            names.extend(
                [
                    &farm.event,
                    &farm.quest,
                    &farm.formation_support,
                    &farm.support_filter,
                    &farm.support_tab,
                    &farm.support_name,
                ]
                .into_iter()
                .cloned(),
            );
            if let Some(level) = &farm.support_level {
                names.push(level.clone());
            }
        }
        if self.confirm_skill_use {
            names.extend(
                [
                    "skill_confirm_header",
                    "skill_confirm_cancel",
                    "skill_confirm_button",
                    "skill_none_2",
                ]
                .map(str::to_owned),
            );
        }
        names
    }

    pub fn required_markers(&self) -> Vec<String> {
        let mut markers = self.opening_markers.clone();
        markers.push(self.support_marker.clone());
        if let Some(order) = &self.order_change {
            markers.extend(order.before_markers.iter().cloned());
            markers.extend(order.after_markers.iter().cloned());
        }
        markers.sort();
        markers.dedup();
        markers
    }

    pub fn atlas_servant_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self
            .servants
            .iter()
            .map(|servant| servant.atlas_id)
            .chain(
                self.support_candidates
                    .iter()
                    .map(|servant| servant.atlas_id),
            )
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    pub const fn support_party_slot(&self) -> PartySlot {
        match self.support_field {
            FieldSlot::One => PartySlot::One,
            FieldSlot::Two => PartySlot::Two,
            FieldSlot::Three => PartySlot::Three,
        }
    }
}

pub static RIDER_PROFILE_V1: LazyLock<PartyProfile> = LazyLock::new(|| {
    let profile: PartyProfile = serde_json::from_str(include_str!("../profiles/rider-v1.json"))
        .expect("embedded rider-v1 profile must be valid JSON");
    profile
        .validate()
        .expect("embedded rider-v1 profile must be valid");
    profile
});

impl Default for PartyProfile {
    fn default() -> Self {
        RIDER_PROFILE_V1.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_skill_versions_validate_each_candidate_and_preserve_old_profiles() {
        let mut profile = PartyProfile::default();
        assert!(profile.support_candidates[0].skill_versions.is_empty());
        assert!(
            serde_json::to_value(&profile).unwrap()["support_candidates"][0]
                .get("skill_versions")
                .is_none()
        );
        profile.support_candidates[0]
            .skill_versions
            .push(SupportSkillVersion {
                skill: SkillSlot::One,
                atlas_skill_id: 123,
            });
        let mut candidate = profile.support_candidates[0].clone();
        candidate.atlas_id += 1;
        candidate.skill_versions[0].atlas_skill_id = 456;
        profile.support_candidates.push(candidate);
        profile.validate().unwrap();
        let decoded: PartyProfile =
            serde_json::from_value(serde_json::to_value(&profile).unwrap()).unwrap();
        assert_eq!(decoded, profile);
        profile.support_candidates[1].skill_versions.clear();
        assert!(
            profile
                .validate()
                .unwrap_err()
                .to_string()
                .contains("same skill version slots")
        );
        profile.support_candidates.pop();
        profile.support_candidates[0]
            .skill_versions
            .push(SupportSkillVersion {
                skill: SkillSlot::One,
                atlas_skill_id: 456,
            });
        assert!(
            profile
                .validate()
                .unwrap_err()
                .to_string()
                .contains("repeats a skill version slot")
        );
        profile.support_candidates[0].skill_versions.pop();
        profile.support_candidates[0].skill_versions[0].atlas_skill_id = 0;
        assert!(
            profile
                .validate()
                .unwrap_err()
                .to_string()
                .contains("invalid Atlas skill ID")
        );
        assert!(
            serde_json::from_str::<SupportSkillVersion>(r#"{"skill":"four","atlas_skill_id":123}"#)
                .is_err()
        );
    }

    #[test]
    fn omitted_speed_defaults_to_double_and_explicit_unchanged_round_trips() {
        let profile = PartyProfile::default();
        assert_eq!(profile.battle_speed, BattleSpeedPolicy::Double);
        let mut value = serde_json::to_value(profile).unwrap();
        value.as_object_mut().unwrap().remove("battle_speed");
        assert_eq!(
            serde_json::from_value::<PartyProfile>(value.clone())
                .unwrap()
                .battle_speed,
            BattleSpeedPolicy::Double
        );
        value["battle_speed"] = "unchanged".into();
        assert_eq!(
            serde_json::from_value::<PartyProfile>(value.clone())
                .unwrap()
                .battle_speed,
            BattleSpeedPolicy::Unchanged
        );
        value["battle_speed"] = "normal".into();
        assert!(serde_json::from_value::<PartyProfile>(value).is_err());
    }

    #[test]
    fn master_targets_keep_legacy_allies_and_reject_invalid_enemy_objects() {
        let ally: MasterSkillRule =
            serde_json::from_str(r#"{"skill":"one","target":"three"}"#).unwrap();
        assert_eq!(ally.target, MasterSkillTarget::Ally(PartySlot::Three));
        let enemy: MasterSkillRule =
            serde_json::from_str(r#"{"skill":"two","target":{"enemy":"six"}}"#).unwrap();
        assert_eq!(
            enemy.target,
            MasterSkillTarget::Enemy {
                enemy: crate::domain::EnemySlot::Six
            }
        );
        let lowest_np: MasterSkillRule =
            serde_json::from_str(r#"{"skill":"one","target":{"lowest_np":{"below":100}}}"#)
                .unwrap();
        assert_eq!(
            lowest_np.target,
            MasterSkillTarget::LowestNp {
                lowest_np: LowestNpTarget { below: 100 }
            }
        );
        let lowest_hp: MasterSkillRule =
            serde_json::from_str(r#"{"skill":"two","target":{"lowest_hp":{"below":6600}}}"#)
                .unwrap();
        assert_eq!(
            lowest_hp.target,
            MasterSkillTarget::LowestHp {
                lowest_hp: LowestHpTarget { below: 6600 }
            }
        );
        let mut invalid = PartyProfile::default();
        let mut invalid_rule = lowest_np.clone();
        invalid_rule.target = MasterSkillTarget::LowestNp {
            lowest_np: LowestNpTarget { below: 0 },
        };
        invalid.master_skill_rules = vec![invalid_rule];
        assert!(invalid.validate().is_err());
        for target in [
            r#"{"enemy":"seven"}"#,
            r#"{"enemy":"one","ally":"two"}"#,
            r#"{"lowest_np":{"below":"100"}}"#,
            r#"{}"#,
        ] {
            assert!(
                serde_json::from_str::<MasterSkillRule>(&format!(
                    r#"{{"skill":"two","target":{target}}}"#
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn absent_order_change_allows_third_skill_without_swap_markers() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("order_change");
        let mut profile: PartyProfile = serde_json::from_value(value).unwrap();
        profile.master_skill_rules = vec![MasterSkillRule {
            skill: SkillSlot::Three,
            target: PartySlot::Three.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        }];
        profile.validate().unwrap();
        assert!(!profile.required_markers().contains(&"reines".into()));
        profile.order_change = RIDER_PROFILE_V1.order_change.clone();
        assert!(profile.validate().is_err());
        profile.order_change.as_mut().unwrap().master_skill = SkillSlot::Two;
        profile.validate().unwrap();
        profile.master_skill_rules[0].skill = SkillSlot::Two;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn account_state_requires_valid_values_and_an_explicit_skill_reference() {
        let mut profile = RIDER_PROFILE_V1.clone();
        profile.servants[0].account_state = Some(Default::default());
        assert!(profile.validate().is_err());
        profile.skill_rules = vec![ServantSkillRule {
            atlas_skill_id: Some(5550),
            servant: PartySlot::One,
            skill: SkillSlot::One,
            target: PartySlot::Three.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        }];
        profile.validate().unwrap();
        profile.servants[0].account_state.as_mut().unwrap().level = Some(0);
        assert!(profile.validate().is_err());
        profile.servants[0].account_state.as_mut().unwrap().level = Some(90);
        profile.servants[0]
            .account_state
            .as_mut()
            .unwrap()
            .ascension = Some(5);
        assert!(profile.validate().is_err());
        profile.servants[0]
            .account_state
            .as_mut()
            .unwrap()
            .ascension = Some(4);
        profile.servants[0]
            .account_state
            .as_mut()
            .unwrap()
            .quest_phases
            .insert(0, 3);
        assert!(profile.validate().is_err());
    }

    #[test]
    fn atlas_skill_references_require_positive_ids_and_fixed_servants() {
        let mut profile = RIDER_PROFILE_V1.clone();
        profile.skill_rules = vec![ServantSkillRule {
            atlas_skill_id: Some(5550),
            servant: PartySlot::One,
            skill: SkillSlot::One,
            target: PartySlot::Three.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        }];
        profile.validate().unwrap();
        profile.skill_rules[0].atlas_skill_id = Some(0);
        assert!(profile.validate().is_err());
        profile.skill_rules[0].atlas_skill_id = Some(5550);
        profile.skill_rules[0].servant = PartySlot::Three;
        assert!(profile.validate().is_err());
        profile.skill_rules[0].atlas_skill_id = None;
        profile.validate().unwrap();
    }

    #[test]
    fn master_rules_validate_schema_and_keep_legacy_defaults() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("master_skill_rules");
        let mut profile: PartyProfile = serde_json::from_value(value).unwrap();
        assert!(profile.master_skill_rules.is_empty());
        let rule = MasterSkillRule {
            skill: SkillSlot::One,
            target: PartySlot::Five.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        };
        profile.master_skill_rules = vec![rule.clone()];
        profile.validate().unwrap();
        profile.master_skill_rules.push(rule);
        assert!(profile.validate().is_err());
        profile.master_skill_rules.pop();
        profile.master_skill_rules[0].skill = SkillSlot::Three;
        assert!(profile.validate().is_err());
        profile.master_skill_rules[0].skill = SkillSlot::Two;
        for (wave, turn) in [(0, 1), (4, 1), (1, 0)] {
            profile.master_skill_rules[0].earliest = Some(SkillTiming { wave, turn });
            assert!(profile.validate().is_err());
        }
    }

    #[test]
    fn skill_rules_validate_timing_duplicates_and_legacy_defaults() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("skill_rules");
        let mut profile: PartyProfile = serde_json::from_value(value).unwrap();
        assert!(profile.skill_rules.is_empty());
        let rule = ServantSkillRule {
            atlas_skill_id: None,
            servant: PartySlot::Five,
            skill: SkillSlot::One,
            target: PartySlot::Two.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: Some(SkillTiming { wave: 2, turn: 1 }),
        };
        profile.skill_rules = vec![rule.clone()];
        profile.validate().unwrap();
        profile.skill_rules.push(rule);
        assert!(profile.validate().is_err());
        profile.skill_rules.pop();
        for (wave, turn) in [(0, 1), (4, 1), (1, 0)] {
            profile.skill_rules[0].earliest = Some(SkillTiming { wave, turn });
            assert!(profile.validate().is_err());
        }
    }

    #[test]
    fn embedded_profile_is_valid_and_markers_are_unique() {
        RIDER_PROFILE_V1.validate().unwrap();
        assert_eq!(
            RIDER_PROFILE_V1.enemy_target,
            EnemyTargetStrategy::HighestHp
        );
        assert_eq!(
            RIDER_PROFILE_V1.required_markers(),
            vec!["iskandar", "reines", "support_slot_3", "taigong"]
        );
    }

    #[test]
    fn rejects_an_invalid_order_change_boundary() {
        let mut profile = RIDER_PROFILE_V1.clone();
        profile.order_change.as_mut().unwrap().reserve = PartySlot::Three;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn profile_without_enemy_strategy_keeps_the_current_target() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("enemy_target");
        let profile: PartyProfile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.enemy_target, EnemyTargetStrategy::Current);
    }

    #[test]
    fn legacy_profile_without_card_strategy_uses_compatible_defaults() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("cards");
        let profile: PartyProfile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.cards, CardStrategyProfile::default());
    }

    #[test]
    fn legacy_profile_without_servant_references_remains_valid() {
        let mut value = serde_json::to_value(RIDER_PROFILE_V1.clone()).unwrap();
        value.as_object_mut().unwrap().remove("servants");
        value.as_object_mut().unwrap().remove("support_candidates");
        let profile: PartyProfile = serde_json::from_value(value).unwrap();
        profile.validate().unwrap();
        assert!(profile.servants.is_empty());
        assert!(profile.atlas_servant_ids().is_empty());
    }

    #[test]
    fn rejects_duplicate_np_identity_or_face_priority() {
        let mut profile = RIDER_PROFILE_V1.clone();
        profile.cards.noble_phantasm_order = Some([
            PartySlot::One,
            PartySlot::One,
            PartySlot::Three,
            PartySlot::Four,
            PartySlot::Five,
            PartySlot::Six,
        ]);
        assert!(profile.validate().is_err());

        let mut profile = RIDER_PROFILE_V1.clone();
        profile.cards.face_card_priority[1] = FaceCardPriority::SupportCount;
        assert!(profile.validate().is_err());

        let mut profile = RIDER_PROFILE_V1.clone();
        profile.cards.color_score[0] = 0;
        assert!(profile.validate().is_err());
    }

    #[test]
    fn servant_references_require_unique_slots_and_positive_ids() {
        let mut profile = RIDER_PROFILE_V1.clone();
        profile.servants.push(profile.servants[0].clone());
        assert!(profile.validate().is_err());

        let mut profile = RIDER_PROFILE_V1.clone();
        profile.servants[0].atlas_id = 0;
        assert!(profile.validate().is_err());

        let mut profile = RIDER_PROFILE_V1.clone();
        profile
            .support_candidates
            .push(profile.support_candidates[0].clone());
        assert!(profile.validate().is_err());

        let mut profile = RIDER_PROFILE_V1.clone();
        profile.support_candidates[0].atlas_id = 0;
        assert!(profile.validate().is_err());
    }
}

#[cfg(test)]
mod np_condition_tests {
    use super::*;

    #[test]
    fn optional_np_conditions_round_trip_and_validate_both_rule_types() {
        for master in [false, true] {
            for percent in [0, 1, 100, 300, 301] {
                let mut value = serde_json::to_value(PartyProfile::default()).unwrap();
                let mut rule = serde_json::json!({
                    "skill": "one", "target": "three",
                    "np_below": { "servant": "five", "percent": percent }
                });
                let key = if master {
                    "master_skill_rules"
                } else {
                    rule["servant"] = "two".into();
                    "skill_rules"
                };
                value[key] = serde_json::json!([rule]);
                let profile: PartyProfile = serde_json::from_value(value).unwrap();
                assert_eq!(profile.validate().is_ok(), (1..=300).contains(&percent));
                let encoded = serde_json::to_value(&profile).unwrap();
                assert_eq!(encoded[key][0]["np_below"]["percent"], percent);
                let mut legacy = encoded;
                legacy[key][0].as_object_mut().unwrap().remove("np_below");
                let legacy: PartyProfile = serde_json::from_value(legacy).unwrap();
                assert!(legacy.validate().is_ok());
            }
        }
    }
}

#[cfg(test)]
mod hp_condition_tests {
    use super::*;
    #[test]
    fn hp_thresholds_validate_for_servant_and_master_rules_and_default_to_absent() {
        for master in [false, true] {
            for hp in [0, 1, 6600, 999999, 1000000] {
                let mut value = serde_json::to_value(PartyProfile::default()).unwrap();
                let mut rule = serde_json::json!({"skill":"one","target":"three","hp_below":{"servant":"five","hp":hp}});
                let key = if master {
                    "master_skill_rules"
                } else {
                    rule["servant"] = "two".into();
                    "skill_rules"
                };
                value[key] = serde_json::json!([rule]);
                let profile: PartyProfile = serde_json::from_value(value).unwrap();
                assert_eq!(profile.validate().is_ok(), (1..=999999).contains(&hp));
                let mut old = serde_json::to_value(profile).unwrap();
                old[key][0].as_object_mut().unwrap().remove("hp_below");
                let old: PartyProfile = serde_json::from_value(old).unwrap();
                assert!(old.validate().is_ok());
            }
        }
    }
}

#[cfg(test)]
mod confirmation_tests {
    use super::*;
    #[test]
    fn confirmation_defaults_off_and_requires_all_anchors_only_when_enabled() {
        let mut profile = PartyProfile::default();
        assert!(!profile.confirm_skill_use);
        assert!(
            serde_json::to_value(&profile)
                .unwrap()
                .get("confirm_skill_use")
                .is_none()
        );
        assert_eq!(profile.required_markers(), profile.required_templates());
        profile.confirm_skill_use = true;
        for name in [
            "skill_confirm_header",
            "skill_confirm_cancel",
            "skill_confirm_button",
            "skill_none_2",
        ] {
            assert!(profile.required_templates().contains(&name.to_owned()));
            assert!(!profile.required_markers().contains(&name.to_owned()));
        }
        let decoded: PartyProfile =
            serde_json::from_value(serde_json::to_value(profile).unwrap()).unwrap();
        assert!(decoded.confirm_skill_use);
    }
}

#[cfg(test)]
mod enemy_charge_condition_tests {
    use super::*;

    #[test]
    fn enemy_charge_condition_is_opt_in_for_servant_and_master_rules() {
        let servant: ServantSkillRule = serde_json::from_value(serde_json::json!({
            "servant": "one",
            "skill": "one",
            "target": "one"
        }))
        .unwrap();
        assert!(!servant.enemy_charge_full);
        assert!(
            serde_json::to_value(&servant)
                .unwrap()
                .get("enemy_charge_full")
                .is_none()
        );

        let master: MasterSkillRule = serde_json::from_value(serde_json::json!({
            "skill": "one",
            "target": "one",
            "enemy_charge_full": true
        }))
        .unwrap();
        assert!(master.enemy_charge_full);
        assert_eq!(
            serde_json::to_value(master).unwrap()["enemy_charge_full"],
            true
        );
    }

    #[test]
    fn any_of_conditions_round_trip_and_reject_empty_alternatives() {
        let mut value = serde_json::to_value(PartyProfile::default()).unwrap();
        value["skill_rules"] = serde_json::json!([{
            "servant": "one",
            "skill": "one",
            "target": "one",
            "any_of": [
                {"np_below": {"servant": "one", "percent": 50}},
                {"enemy_charge_full": true}
            ]
        }]);
        let profile: PartyProfile = serde_json::from_value(value).unwrap();
        profile.validate().unwrap();
        assert_eq!(profile.skill_rules[0].any_of.len(), 2);
        let encoded = serde_json::to_value(profile).unwrap();
        assert_eq!(
            encoded["skill_rules"][0]["any_of"][1]["enemy_charge_full"],
            true
        );

        let invalid = PartyProfile {
            skill_rules: vec![ServantSkillRule {
                atlas_skill_id: None,
                servant: PartySlot::One,
                skill: SkillSlot::One,
                target: PartySlot::One.into(),
                earliest: None,
                np_below: None,
                hp_below: None,
                enemy_charge_full: false,
                any_of: vec![SkillRuleConditionAlternative::default()],
            }],
            ..Default::default()
        };
        assert!(invalid.validate().is_err());
    }
}
