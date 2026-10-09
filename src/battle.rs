//! Verified battle actions. Rhai chooses the strategy; this layer owns transitions.
use crate::{
    domain::{
        Command, EnemySlot, FieldSlot, PartySlot, Scene, SkillSlot, Target, VerificationStatus,
    },
    executor::Executor,
    layout::UI_LAYOUT_V1,
    observation::{FieldEvidence, Recognition, SceneFacts},
    profile::{
        BattleSpeedPolicy, CardStrategyProfile, EnemyTargetStrategy, FaceCardPriority,
        HpBelowCondition, NpBelowCondition, PartyProfile, RIDER_PROFILE_V1,
    },
    verification::{ActionVerificationContext, StabilityRule, VerificationProgress},
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::collections::BTreeSet;

const PORTRAIT_CHANGE_THRESHOLD: f64 = 0.05;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Arts,
    Quick,
    Buster,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    Available,
    Cooldown,
    Disabled,
}

impl SkillState {
    const fn name(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Cooldown => "cooldown",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BattleSpeed {
    Normal,
    Double,
}

/// Expensive battle facts requested by the action currently being verified.
/// Scene-safety detectors always run; these flags control additional decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequiredBattleFacts(u16);

impl RequiredBattleFacts {
    pub const WAVE: Self = Self(1 << 0);
    pub const MARKERS: Self = Self(1 << 1);
    pub const ROSTER: Self = Self(1 << 2);
    pub const ENEMIES: Self = Self(1 << 3);
    pub const SKILLS: Self = Self(1 << 4);
    pub const CARDS: Self = Self(1 << 5);
    pub const NP: Self = Self(1 << 6);
    pub const HP: Self = Self(1 << 7);
    pub const SPEED: Self = Self(1 << 8);
    pub const TRACKING: Self = Self(Self::WAVE.0 | Self::ROSTER.0);
    pub const ALL: Self = Self((1 << 9) - 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BattleObservationRequest<'a> {
    pub facts: RequiredBattleFacts,
    pub expected_scenes: &'a [Scene],
}

impl<'a> BattleObservationRequest<'a> {
    pub const fn new(
        facts: RequiredBattleFacts,
        expected_scenes: &'a [Scene],
    ) -> BattleObservationRequest<'a> {
        Self {
            facts,
            expected_scenes,
        }
    }
}

/// Frame-local identity for face cards belonging to the same servant.
///
/// Values are stable only within one command-card observation and are not
/// servant IDs.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct CardGroup(u8);

impl CardGroup {
    pub const fn id(self) -> u8 {
        self.0
    }

    pub const fn from_id(id: u8) -> Option<Self> {
        if id < 5 { Some(Self(id)) } else { None }
    }

    pub(crate) fn from_index(index: usize) -> Self {
        Self::from_id(u8::try_from(index).expect("five face-card groups fit in u8"))
            .expect("card group index is in 0..5")
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct BattleWave {
    pub current: u8,
    pub total: u8,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct EnemyCharge {
    pub current: u8,
    pub total: u8,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct EnemyState {
    pub slot: EnemySlot,
    pub hp: u32,
    pub charge: Option<EnemyCharge>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnemyLayout {
    Standard,
    Six,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BattleOutcome {
    Victory,
    Defeated,
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitFingerprint {
    pub(crate) samples: [[u8; 3]; 64],
}

impl PortraitFingerprint {
    pub(crate) fn difference(&self, other: &Self) -> f64 {
        let squared_difference: u64 = self
            .samples
            .iter()
            .zip(&other.samples)
            .flat_map(|(left, right)| left.iter().zip(right))
            .map(|(left, right)| (*left as i32 - *right as i32).unsigned_abs().pow(2) as u64)
            .sum();
        squared_difference as f64 / (64.0 * 3.0 * 255.0 * 255.0)
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterObservation {
    pub(crate) portraits: [PortraitFingerprint; 3],
    pub(crate) support: [bool; 3],
}

impl RosterObservation {
    fn changed_slots(&self, next: &Self) -> [bool; 3] {
        std::array::from_fn(|index| {
            self.support[index] != next.support[index]
                || self.portraits[index].difference(&next.portraits[index])
                    > PORTRAIT_CHANGE_THRESHOLD
        })
    }
}

impl BattleWave {
    pub fn new(current: u8, total: u8) -> Result<Self> {
        ensure!(
            total > 0 && current > 0 && current <= total,
            "invalid battle wave {current}/{total}"
        );
        Ok(Self { current, total })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct BattleProgress {
    /// Turns observed since this runtime attached to the battle.
    pub turns_seen: u32,
    /// Current wave remains optional when the supported 1..3 digit pair is not confirmed.
    pub wave: Option<BattleWave>,
    pub wave_turn: u32,
    pub outcome: Option<BattleOutcome>,
    /// Current game positions mapped to their original party slots; `None` means defeated.
    pub positions: [Option<PartySlot>; 6],
}

impl Default for BattleProgress {
    fn default() -> Self {
        Self {
            turns_seen: 0,
            wave: None,
            wave_turn: 0,
            outcome: None,
            positions: [
                Some(PartySlot::One),
                Some(PartySlot::Two),
                Some(PartySlot::Three),
                Some(PartySlot::Four),
                Some(PartySlot::Five),
                Some(PartySlot::Six),
            ],
        }
    }
}

impl BattleProgress {
    pub fn frontline(&self) -> [Option<PartySlot>; 3] {
        [self.positions[0], self.positions[1], self.positions[2]]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TurnPhase {
    #[default]
    Awaiting,
    Ready,
    Resolving,
    Finished,
}

#[derive(Clone, Debug, Default)]
pub struct BattleTracker {
    progress: BattleProgress,
    phase: TurnPhase,
    roster: Option<RosterObservation>,
    refresh_roster: bool,
}

impl BattleTracker {
    pub fn progress(&self) -> &BattleProgress {
        &self.progress
    }

    pub fn reset(&mut self) -> bool {
        let changed =
            self.progress != BattleProgress::default() || self.phase != TurnPhase::Awaiting;
        *self = Self::default();
        changed
    }

    pub fn observe(&mut self, state: &BattleState) -> Result<bool> {
        if let Some(wave) = state.wave {
            BattleWave::new(wave.current, wave.total)?;
        }
        if state.victory || state.defeated {
            let outcome = if state.defeated {
                BattleOutcome::Defeated
            } else {
                BattleOutcome::Victory
            };
            let changed = self.progress.outcome != Some(outcome);
            self.progress.outcome = Some(outcome);
            self.phase = TurnPhase::Finished;
            return Ok(changed);
        }
        if !state.idle() {
            return Ok(false);
        }

        if self.refresh_roster {
            self.roster.clone_from(&state.roster);
            self.refresh_roster = false;
        }
        if self.phase == TurnPhase::Awaiting && self.roster.is_none() {
            self.roster.clone_from(&state.roster);
        }
        if self.phase == TurnPhase::Resolving {
            if let (Some(previous), Some(current)) = (&self.roster, &state.roster) {
                let changed = previous.changed_slots(current);
                for (index, changed) in changed.into_iter().enumerate() {
                    if changed && self.progress.positions[index].is_some() {
                        self.replace_defeated(
                            FieldSlot::try_from(index + 1).map_err(anyhow::Error::msg)?,
                        );
                    }
                }
            }
            self.roster.clone_from(&state.roster);
        }
        if !matches!(self.phase, TurnPhase::Awaiting | TurnPhase::Resolving) {
            return Ok(false);
        }

        self.progress.turns_seen = self.progress.turns_seen.saturating_add(1);
        self.progress.wave_turn = match (self.progress.wave, state.wave) {
            (Some(previous), Some(current)) if previous.current == current.current => {
                self.progress.wave_turn.saturating_add(1)
            }
            (_, Some(_)) => 1,
            _ => 0,
        };
        self.progress.wave = state.wave;
        self.phase = TurnPhase::Ready;
        Ok(true)
    }

    pub fn attack_committed(&mut self) -> bool {
        if self.phase != TurnPhase::Ready {
            return false;
        }
        self.phase = TurnPhase::Resolving;
        true
    }

    pub fn order_change_committed(&mut self, field: PartySlot, reserve: PartySlot) -> Result<bool> {
        ensure!(
            field.index() < 3,
            "order-change field member must be in slots 1..3"
        );
        ensure!(
            reserve.index() >= 3,
            "order-change reserve member must be in slots 4..6"
        );
        self.progress.positions.swap(field.index(), reserve.index());
        self.refresh_roster = true;
        Ok(true)
    }

    /// Apply the game's automatic replacement using the first living reserve.
    pub fn replace_defeated(&mut self, field: FieldSlot) -> bool {
        let Some(reserve) = (3..6).find(|index| self.progress.positions[*index].is_some()) else {
            self.progress.positions[field.index()] = None;
            return true;
        };
        self.progress.positions[field.index()] = self.progress.positions[reserve].take();
        true
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BattleState {
    pub turn: bool,
    /// The command-card scene has enough independent card anchors to be confirmed.
    pub command_cards: bool,
    #[serde(skip_serializing_if = "reading_not_requested")]
    pub battle_speed: Recognition<BattleSpeed>,
    pub first_turn: bool,
    pub wave: Option<BattleWave>,
    /// Present only when every visible enemy in a supported layout was read.
    pub enemy_layout: Option<EnemyLayout>,
    pub enemies: [Option<EnemyState>; 6],
    pub selected_enemy: Option<EnemySlot>,
    pub iskandar: bool,
    pub taigong: bool,
    pub reines: bool,
    pub support: bool,
    pub markers: BTreeSet<String>,
    pub skills: [[Recognition<SkillState>; 3]; 3],
    pub master_open: bool,
    pub master_skills: [Recognition<SkillState>; 3],
    pub order_change: bool,
    pub selected: [bool; 6],
    pub target_count: u8,
    pub skill_error: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub skill_confirmation: bool,
    pub colors: [Option<Color>; 5],
    pub card_groups: [Option<CardGroup>; 5],
    pub card_owners: [Option<PartySlot>; 5],
    /// Atlas identity of the dynamic support, confirmed from one or more
    /// support-marked face cards on the current command-card screen.
    pub support_atlas_id: Option<u32>,
    /// Detailed current-frame result; support_atlas_id is its compatibility projection.
    pub support_identity: Recognition<u32>,
    pub sealed: [bool; 5],
    pub support_cards: [bool; 5],
    pub resist: [f64; 5],
    pub critical: [Recognition<f64>; 5],
    /// Current field-order NP percentages, requested only for numeric conditions.
    #[serde(skip_serializing_if = "readings_not_requested")]
    pub np_percent: [Recognition<u16>; 3],
    /// Absolute current HP, in current field order.
    #[serde(skip_serializing_if = "readings_not_requested")]
    pub ally_hp: [Recognition<u32>; 3],
    pub np_glow: [bool; 3],
    pub np_blocked: [bool; 3],
    pub victory: bool,
    pub defeated: bool,
    #[serde(skip)]
    #[doc(hidden)]
    pub roster: Option<RosterObservation>,
}

fn reading_not_requested<T>(value: &Recognition<T>) -> bool {
    matches!(value, Recognition::NotRequested)
}

fn readings_not_requested<T>(values: &[Recognition<T>; 3]) -> bool {
    values
        .iter()
        .all(|value| matches!(value, Recognition::NotRequested))
}

impl BattleState {
    pub fn cards(&self) -> bool {
        self.command_cards || self.cards_complete()
    }
    pub fn cards_complete(&self) -> bool {
        self.colors.iter().all(Option::is_some)
    }
    pub fn idle(&self) -> bool {
        self.turn
            && !self.cards()
            && !self.order_change
            && self.target_count == 0
            && !self.skill_error
            && !self.skill_confirmation
    }

    pub fn scene(&self) -> Scene {
        self.scene_candidates()
            .first()
            .copied()
            .unwrap_or(Scene::Unknown)
    }

    pub fn scene_candidates(&self) -> Vec<Scene> {
        let mut scenes = Vec::new();
        if self.defeated {
            scenes.push(Scene::Defeated);
        }
        if self.victory {
            scenes.push(Scene::Victory);
        }
        if self.skill_error {
            scenes.push(Scene::UnexpectedDialog);
        }
        if self.skill_confirmation {
            scenes.push(Scene::SkillConfirmation);
        }
        if self.order_change {
            scenes.push(Scene::OrderChange);
        }
        if self.target_count > 0 {
            scenes.push(Scene::SkillTarget);
        }
        if self.cards() {
            scenes.push(Scene::CommandCards);
        }
        if scenes.is_empty() && self.turn {
            scenes.push(Scene::BattleTurn);
        }
        scenes
    }

    pub fn has_marker(&self, name: &str) -> bool {
        self.markers.contains(name)
            || match name {
                // Transitional aliases keep existing observations and replay tests stable.
                "iskandar" => self.iskandar,
                "taigong" => self.taigong,
                "reines" => self.reines,
                "support_slot_3" => self.support,
                _ => false,
            }
    }

    pub fn has_markers<'a>(&self, markers: impl IntoIterator<Item = &'a str>) -> bool {
        markers.into_iter().all(|name| self.has_marker(name))
    }

    pub fn skill_state(&self, servant: usize, skill: usize) -> Option<SkillState> {
        self.skills[servant][skill].value()
    }

    pub fn master_skill_state(&self, skill: usize) -> Option<SkillState> {
        self.master_skills[skill].value()
    }
}

impl SceneFacts for BattleState {
    fn scene_candidates(&self) -> Vec<Scene> {
        BattleState::scene_candidates(self)
    }

    fn unknown_fields(&self) -> Vec<String> {
        let mut fields = Vec::new();
        if self.battle_speed.is_unknown() {
            fields.push("battle_speed".into());
        }
        if self.support_identity.is_unknown() {
            fields.push("support_identity".into());
        }
        for servant in 0..3 {
            if self.ally_hp[servant].is_unknown() {
                fields.push(format!("ally_hp.{}", servant + 1));
            }
            if self.np_percent[servant].is_unknown() {
                fields.push(format!("np_percent.{}", servant + 1));
            }
            for skill in 0..3 {
                if self.skills[servant][skill].is_unknown() {
                    fields.push(format!("skills.{}.{}", servant + 1, skill + 1));
                }
            }
        }
        for skill in 0..3 {
            if self.master_skills[skill].is_unknown() {
                fields.push(format!("master_skills.{}", skill + 1));
            }
        }
        for card in 0..5 {
            if self.critical[card].is_unknown() {
                fields.push(format!("critical.{}", card + 1));
            }
        }
        fields
    }

    fn recognition_evidence(&self) -> Vec<FieldEvidence> {
        let mut evidence = Vec::new();
        if let Some(value) = self.battle_speed.evidence() {
            evidence.push(FieldEvidence {
                field: "battle_speed".into(),
                value: self.battle_speed.value().map(|speed| format!("{speed:?}")),
                evidence: value,
            });
        }
        for servant in 0..3 {
            if let Some(value) = self.ally_hp[servant].evidence() {
                evidence.push(FieldEvidence {
                    field: format!("ally_hp.{}", servant + 1),
                    value: self.ally_hp[servant].value().map(|v| v.to_string()),
                    evidence: value,
                });
            }
            if let Some(value) = self.np_percent[servant].evidence() {
                evidence.push(FieldEvidence {
                    field: format!("np_percent.{}", servant + 1),
                    value: self.np_percent[servant]
                        .value()
                        .map(|value| value.to_string()),
                    evidence: value,
                });
            }
            for skill in 0..3 {
                if let Some(value) = self.skills[servant][skill].evidence() {
                    evidence.push(FieldEvidence {
                        field: format!("skills.{}.{}", servant + 1, skill + 1),
                        value: self.skills[servant][skill]
                            .value()
                            .map(|state| state.name().to_owned()),
                        evidence: value,
                    });
                }
            }
        }
        for skill in 0..3 {
            if let Some(value) = self.master_skills[skill].evidence() {
                evidence.push(FieldEvidence {
                    field: format!("master_skills.{}", skill + 1),
                    value: self.master_skills[skill]
                        .value()
                        .map(|state| state.name().to_owned()),
                    evidence: value,
                });
            }
        }
        for card in 0..5 {
            if let Some(value) = self.critical[card].evidence() {
                evidence.push(FieldEvidence {
                    field: format!("critical.{}", card + 1),
                    value: self.critical[card].value().map(|value| value.to_string()),
                    evidence: value,
                });
            }
        }
        evidence
    }
}

/// Time and screen acquisition are injected so transition tests need no device or real sleeps.
pub trait BattlePort: Executor {
    fn observe(&mut self) -> Result<BattleState>;
    fn observe_required(&mut self, _facts: RequiredBattleFacts) -> Result<BattleState> {
        self.observe()
    }
    fn observe_for(&mut self, request: BattleObservationRequest<'_>) -> Result<BattleState> {
        self.observe_required(request.facts)
    }
    fn profile(&self) -> &PartyProfile {
        &RIDER_PROFILE_V1
    }
    fn battle_progress(&self) -> Option<BattleProgress> {
        None
    }
    fn frontline(&self) -> [Option<PartySlot>; 3] {
        [
            Some(PartySlot::One),
            Some(PartySlot::Two),
            Some(PartySlot::Three),
        ]
    }
    fn noble_phantasm_color(&self, _identity: PartySlot) -> Option<Color> {
        None
    }
    fn atlas_noble_phantasm_color(&self, _atlas_id: u32) -> Option<Color> {
        None
    }
    fn attack_committed(&mut self) -> Result<()> {
        Ok(())
    }
    fn order_change_committed(&mut self, _field: PartySlot, _reserve: PartySlot) -> Result<()> {
        Ok(())
    }
}

pub struct BattleDriver<'a, P: BattlePort + ?Sized> {
    pub port: &'a mut P,
}

#[derive(Clone, Copy, Debug, Default)]
struct SkillRuleConditions<'a> {
    np_below: Option<&'a NpBelowCondition>,
    hp_below: Option<&'a HpBelowCondition>,
    enemy_charge_full: bool,
    any_of: Option<&'a [crate::profile::SkillRuleConditionAlternative]>,
}

#[derive(Clone, Copy, Debug)]
enum SkillTargetPlan {
    Fixed(Target),
    LowestNp { below: u16 },
    LowestHp { below: u32 },
}

impl SkillTargetPlan {
    fn required_facts(self) -> RequiredBattleFacts {
        match self {
            Self::Fixed(Target::Enemy { .. }) => RequiredBattleFacts::ENEMIES,
            Self::LowestNp { .. } => RequiredBattleFacts::NP,
            Self::LowestHp { .. } => RequiredBattleFacts::HP,
            Self::Fixed(Target::Ally { .. }) => RequiredBattleFacts::default(),
        }
    }

    fn resolve(
        self,
        frontline: [Option<PartySlot>; 3],
        state: &BattleState,
    ) -> Result<Option<Target>> {
        match self {
            Self::Fixed(target) => Ok(Some(target)),
            Self::LowestNp { below } => {
                let mut best = None;
                for (index, identity) in frontline.iter().enumerate() {
                    if identity.is_none() {
                        continue;
                    }
                    let value = state.np_percent[index].value().ok_or_else(|| {
                        anyhow::anyhow!(
                            "lowest-NP target requires a confirmed NP for field slot {}",
                            index + 1
                        )
                    })?;
                    ensure!(value <= 300, "NP percentage is outside 0..300");
                    if best.is_none_or(|(_, previous)| value < previous) {
                        best = Some((index, value));
                    }
                }
                let Some((index, value)) = best else {
                    bail!("lowest-NP target has no confirmed frontline member")
                };
                Ok((value < below).then_some(Target::Ally {
                    slot: FieldSlot::try_from(index + 1).map_err(anyhow::Error::msg)?,
                }))
            }
            Self::LowestHp { below } => {
                let mut best = None;
                for (index, identity) in frontline.iter().enumerate() {
                    if identity.is_none() {
                        continue;
                    }
                    let value = state.ally_hp[index].value().ok_or_else(|| {
                        anyhow::anyhow!(
                            "lowest-HP target requires confirmed HP for field slot {}",
                            index + 1
                        )
                    })?;
                    ensure!(value <= 999_999, "HP is outside 0..999999");
                    if best.is_none_or(|(_, previous)| value < previous) {
                        best = Some((index, value));
                    }
                }
                let Some((index, value)) = best else {
                    bail!("lowest-HP target has no confirmed frontline member")
                };
                Ok((value < below).then_some(Target::Ally {
                    slot: FieldSlot::try_from(index + 1).map_err(anyhow::Error::msg)?,
                }))
            }
        }
    }
}

impl<P: BattlePort + ?Sized> BattleDriver<'_, P> {
    fn observe(
        &mut self,
        facts: RequiredBattleFacts,
        expected_scenes: &[Scene],
    ) -> Result<BattleState> {
        let s = self
            .port
            .observe_for(BattleObservationRequest::new(facts, expected_scenes))?;
        ensure!(
            !s.skill_confirmation || expected_scenes.contains(&Scene::SkillConfirmation),
            "unexpected skill confirmation; only an opted-in pending skill can confirm"
        );
        ensure!(!s.defeated, "battle defeated");
        ensure!(!s.skill_error, "skill failed or unrecognized battle dialog");
        Ok(s)
    }

    fn wait(
        &mut self,
        description: &str,
        timeout: u64,
        expected_scenes: &[Scene],
        facts: RequiredBattleFacts,
        predicate: impl Fn(&BattleState) -> bool,
    ) -> Result<BattleState> {
        self.wait_with_stability(
            description,
            timeout,
            expected_scenes,
            facts,
            None,
            StabilityRule::ONCE,
            predicate,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_with_stability(
        &mut self,
        description: &str,
        timeout: u64,
        expected_scenes: &[Scene],
        facts: RequiredBattleFacts,
        transition_from: Option<Scene>,
        stability: StabilityRule,
        predicate: impl Fn(&BattleState) -> bool,
    ) -> Result<BattleState> {
        let mut context = ActionVerificationContext::new(
            description,
            expected_scenes,
            self.port.verification_origin(),
            transition_from,
            self.port.elapsed_ms(),
            timeout,
            stability,
        )?;
        self.port
            .verification(&context, VerificationStatus::Started)?;
        loop {
            let state = self.observe(facts, expected_scenes)?;
            match context.observe(
                self.port.current_frame_id(),
                state.scene(),
                predicate(&state),
                self.port.elapsed_ms(),
            ) {
                VerificationProgress::Succeeded => {
                    self.port
                        .verification(&context, VerificationStatus::Succeeded)?;
                    return Ok(state);
                }
                VerificationProgress::TimedOut => {
                    self.port
                        .verification(&context, VerificationStatus::TimedOut)?;
                    bail!("timed out: {description}");
                }
                VerificationProgress::Pending => {
                    ensure!(!state.victory, "battle ended while {description}");
                }
            }
            self.port.sleep(150)?;
        }
    }

    fn idle(&mut self, facts: RequiredBattleFacts) -> Result<BattleState> {
        self.wait(
            "waiting for skill screen",
            45000,
            &[Scene::BattleTurn],
            facts,
            BattleState::idle,
        )
    }

    pub fn verify_opening(&mut self) -> Result<()> {
        let profile = self.port.profile().clone();
        // Animated level labels can miss a single frame even when the party is
        // correct. Require the complete opening on a fresh frame before input.
        let s = self
            .wait(
                "opening markers",
                5000,
                &[Scene::BattleTurn],
                RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS),
                |s| {
                    s.idle()
                        && s.first_turn
                        && s.has_markers(profile.opening_markers.iter().map(String::as_str))
                },
            )
            .with_context(|| format!("opening does not match party profile {}", profile.id))?;
        ensure!(
            profile.opening_skills_ready.iter().all(|required| s
                .skill_state(required.servant.index(), required.skill.index())
                == Some(SkillState::Available)),
            "party profile {} opening skills are used, disabled, or unknown",
            profile.id
        );
        self.port.log("Opening verified")
    }

    pub fn verify_support(&mut self) -> Result<()> {
        let profile = self.port.profile().clone();
        ensure!(
            self.idle(RequiredBattleFacts::MARKERS)?
                .has_marker(&profile.support_marker),
            "party profile {} support marker is no longer visible",
            profile.id
        );
        Ok(())
    }

    fn master_panel(&mut self, opened: bool, facts: RequiredBattleFacts) -> Result<BattleState> {
        for _ in 0..3 {
            let s = self.idle(facts)?;
            if s.master_open == opened {
                return Ok(s);
            }
            self.port.tap(UI_LAYOUT_V1.battle.master_toggle)?;
            self.port.sleep(500)?;
        }
        let s = self.idle(facts)?;
        ensure!(
            s.master_open == opened,
            "master skill panel failed to change state"
        );
        Ok(s)
    }

    fn target_ally(&mut self, count: u8, slot: FieldSlot) -> Result<()> {
        ensure!(
            (1..=3).contains(&count),
            "unrecognized skill target layout ({count})"
        );
        // Preserve the CN layout mapping in FGO-py: a merged region has one
        // tappable area, while separated regions expose each field servant.
        let target = if count == 1 {
            UI_LAYOUT_V1.battle.ally_target_merged
        } else {
            UI_LAYOUT_V1.battle.ally_targets[slot.index()]
        };
        self.port.tap(target)?;
        self.port.sleep(400)
    }

    fn skill_confirmation(
        &mut self,
        state: &BattleState,
        sent: &mut bool,
        left: &mut bool,
        target_sent: bool,
        deadline_ms: u64,
    ) -> Result<()> {
        if !state.skill_confirmation {
            *left |= *sent;
            return Ok(());
        }
        ensure!(
            self.port.profile().confirm_skill_use && !target_sent && !*left,
            "unexpected or repeated skill confirmation"
        );
        if !*sent {
            ensure!(
                self.port.elapsed_ms() < deadline_ms,
                "skill confirmation arrived after the action deadline"
            );
            self.port.command(Command::ConfirmSkill)?;
            self.port.tap(UI_LAYOUT_V1.battle.skill_confirm)?;
            self.port.sleep(500)?;
            *sent = true;
        }
        Ok(())
    }

    fn hp_condition_ready(
        &self,
        condition: Option<&HpBelowCondition>,
        state: &BattleState,
    ) -> Result<bool> {
        let Some(condition) = condition else {
            return Ok(true);
        };
        let field = self
            .port
            .frontline()
            .iter()
            .position(|slot| *slot == Some(condition.servant))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "HP condition party slot {} is not confirmed on the field",
                    condition.servant.index() + 1
                )
            })?;
        let value = state.ally_hp[field].value().ok_or_else(|| {
            anyhow::anyhow!(
                "HP condition party slot {} is unknown",
                condition.servant.index() + 1
            )
        })?;
        ensure!(value <= 999_999, "HP is outside 0..999999");
        Ok(value < condition.hp)
    }

    fn np_condition_ready(
        &self,
        condition: Option<&NpBelowCondition>,
        state: &BattleState,
    ) -> Result<bool> {
        let Some(condition) = condition else {
            return Ok(true);
        };
        let field = self
            .port
            .frontline()
            .iter()
            .position(|slot| *slot == Some(condition.servant))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "NP condition party slot {} is not confirmed on the field",
                    condition.servant.index() + 1
                )
            })?;
        let value = state.np_percent[field].value().ok_or_else(|| {
            anyhow::anyhow!(
                "NP condition party slot {} percentage is unknown",
                condition.servant.index() + 1
            )
        })?;
        ensure!(value <= 300, "NP percentage is outside 0..300");
        Ok(value < condition.percent)
    }

    fn enemy_charge_condition_ready(&self, enabled: bool, state: &BattleState) -> Result<bool> {
        if !enabled {
            return Ok(true);
        }
        let mut has_enemy = false;
        let mut has_unknown = false;
        for enemy in state.enemies.iter().flatten() {
            has_enemy = true;
            let Some(charge) = enemy.charge else {
                has_unknown = true;
                continue;
            };
            ensure!(
                charge.total > 0 && charge.current <= charge.total,
                "enemy charge is outside its confirmed range"
            );
            if charge.current == charge.total {
                return Ok(true);
            }
        }
        ensure!(has_enemy, "enemy charge condition has no confirmed enemies");
        ensure!(
            !has_unknown,
            "enemy charge condition contains an unknown gauge"
        );
        Ok(false)
    }

    fn skill_condition_alternative_ready(
        &self,
        alternative: &crate::profile::SkillRuleConditionAlternative,
        state: &BattleState,
    ) -> Result<bool> {
        let mut ready = true;
        if alternative.np_below.is_some() {
            ready &= self.np_condition_ready(alternative.np_below.as_ref(), state)?;
        }
        if alternative.hp_below.is_some() {
            ready &= self.hp_condition_ready(alternative.hp_below.as_ref(), state)?;
        }
        if alternative.enemy_charge_full {
            ready &= self.enemy_charge_condition_ready(true, state)?;
        }
        Ok(ready)
    }

    fn any_skill_condition_ready(
        &self,
        alternatives: Option<&[crate::profile::SkillRuleConditionAlternative]>,
        state: &BattleState,
    ) -> Result<bool> {
        let Some(alternatives) = alternatives else {
            return Ok(true);
        };
        ensure!(
            !alternatives.is_empty(),
            "skill condition alternatives must not be empty"
        );
        alternatives.iter().try_fold(false, |matched, alternative| {
            let branch = self.skill_condition_alternative_ready(alternative, state)?;
            Ok(matched || branch)
        })
    }

    /// Run each eligible profile rule once, in declared priority order. Readiness
    /// and cooldown verification remain owned by the existing skill executor.
    pub fn cast_profile_skills(&mut self) -> Result<usize> {
        let rules = self.port.profile().skill_rules.clone();
        if rules.is_empty() {
            return Ok(0);
        }
        self.idle(RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS))?;
        let frontline = self.port.frontline();
        let progress = self.port.battle_progress();
        let mut planned = Vec::new();
        for rule in rules {
            let Some(caster) = frontline.iter().position(|id| *id == Some(rule.servant)) else {
                continue;
            };
            if !skill_timing_ready(rule.earliest.as_ref(), progress.as_ref())? {
                continue;
            }
            let target = plan_skill_target(rule.target, frontline)?;
            planned.push((
                caster + 1,
                rule.skill.index() + 1,
                target,
                rule.np_below,
                rule.hp_below,
                rule.enemy_charge_full,
                rule.any_of,
            ));
        }
        let mut cast = 0;
        for (servant, skill, target, condition, hp_condition, enemy_charge_full, any_of) in planned
        {
            cast += usize::from(self.cast_skill_to(
                servant,
                skill,
                target,
                false,
                SkillRuleConditions {
                    np_below: condition.as_ref(),
                    hp_below: hp_condition.as_ref(),
                    enemy_charge_full,
                    any_of: (!any_of.is_empty()).then_some(any_of.as_slice()),
                },
            )?);
        }
        Ok(cast)
    }

    pub fn cast_skill(&mut self, slot: usize, skill: usize, strict: bool) -> Result<bool> {
        let target = self.port.profile().support_field;
        self.cast_skill_targeted(slot, skill, target, strict)
    }

    pub fn cast_skill_targeted(
        &mut self,
        slot: usize,
        skill: usize,
        target: FieldSlot,
        strict: bool,
    ) -> Result<bool> {
        self.cast_skill_to(
            slot,
            skill,
            SkillTargetPlan::Fixed(Target::Ally { slot: target }),
            strict,
            SkillRuleConditions::default(),
        )
    }

    pub fn cast_skill_on_enemy(
        &mut self,
        slot: usize,
        skill: usize,
        target: EnemySlot,
        strict: bool,
    ) -> Result<bool> {
        self.cast_skill_to(
            slot,
            skill,
            SkillTargetPlan::Fixed(Target::Enemy { slot: target }),
            strict,
            SkillRuleConditions::default(),
        )
    }

    fn cast_skill_to(
        &mut self,
        slot: usize,
        skill: usize,
        target: SkillTargetPlan,
        strict: bool,
        conditions: SkillRuleConditions<'_>,
    ) -> Result<bool> {
        let profile = self.port.profile().clone();
        let servant = FieldSlot::try_from(slot).map_err(anyhow::Error::msg)?;
        let skill_slot = SkillSlot::try_from(skill).map_err(anyhow::Error::msg)?;
        let (i, j) = (servant.index(), skill_slot.index());
        let mut facts = RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS);
        facts = facts.union(target.required_facts());
        if conditions.enemy_charge_full {
            facts = facts.union(RequiredBattleFacts::ENEMIES);
        }
        if let Some(alternatives) = conditions.any_of {
            for alternative in alternatives {
                if alternative.np_below.is_some() {
                    facts = facts.union(RequiredBattleFacts::NP);
                }
                if alternative.hp_below.is_some() {
                    facts = facts.union(RequiredBattleFacts::HP);
                }
                if alternative.enemy_charge_full {
                    facts = facts.union(RequiredBattleFacts::ENEMIES);
                }
            }
        }
        if conditions.hp_below.is_some() {
            facts = facts.union(RequiredBattleFacts::HP);
        }
        if conditions.np_below.is_some() {
            facts = facts.union(RequiredBattleFacts::NP);
        }
        let s = self.master_panel(false, facts)?;
        ensure!(
            s.has_marker(&profile.support_marker),
            "party profile {} support marker is no longer visible",
            profile.id
        );
        match s.skill_state(i, j) {
            Some(SkillState::Available) => {}
            Some(SkillState::Cooldown | SkillState::Disabled) => {
                ensure!(
                    !strict,
                    "required servant {slot} skill {skill} is not ready"
                );
                return Ok(false);
            }
            None => bail!("servant {slot} skill {skill} state is unknown"),
        }
        if !self.np_condition_ready(conditions.np_below, &s)?
            || !self.hp_condition_ready(conditions.hp_below, &s)?
            || !self.enemy_charge_condition_ready(conditions.enemy_charge_full, &s)?
            || !self.any_skill_condition_ready(conditions.any_of, &s)?
        {
            return Ok(false);
        }
        let Some(target) = target.resolve(self.port.frontline(), &s)? else {
            return Ok(false);
        };
        if let Target::Enemy { slot: enemy } = target {
            self.select_enemy(&s, enemy, "Select skill enemy")?;
            let confirmed = self.master_panel(false, facts)?;
            ensure!(
                confirmed.selected_enemy == Some(enemy)
                    && confirmed
                        .enemies
                        .iter()
                        .flatten()
                        .any(|value| value.slot == enemy)
                    && confirmed.skill_state(i, j) == Some(SkillState::Available)
                    && confirmed.has_marker(&profile.support_marker),
                "enemy target or servant skill readiness changed before cast"
            );
            if !self.np_condition_ready(conditions.np_below, &confirmed)?
                || !self.hp_condition_ready(conditions.hp_below, &confirmed)?
                || !self.enemy_charge_condition_ready(conditions.enemy_charge_full, &confirmed)?
                || !self.any_skill_condition_ready(conditions.any_of, &confirmed)?
            {
                return Ok(false);
            }
        }
        self.port.command(Command::CastServantSkill {
            servant,
            skill: skill_slot,
            target,
            strict,
        })?;
        let target_description = match target {
            Target::Ally { slot } => format!("ally target: field slot {}", slot.index() + 1),
            Target::Enemy { slot } => format!("enemy target: slot {}", slot.index() + 1),
        };
        self.port.log(&format!(
            "Servant {slot} skill {skill}; {target_description}"
        ))?;
        self.port.tap(UI_LAYOUT_V1.battle.servant_skills[i][j])?;
        self.port.sleep(800)?;
        let mut expected_scenes = vec![Scene::SkillTarget, Scene::BattleTurn];
        if profile.confirm_skill_use {
            expected_scenes.push(Scene::SkillConfirmation);
        }
        let description = format!("servant {slot} skill {skill} entering cooldown");
        let mut verification = ActionVerificationContext::new(
            &description,
            &expected_scenes,
            self.port.verification_origin(),
            Some(Scene::BattleTurn),
            self.port.elapsed_ms(),
            35000,
            StabilityRule::ONCE,
        )?;
        self.port
            .verification(&verification, VerificationStatus::Started)?;
        let mut target_sent = false;
        let (mut confirmation_sent, mut confirmation_left) = (false, false);
        loop {
            let s = self.observe(RequiredBattleFacts::SKILLS, &expected_scenes)?;
            ensure!(!s.victory, "battle ended during skill cast");
            ensure!(
                !s.order_change,
                "servant skill opened an unexpected order-change dialog"
            );
            self.skill_confirmation(
                &s,
                &mut confirmation_sent,
                &mut confirmation_left,
                target_sent,
                verification.deadline_ms,
            )?;
            if s.target_count > 0 && !target_sent {
                let Target::Ally { slot } = target else {
                    bail!("enemy servant skill opened an unexpected ally-target dialog");
                };
                self.target_ally(s.target_count, slot)?;
                target_sent = true;
            }
            match verification.observe(
                self.port.current_frame_id(),
                s.scene(),
                s.idle() && s.skill_state(i, j) == Some(SkillState::Cooldown),
                self.port.elapsed_ms(),
            ) {
                VerificationProgress::Succeeded => {
                    self.port
                        .verification(&verification, VerificationStatus::Succeeded)?;
                    self.port.sleep(300)?;
                    return Ok(true);
                }
                VerificationProgress::TimedOut => {
                    self.port
                        .verification(&verification, VerificationStatus::TimedOut)?;
                    bail!("servant {slot} skill {skill} did not enter cooldown");
                }
                VerificationProgress::Pending => {}
            }
            self.port.sleep(150)?;
        }
    }

    fn open_order_change_with_confirmation(&mut self) -> Result<BattleState> {
        let expected = [Scene::OrderChange, Scene::SkillConfirmation];
        let mut verification = ActionVerificationContext::new(
            "opening order change",
            &expected,
            self.port.verification_origin(),
            Some(Scene::BattleTurn),
            self.port.elapsed_ms(),
            15000,
            StabilityRule::ONCE,
        )?;
        self.port
            .verification(&verification, VerificationStatus::Started)?;
        let (mut sent, mut left) = (false, false);
        loop {
            let s = self.observe(RequiredBattleFacts::default(), &expected)?;
            self.skill_confirmation(&s, &mut sent, &mut left, false, verification.deadline_ms)?;
            match verification.observe(
                self.port.current_frame_id(),
                s.scene(),
                s.order_change,
                self.port.elapsed_ms(),
            ) {
                VerificationProgress::Succeeded => {
                    self.port
                        .verification(&verification, VerificationStatus::Succeeded)?;
                    return Ok(s);
                }
                VerificationProgress::TimedOut => {
                    self.port
                        .verification(&verification, VerificationStatus::TimedOut)?;
                    bail!("timed out: opening order change");
                }
                VerificationProgress::Pending => {}
            }
            self.port.sleep(150)?;
        }
    }

    pub fn order_change(&mut self) -> Result<()> {
        let profile = self.port.profile().clone();
        let order = profile.order_change.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "party profile {} has no order change configured",
                profile.id
            )
        })?;
        let master_skill = order.master_skill.index();
        let field = order.field.index();
        let reserve = order.reserve.index();
        let facts = RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS);
        let s = self.master_panel(true, facts)?;
        ensure!(
            s.has_markers(order.before_markers.iter().map(String::as_str))
                && s.master_skill_state(master_skill) == Some(SkillState::Available),
            "order change precondition does not match party profile {}",
            profile.id
        );
        self.port.command(Command::OrderChange {
            field: order.field,
            reserve: order.reserve,
        })?;
        self.port.log(&format!(
            "Order change: field slot {} <-> reserve slot {}",
            field + 1,
            reserve + 1
        ))?;
        self.port
            .tap(UI_LAYOUT_V1.battle.master_skills[master_skill])?;
        self.port.sleep(500)?;
        let s = if profile.confirm_skill_use {
            self.open_order_change_with_confirmation()?
        } else {
            self.wait(
                "opening order change",
                15000,
                &[Scene::OrderChange],
                RequiredBattleFacts::default(),
                |s| s.order_change,
            )?
        };
        ensure!(
            s.selected.iter().all(|v| !v),
            "order change already has selected servants"
        );
        self.port.tap(UI_LAYOUT_V1.battle.order_members[field])?;
        self.port.sleep(400)?;
        let s = self.wait(
            "selecting order change slot 1",
            5000,
            &[Scene::OrderChange],
            RequiredBattleFacts::default(),
            |s| s.order_change && s.selected[field],
        )?;
        ensure!(
            s.selected.iter().filter(|v| **v).count() == 1,
            "unexpected reserve selection"
        );
        self.port.tap(UI_LAYOUT_V1.battle.order_members[reserve])?;
        self.port.sleep(400)?;
        let s = self.wait(
            "selecting order change slot 5",
            5000,
            &[Scene::OrderChange],
            RequiredBattleFacts::default(),
            |s| s.order_change && s.selected[field] && s.selected[reserve],
        )?;
        ensure!(
            s.selected.iter().filter(|v| **v).count() == 2,
            "unexpected order change selections"
        );
        self.port.tap(UI_LAYOUT_V1.battle.order_confirm)?;
        self.port.sleep(1500)?;
        self.wait(
            "confirming order change result",
            45000,
            &[Scene::BattleTurn],
            RequiredBattleFacts::MARKERS,
            |s| s.idle() && s.has_markers(order.after_markers.iter().map(String::as_str)),
        )?;
        self.port
            .order_change_committed(order.field, order.reserve)?;
        self.port.sleep(500)
    }

    /// Cast eligible ordinary master skills in Profile order. Rhai controls the
    /// ordering between servant skills, master skills, and order change.
    pub fn cast_profile_master_skills(&mut self) -> Result<usize> {
        let rules = self.port.profile().master_skill_rules.clone();
        if rules.is_empty() {
            return Ok(0);
        }
        self.idle(RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS))?;
        let frontline = self.port.frontline();
        let progress = self.port.battle_progress();
        let mut planned = Vec::new();
        for rule in rules {
            ensure!(
                self.port
                    .profile()
                    .order_change
                    .as_ref()
                    .is_none_or(|order| rule.skill != order.master_skill),
                "use order_change() for the configured swap skill"
            );
            if !skill_timing_ready(rule.earliest.as_ref(), progress.as_ref())? {
                continue;
            }
            let target = plan_skill_target(rule.target, frontline)?;
            planned.push((
                rule.skill.index() + 1,
                target,
                rule.np_below,
                rule.hp_below,
                rule.enemy_charge_full,
                rule.any_of,
            ));
        }
        let mut cast = 0;
        for (skill, target, condition, hp_condition, enemy_charge_full, any_of) in planned {
            cast += usize::from(self.cast_master_skill_to(
                skill,
                target,
                false,
                SkillRuleConditions {
                    np_below: condition.as_ref(),
                    hp_below: hp_condition.as_ref(),
                    enemy_charge_full,
                    any_of: (!any_of.is_empty()).then_some(any_of.as_slice()),
                },
            )?);
        }
        Ok(cast)
    }

    pub fn cast_master_skill(&mut self, skill: usize, strict: bool) -> Result<bool> {
        let target = self.port.profile().support_field;
        self.cast_master_skill_targeted(skill, target, strict)
    }

    pub fn cast_master_skill_targeted(
        &mut self,
        skill: usize,
        target: FieldSlot,
        strict: bool,
    ) -> Result<bool> {
        self.cast_master_skill_to(
            skill,
            SkillTargetPlan::Fixed(Target::Ally { slot: target }),
            strict,
            SkillRuleConditions::default(),
        )
    }

    pub fn cast_master_skill_on_enemy(
        &mut self,
        skill: usize,
        target: EnemySlot,
        strict: bool,
    ) -> Result<bool> {
        self.cast_master_skill_to(
            skill,
            SkillTargetPlan::Fixed(Target::Enemy { slot: target }),
            strict,
            SkillRuleConditions::default(),
        )
    }

    fn cast_master_skill_to(
        &mut self,
        skill: usize,
        target: SkillTargetPlan,
        strict: bool,
        conditions: SkillRuleConditions<'_>,
    ) -> Result<bool> {
        let profile = self.port.profile().clone();
        let skill_slot = SkillSlot::try_from(skill).map_err(anyhow::Error::msg)?;
        ensure!(
            profile
                .order_change
                .as_ref()
                .is_none_or(|order| order.master_skill != skill_slot),
            "use order_change() for the configured swap skill"
        );
        let i = skill - 1;
        let mut facts = RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::SKILLS);
        facts = facts.union(target.required_facts());
        if conditions.hp_below.is_some() {
            facts = facts.union(RequiredBattleFacts::HP);
        }
        if conditions.np_below.is_some() {
            facts = facts.union(RequiredBattleFacts::NP);
        }
        if conditions.enemy_charge_full {
            facts = facts.union(RequiredBattleFacts::ENEMIES);
        }
        if let Some(alternatives) = conditions.any_of {
            for alternative in alternatives {
                if alternative.np_below.is_some() {
                    facts = facts.union(RequiredBattleFacts::NP);
                }
                if alternative.hp_below.is_some() {
                    facts = facts.union(RequiredBattleFacts::HP);
                }
                if alternative.enemy_charge_full {
                    facts = facts.union(RequiredBattleFacts::ENEMIES);
                }
            }
        }
        let s = self.master_panel(true, facts)?;
        ensure!(
            s.has_marker(&profile.support_marker),
            "party profile {} support marker is no longer visible",
            profile.id
        );
        match s.master_skill_state(i) {
            Some(SkillState::Available) => {}
            Some(SkillState::Cooldown | SkillState::Disabled) => {
                self.master_panel(false, RequiredBattleFacts::SKILLS)?;
                ensure!(!strict, "required master skill {skill} is not ready");
                return Ok(false);
            }
            None => {
                self.master_panel(false, RequiredBattleFacts::SKILLS)?;
                bail!("master skill {skill} state is unknown");
            }
        }
        if !self.np_condition_ready(conditions.np_below, &s)?
            || !self.hp_condition_ready(conditions.hp_below, &s)?
            || !self.enemy_charge_condition_ready(conditions.enemy_charge_full, &s)?
            || !self.any_skill_condition_ready(conditions.any_of, &s)?
        {
            self.master_panel(false, RequiredBattleFacts::SKILLS)?;
            return Ok(false);
        }
        let Some(target) = target.resolve(self.port.frontline(), &s)? else {
            self.master_panel(false, RequiredBattleFacts::SKILLS)?;
            return Ok(false);
        };
        if let Target::Enemy { slot } = target {
            let state = self.master_panel(false, RequiredBattleFacts::ENEMIES)?;
            self.select_enemy(&state, slot, "Select skill enemy")?;
            let state = self.master_panel(true, facts.union(RequiredBattleFacts::ENEMIES))?;
            ensure!(
                state.selected_enemy == Some(slot)
                    && state
                        .enemies
                        .iter()
                        .flatten()
                        .any(|enemy| enemy.slot == slot)
                    && state.master_skill_state(i) == Some(SkillState::Available)
                    && state.has_marker(&profile.support_marker),
                "enemy target or master skill readiness changed before cast"
            );
            if !self.np_condition_ready(conditions.np_below, &state)?
                || !self.hp_condition_ready(conditions.hp_below, &state)?
                || !self.enemy_charge_condition_ready(conditions.enemy_charge_full, &state)?
                || !self.any_skill_condition_ready(conditions.any_of, &state)?
            {
                self.master_panel(false, RequiredBattleFacts::SKILLS)?;
                return Ok(false);
            }
        }
        self.port.command(Command::CastMasterSkill {
            skill: SkillSlot::try_from(skill).map_err(anyhow::Error::msg)?,
            target,
            strict,
        })?;
        self.port.log(&format!("Master skill {skill}"))?;
        self.port.tap(UI_LAYOUT_V1.battle.master_skills[i])?;
        self.port.sleep(800)?;
        let mut expected_scenes = vec![Scene::SkillTarget, Scene::BattleTurn];
        if profile.confirm_skill_use {
            expected_scenes.push(Scene::SkillConfirmation);
        }
        let description = format!("master skill {skill} target and animation");
        let mut verification = ActionVerificationContext::new(
            &description,
            &expected_scenes,
            self.port.verification_origin(),
            Some(Scene::BattleTurn),
            self.port.elapsed_ms(),
            35000,
            StabilityRule::ONCE,
        )?;
        self.port
            .verification(&verification, VerificationStatus::Started)?;
        let mut target_sent = false;
        let (mut confirmation_sent, mut confirmation_left) = (false, false);
        loop {
            let s = self.observe(RequiredBattleFacts::SKILLS, &expected_scenes)?;
            ensure!(!s.victory, "battle ended during master skill");
            ensure!(
                !s.order_change,
                "ordinary master skill opened an unexpected order-change dialog"
            );
            self.skill_confirmation(
                &s,
                &mut confirmation_sent,
                &mut confirmation_left,
                target_sent,
                verification.deadline_ms,
            )?;
            if s.target_count > 0 && !target_sent {
                let Target::Ally { slot } = target else {
                    bail!("enemy master skill opened an unexpected ally-target dialog");
                };
                self.target_ally(s.target_count, slot)?;
                target_sent = true;
            }
            match verification.observe(
                self.port.current_frame_id(),
                s.scene(),
                s.idle(),
                self.port.elapsed_ms(),
            ) {
                VerificationProgress::Succeeded => {
                    self.port
                        .verification(&verification, VerificationStatus::Succeeded)?;
                    break;
                }
                VerificationProgress::TimedOut => {
                    self.port
                        .verification(&verification, VerificationStatus::TimedOut)?;
                    bail!("master skill target/animation timed out");
                }
                VerificationProgress::Pending => {}
            }
            self.port.sleep(150)?;
        }
        self.port.sleep(1000)?;
        let s = self.master_panel(true, RequiredBattleFacts::SKILLS)?;
        ensure!(
            s.master_skill_state(i) == Some(SkillState::Cooldown),
            "master skill {skill} did not enter cooldown"
        );
        self.master_panel(false, RequiredBattleFacts::SKILLS)?;
        Ok(true)
    }

    pub fn attack(&mut self, first: bool) -> Result<()> {
        self.attack_with_mask(first, [true; 3], false).map(|_| ())
    }

    /// Probe command-card availability, then commit only the requested NPs.
    /// An unavailable optional NP returns to the verified skill screen.
    pub fn attack_np(&mut self, slot: usize, required: bool) -> Result<bool> {
        ensure!((1..=3).contains(&slot), "NP field slot must be 1..3");
        let mut mask = [false; 3];
        mask[slot - 1] = true;
        let attacked = self.attack_with_mask(false, mask, true)?;
        ensure!(
            attacked || !required,
            "required NP in field slot {slot} is unavailable"
        );
        Ok(attacked)
    }

    pub fn attack_other_nps(&mut self) -> Result<bool> {
        let mut mask = [true; 3];
        mask[self.port.profile().support_field.index()] = false;
        self.attack_with_mask(false, mask, true)
    }

    fn attack_with_mask(&mut self, first: bool, mask: [bool; 3], probe: bool) -> Result<bool> {
        let profile = self.port.profile().clone();
        let facts = if profile.enemy_target == EnemyTargetStrategy::HighestHp {
            RequiredBattleFacts::MARKERS.union(RequiredBattleFacts::ENEMIES)
        } else {
            RequiredBattleFacts::MARKERS
        };
        let before = self.master_panel(false, facts)?;
        ensure!(
            before.has_marker(&profile.support_marker),
            "party profile {} support marker is no longer visible",
            profile.id
        );
        self.apply_enemy_target(&before, profile.enemy_target)?;
        self.port.command(Command::Attack { opening: first })?;
        self.port.tap(UI_LAYOUT_V1.battle.attack)?;
        self.port.sleep(1800)?;
        let card_facts = if profile.battle_speed == BattleSpeedPolicy::Double {
            RequiredBattleFacts::CARDS.union(RequiredBattleFacts::SPEED)
        } else {
            RequiredBattleFacts::CARDS
        };
        let first_frame = self.wait(
            "opening command cards",
            15000,
            &[Scene::CommandCards],
            card_facts,
            BattleState::cards,
        )?;
        self.port.sleep(150)?;
        let mut current = self.wait(
            "confirming command-card details",
            5000,
            &[Scene::CommandCards],
            card_facts,
            BattleState::cards_complete,
        )?;
        if profile.battle_speed == BattleSpeedPolicy::Double {
            match current.battle_speed.value() {
                Some(BattleSpeed::Double) => {}
                Some(BattleSpeed::Normal) => {
                    ensure!(
                        first_frame.battle_speed.value() == Some(BattleSpeed::Normal),
                        "battle speed is not stably normal; refusing to toggle"
                    );
                    self.port.command(Command::SetDoubleSpeed)?;
                    self.port.tap(UI_LAYOUT_V1.battle.speed_toggle)?;
                    self.port.sleep(300)?;
                    current = self.wait_with_stability(
                        "confirming double battle speed",
                        5000,
                        &[Scene::CommandCards],
                        card_facts,
                        None,
                        StabilityRule::consecutive(2, 150),
                        |state| {
                            state.cards_complete()
                                && state.battle_speed.value() == Some(BattleSpeed::Double)
                        },
                    )?;
                }
                None => bail!("battle speed is unknown; refusing to toggle or select cards"),
            }
        }
        let ready = std::array::from_fn(|i| {
            mask[i]
                && !first_frame.np_blocked[i]
                && !current.np_blocked[i]
                && (first_frame.np_glow[i] || current.np_glow[i])
        });
        if first && !ready.iter().all(|value| *value) {
            self.port.tap(UI_LAYOUT_V1.battle.command_back)?;
            bail!("opening requires all three noble phantasms, ready={ready:?}");
        }
        if probe && !ready.iter().any(|value| *value) {
            self.port.tap(UI_LAYOUT_V1.battle.command_back)?;
            self.port.sleep(300)?;
            self.idle(RequiredBattleFacts::MARKERS)?;
            return Ok(false);
        }
        let frontline = self.port.frontline();
        let observed_support = match (first_frame.support_atlas_id, current.support_atlas_id) {
            (Some(first), Some(current)) if first != current => None,
            (_, Some(atlas_id)) => Some(atlas_id),
            (_, None) => None,
        };
        if observed_support.is_none() && first_frame.support_atlas_id.is_some() {
            self.port.decision(
                "Dynamic support identity unconfirmed in current frame or conflicting across frames; discard previous-frame identity for NP color",
            )?;
        }
        let support_slot = profile.support_party_slot();
        let noble_phantasm_colors = std::array::from_fn(|field| {
            frontline[field].and_then(|identity| {
                (identity == support_slot)
                    .then(|| {
                        observed_support
                            .and_then(|atlas_id| self.port.atlas_noble_phantasm_color(atlas_id))
                    })
                    .flatten()
                    .or_else(|| self.port.noble_phantasm_color(identity))
            })
        });
        let unknown_critical = current
            .critical
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.value().is_none().then_some(index + 1))
            .collect::<Vec<_>>();
        if !unknown_critical.is_empty() {
            self.port.decision(&format!(
                "Critical rates unavailable for face cards {unknown_critical:?}; use 0 only as the compatibility damage-score fallback"
            ))?;
        }
        let unknown_owners = current
            .card_owners
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                selected_card_owner(index + 1, &current, frontline)
                    .is_none()
                    .then_some(index + 1)
            })
            .collect::<Vec<_>>();
        if !unknown_owners.is_empty() {
            self.port.decision(&format!(
                "Owners unavailable or outside current frontline for face cards {unknown_owners:?}; chain scoring uses confirmed current identities only"
            ))?;
        }
        let cards = rank_cards_with_party_data(
            &current,
            ready,
            frontline,
            noble_phantasm_colors,
            &profile.cards,
        )?;
        self.port.log(&format!("Select cards: {cards:?}"))?;
        self.port.command(Command::SelectCards)?;
        for (i, card) in cards.into_iter().enumerate() {
            let p = if card <= 5 {
                UI_LAYOUT_V1.battle.face_cards[card - 1]
            } else {
                UI_LAYOUT_V1.battle.noble_phantasms[card - 6]
            };
            self.port.tap(p)?;
            self.port.sleep(if i == 2 { 1800 } else { 400 })?;
        }
        self.wait_with_stability(
            "starting attack",
            15000,
            &[Scene::BattleAnimation],
            RequiredBattleFacts::default(),
            Some(Scene::CommandCards),
            StabilityRule::consecutive(2, 150),
            |s| !s.turn && !s.cards() && !s.order_change && s.target_count == 0,
        )?;
        self.port.attack_committed()?;
        Ok(true)
    }

    fn apply_enemy_target(
        &mut self,
        state: &BattleState,
        strategy: EnemyTargetStrategy,
    ) -> Result<()> {
        if strategy == EnemyTargetStrategy::Current {
            return Ok(());
        }
        ensure!(
            state.enemy_layout.is_some() && state.enemies.iter().any(Option::is_some),
            "highest-HP targeting requires a complete supported enemy observation"
        );
        let target = state
            .enemies
            .iter()
            .flatten()
            .max_by_key(|enemy| enemy.hp)
            .expect("supported enemy observation is non-empty")
            .slot;
        ensure!(
            state.selected_enemy.is_some(),
            "highest-HP targeting requires a confirmed current target"
        );
        self.select_enemy(state, target, "Select highest-HP enemy")
    }

    fn select_enemy(&mut self, state: &BattleState, target: EnemySlot, label: &str) -> Result<()> {
        ensure!(
            state.enemy_layout.is_some()
                && state.selected_enemy.is_some()
                && state
                    .enemies
                    .iter()
                    .flatten()
                    .any(|enemy| enemy.slot == target),
            "enemy targeting requires a supported layout, a present enemy and a confirmed current target"
        );
        if state.selected_enemy == Some(target) {
            return Ok(());
        }
        self.port.command(Command::SelectEnemy { slot: target })?;
        self.port
            .log(&format!("{label}: slot {}", target.index() + 1))?;
        let point = match state.enemy_layout.expect("supported layout checked") {
            EnemyLayout::Standard => *UI_LAYOUT_V1
                .battle
                .enemy_targets_standard
                .get(target.index())
                .ok_or_else(|| anyhow::anyhow!("standard enemy slot is outside 1..3"))?,
            EnemyLayout::Six => UI_LAYOUT_V1.battle.enemy_targets_six[target.index()],
        };
        self.port.tap(point)?;
        self.port.sleep(300)?;
        self.wait(
            "confirming enemy target",
            5000,
            &[Scene::BattleTurn],
            RequiredBattleFacts::ENEMIES,
            |state| state.idle() && state.selected_enemy == Some(target),
        )?;
        Ok(())
    }

    pub fn wait_next(&mut self) -> Result<bool> {
        let s = self.wait(
            "waiting for next turn or battle result",
            180000,
            &[Scene::BattleTurn, Scene::Victory],
            RequiredBattleFacts::default(),
            |s| s.victory || s.idle(),
        )?;
        Ok(!s.victory)
    }
}

fn plan_skill_target(
    target: crate::profile::SkillRuleTarget,
    frontline: [Option<PartySlot>; 3],
) -> Result<SkillTargetPlan> {
    match target {
        crate::profile::SkillRuleTarget::Ally(identity) => {
            let field = frontline
                .iter()
                .position(|id| *id == Some(identity))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "skill target party slot {} is not in the frontline",
                        identity.index() + 1
                    )
                })?;
            Ok(SkillTargetPlan::Fixed(Target::Ally {
                slot: FieldSlot::try_from(field + 1).map_err(anyhow::Error::msg)?,
            }))
        }
        crate::profile::SkillRuleTarget::Enemy { enemy } => {
            Ok(SkillTargetPlan::Fixed(Target::Enemy { slot: enemy }))
        }
        crate::profile::SkillRuleTarget::LowestNp { lowest_np } => Ok(SkillTargetPlan::LowestNp {
            below: lowest_np.below,
        }),
        crate::profile::SkillRuleTarget::LowestHp { lowest_hp } => Ok(SkillTargetPlan::LowestHp {
            below: lowest_hp.below,
        }),
    }
}

fn skill_timing_ready(
    timing: Option<&crate::profile::SkillTiming>,
    progress: Option<&BattleProgress>,
) -> Result<bool> {
    let Some(timing) = timing else {
        return Ok(true);
    };
    let progress =
        progress.ok_or_else(|| anyhow::anyhow!("skill timing requires battle progress"))?;
    let wave = progress
        .wave
        .ok_or_else(|| anyhow::anyhow!("skill timing requires a known wave"))?;
    ensure!(
        progress.wave_turn > 0,
        "skill timing requires a known wave turn"
    );
    Ok((wave.current, progress.wave_turn) >= (timing.wave.min(wave.total), timing.turn))
}

/// Rank with the default party identity and card strategy. Runtime callers use
/// [`rank_cards_for_party`] so order changes and replacements remain visible.
pub fn rank_cards(state: &BattleState, ready: [bool; 3]) -> Result<[usize; 3]> {
    rank_cards_for_party(
        state,
        ready,
        [
            Some(PartySlot::One),
            Some(PartySlot::Two),
            Some(PartySlot::Three),
        ],
        &CardStrategyProfile::default(),
    )
}

#[derive(Clone, Copy)]
struct FaceCardMetrics {
    support_count: usize,
    brave_chain: bool,
    color_chain: bool,
    damage: f64,
}

fn compare_face_cards(
    left: FaceCardMetrics,
    right: FaceCardMetrics,
    strategy: &CardStrategyProfile,
) -> std::cmp::Ordering {
    for priority in strategy.face_card_priority {
        let ordering = match priority {
            FaceCardPriority::SupportCount => left.support_count.cmp(&right.support_count),
            FaceCardPriority::BraveChain => left.brave_chain.cmp(&right.brave_chain),
            FaceCardPriority::ColorChain => left.color_chain.cmp(&right.color_chain),
            FaceCardPriority::Damage => left.damage.total_cmp(&right.damage),
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    std::cmp::Ordering::Equal
}

fn face_card_orders(faces: &[usize], count: usize) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    for &first in faces {
        if count == 1 {
            result.push(vec![first]);
            continue;
        }
        for &second in faces {
            if second == first {
                continue;
            }
            if count == 2 {
                result.push(vec![first, second]);
                continue;
            }
            for &third in faces {
                if third != first && third != second {
                    result.push(vec![first, second, third]);
                }
            }
        }
    }
    result
}

/// NP cards are ordered by the original party-slot identities maintained by
/// [`BattleTracker`]. Remaining face-card permutations use the profile's
/// lexicographic priorities and damage parameters. Sealed cards are never used
/// as a fallback.
pub fn rank_cards_for_party(
    state: &BattleState,
    ready: [bool; 3],
    frontline: [Option<PartySlot>; 3],
    strategy: &CardStrategyProfile,
) -> Result<[usize; 3]> {
    rank_cards_with_party_data(state, ready, frontline, [None; 3], strategy)
}

pub fn rank_cards_with_party_data(
    state: &BattleState,
    ready: [bool; 3],
    frontline: [Option<PartySlot>; 3],
    noble_phantasm_colors: [Option<Color>; 3],
    strategy: &CardStrategyProfile,
) -> Result<[usize; 3]> {
    ensure!(
        state.cards_complete(),
        "command card colors are not all recognized"
    );
    strategy.validate("runtime card strategy")?;
    let mut noble_phantasms = Vec::new();
    for field in 0..3 {
        if ready[field] {
            let identity = frontline[field]
                .ok_or_else(|| anyhow::anyhow!("ready NP in empty frontline slot {}", field + 1))?;
            noble_phantasms.push((field + 6, identity));
        }
    }
    if let Some(order) = strategy.noble_phantasm_order {
        noble_phantasms.sort_by_key(|(_, identity)| {
            order
                .iter()
                .position(|candidate| candidate == identity)
                .expect("validated profile contains every party slot")
        });
    }
    let mut cards: Vec<_> = noble_phantasms.into_iter().map(|(card, _)| card).collect();
    if cards.len() == 3 {
        return Ok(cards.try_into().expect("exactly three NP cards"));
    }

    let faces: Vec<_> = (0..5).filter(|index| !state.sealed[*index]).collect();
    let needed = 3 - cards.len();
    ensure!(
        faces.len() >= needed,
        "fewer than three usable NP and face cards"
    );
    let mut best: Option<(Vec<usize>, FaceCardMetrics)> = None;
    for order in face_card_orders(&faces, needed) {
        let first_color = cards.first().map_or(state.colors[order[0]], |card| {
            noble_phantasm_colors[*card - 6]
        });
        let first_buster_bonus = if first_color == Some(Color::Buster) {
            strategy.first_buster_bonus as f64 / 100.0
        } else {
            0.0
        };
        let damage = order
            .iter()
            .enumerate()
            .map(|(face_position, index)| {
                let color = match state.colors[*index].expect("validated colors") {
                    Color::Arts => strategy.color_score[0],
                    Color::Quick => strategy.color_score[1],
                    Color::Buster => strategy.color_score[2],
                } as f64
                    / 100.0;
                let card_position = cards.len() + face_position;
                let position = strategy.position_multiplier[card_position] as f64 / 100.0;
                (position * color + first_buster_bonus)
                    * state.resist[*index]
                    * (1.0 + state.critical[*index].value().unwrap_or(0.0))
            })
            .sum();
        let selected: Vec<_> = cards
            .iter()
            .copied()
            .chain(order.iter().map(|index| index + 1))
            .collect();
        let color_chain = selected
            .first()
            .and_then(|card| selected_card_color(*card, state, noble_phantasm_colors))
            .is_some_and(|first| {
                selected.iter().all(|card| {
                    selected_card_color(*card, state, noble_phantasm_colors) == Some(first)
                })
            });
        let identity_brave_chain = selected
            .first()
            .and_then(|card| selected_card_owner(*card, state, frontline))
            .is_some_and(|first| {
                selected
                    .iter()
                    .all(|card| selected_card_owner(*card, state, frontline) == Some(first))
            });
        let local_group_brave_chain = cards.is_empty()
            && state.card_groups[order[0]].is_some()
            && order
                .iter()
                .all(|index| state.card_groups[*index] == state.card_groups[order[0]]);
        let inferred_support = |index: &usize| {
            state.support_cards[*index]
                || (strategy.infer_unknown_support_cards
                    && selected_card_owner(*index + 1, state, frontline).is_none())
        };
        let metrics = FaceCardMetrics {
            support_count: order.iter().filter(|index| inferred_support(index)).count(),
            brave_chain: identity_brave_chain || local_group_brave_chain,
            color_chain,
            damage,
        };
        if best
            .as_ref()
            .is_none_or(|(_, current)| compare_face_cards(metrics, *current, strategy).is_gt())
        {
            best = Some((order, metrics));
        }
    }

    cards.extend(
        best.expect("at least one usable face-card permutation")
            .0
            .into_iter()
            .map(|index| index + 1),
    );
    Ok(cards.try_into().expect("NP and face cards total three"))
}

fn selected_card_color(
    card: usize,
    state: &BattleState,
    noble_phantasm_colors: [Option<Color>; 3],
) -> Option<Color> {
    if card <= 5 {
        state.colors[card - 1]
    } else {
        noble_phantasm_colors[card - 6]
    }
}

fn selected_card_owner(
    card: usize,
    state: &BattleState,
    frontline: [Option<PartySlot>; 3],
) -> Option<PartySlot> {
    if card <= 5 {
        state.card_owners[card - 1].filter(|owner| frontline.contains(&Some(*owner)))
    } else {
        frontline[card - 6]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_at(current: u8, total: u8) -> BattleState {
        BattleState {
            turn: true,
            wave: Some(BattleWave::new(current, total).unwrap()),
            ..Default::default()
        }
    }

    fn roster(values: [u8; 3], support: [bool; 3]) -> RosterObservation {
        RosterObservation {
            portraits: values.map(|value| PortraitFingerprint {
                samples: [[value; 3]; 64],
            }),
            support,
        }
    }

    fn idle_with_roster(values: [u8; 3], support: [bool; 3]) -> BattleState {
        BattleState {
            turn: true,
            wave: Some(BattleWave::new(1, 3).unwrap()),
            roster: Some(roster(values, support)),
            ..Default::default()
        }
    }

    #[test]
    fn tracker_counts_distinct_turns_and_wave_transitions() {
        let mut tracker = BattleTracker::default();
        assert!(tracker.observe(&idle_at(1, 3)).unwrap());
        assert!(!tracker.observe(&idle_at(1, 3)).unwrap());
        assert_eq!(tracker.progress().turns_seen, 1);
        assert_eq!(tracker.progress().wave_turn, 1);

        assert!(tracker.attack_committed());
        assert!(tracker.observe(&idle_at(1, 3)).unwrap());
        assert_eq!(tracker.progress().turns_seen, 2);
        assert_eq!(tracker.progress().wave_turn, 2);

        assert!(tracker.attack_committed());
        assert!(tracker.observe(&idle_at(2, 3)).unwrap());
        assert_eq!(tracker.progress().turns_seen, 3);
        assert_eq!(tracker.progress().wave_turn, 1);
        assert_eq!(tracker.progress().wave.unwrap().current, 2);
    }

    #[test]
    fn tracker_follows_order_change_and_automatic_replacement() {
        let mut tracker = BattleTracker::default();
        tracker
            .order_change_committed(PartySlot::One, PartySlot::Five)
            .unwrap();
        assert_eq!(
            tracker.progress().frontline(),
            [
                Some(PartySlot::Five),
                Some(PartySlot::Two),
                Some(PartySlot::Three)
            ]
        );
        assert!(tracker.replace_defeated(FieldSlot::Two));
        assert_eq!(
            tracker.progress().frontline(),
            [
                Some(PartySlot::Five),
                Some(PartySlot::Four),
                Some(PartySlot::Three)
            ]
        );
        assert_eq!(tracker.progress().positions[3], None);
        assert_eq!(tracker.progress().positions[4], Some(PartySlot::One));
    }

    #[test]
    fn tracker_replaces_changed_portrait_after_an_attack() {
        let mut tracker = BattleTracker::default();
        tracker
            .observe(&idle_with_roster([10, 20, 30], [false; 3]))
            .unwrap();
        assert!(tracker.attack_committed());
        tracker
            .observe(&idle_with_roster([200, 20, 30], [false; 3]))
            .unwrap();
        assert_eq!(
            tracker.progress().frontline(),
            [
                Some(PartySlot::Four),
                Some(PartySlot::Two),
                Some(PartySlot::Three)
            ]
        );
        assert_eq!(tracker.progress().positions[3], None);
    }

    #[test]
    fn tracker_refreshes_portrait_baseline_after_order_change() {
        let mut tracker = BattleTracker::default();
        tracker
            .observe(&idle_with_roster([10, 20, 30], [false; 3]))
            .unwrap();
        tracker
            .order_change_committed(PartySlot::One, PartySlot::Five)
            .unwrap();
        assert!(
            !tracker
                .observe(&idle_with_roster([200, 20, 30], [false; 3]))
                .unwrap()
        );
        assert!(tracker.attack_committed());
        tracker
            .observe(&idle_with_roster([200, 20, 30], [false; 3]))
            .unwrap();
        assert_eq!(
            tracker.progress().frontline(),
            [
                Some(PartySlot::Five),
                Some(PartySlot::Two),
                Some(PartySlot::Three)
            ]
        );
    }

    #[test]
    fn support_marker_change_is_also_a_roster_change() {
        let mut tracker = BattleTracker::default();
        tracker
            .observe(&idle_with_roster([10, 20, 30], [false, false, true]))
            .unwrap();
        assert!(tracker.attack_committed());
        tracker
            .observe(&idle_with_roster([10, 20, 30], [false; 3]))
            .unwrap();
        assert_eq!(
            tracker.progress().frontline(),
            [
                Some(PartySlot::One),
                Some(PartySlot::Two),
                Some(PartySlot::Four)
            ]
        );
    }
}
