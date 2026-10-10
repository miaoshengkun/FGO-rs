use crate::{
    battle::{
        BattleObservationRequest, BattleState, BattleWave, CardGroup, Color, EnemyCharge,
        EnemyLayout, EnemyState, PortraitFingerprint, RequiredBattleFacts, RosterObservation,
        SkillState,
    },
    control::Control,
    digit_ocr::{read_large_white_digits, read_white_digits},
    domain::{EnemySlot, PartySlot, Scene},
    hp_ocr::read_ally_hp,
    layout::UI_LAYOUT_V1,
    np_ocr::read_np_percent,
    observation::{
        DetectorKind, Recognition, RecognitionEvidence, RecognitionMethod, ScoreDirection,
        UnknownReason,
    },
    vision::{FrameMatcher, Point, Rect, Vision},
};
use anyhow::Result;
use image::RgbaImage;

#[derive(Default)]
struct EnemyObservation {
    layout: Option<EnemyLayout>,
    enemies: [Option<EnemyState>; 6],
    selected: Option<EnemySlot>,
}

impl Vision {
    /// CN battle facts computed from one normalized frame; no device input.
    pub fn battle_state(&self, frame: &RgbaImage, control: &Control) -> Result<BattleState> {
        self.battle_state_for_markers(
            frame,
            control,
            ["iskandar", "taigong", "reines", "support_slot_3"],
            RequiredBattleFacts::ALL,
            &[],
        )
    }

    pub fn battle_state_with_markers_and_facts(
        &self,
        frame: &RgbaImage,
        control: &Control,
        marker_names: &[String],
        facts: RequiredBattleFacts,
    ) -> Result<BattleState> {
        self.battle_state_for_markers(
            frame,
            control,
            marker_names.iter().map(String::as_str),
            facts,
            &[],
        )
    }

    pub fn battle_state_with_request(
        &self,
        frame: &RgbaImage,
        control: &Control,
        marker_names: &[String],
        request: BattleObservationRequest<'_>,
    ) -> Result<BattleState> {
        self.battle_state_for_markers(
            frame,
            control,
            marker_names.iter().map(String::as_str),
            request.facts,
            request.expected_scenes,
        )
    }

    fn battle_state_for_markers<'a>(
        &self,
        frame: &RgbaImage,
        control: &Control,
        marker_names: impl IntoIterator<Item = &'a str>,
        facts: RequiredBattleFacts,
        expected_scenes: &[Scene],
    ) -> Result<BattleState> {
        Self::require_canvas(frame)?;
        let mut matcher = self.frame_matcher(frame, control);
        self.prime_expected_battle_scenes(&mut matcher, expected_scenes)?;
        let turn = matcher.has("turn_begin")?;
        let order_change = matcher.has("order_change")?;
        let confirmation_header = matcher.has_optional("skill_confirm_header")?;
        let skill_confirmation = confirmation_header
            && matcher.has_optional("skill_confirm_cancel")?
            && matcher.has_optional("skill_confirm_button")?
            && matcher.has("skill_none_2")?;
        let skill_error = (confirmation_header && !skill_confirmation)
            || matcher.has("skill_error")?
            || matcher.has("skill_none_1")?
            || (!skill_confirmation && matcher.has("skill_none_2")?);
        let victory = matcher.has("victory")?
            || (self.has_template("bond_result") && matcher.has("bond_result")?);
        let defeated = matcher.has("defeated")?;
        let mut s = BattleState {
            turn,
            first_turn: turn
                && (matcher.has("first_turn")? || matcher.has_optional("saber_first_turn")?),
            order_change,
            master_open: turn && (matcher.has("master_ready")? || matcher.has("master_used")?),
            skill_error,
            skill_confirmation,
            victory,
            defeated,
            resist: [1.0; 5],
            ..Default::default()
        };
        if s.order_change {
            for i in 0..6 {
                s.selected[i] = matcher.has(&format!("selected_{}", i + 1))?;
            }
        } else if !s.skill_confirmation && matcher.has("target_cross")? && !s.skill_error {
            if matcher.has("target_header")? {
                s.target_count = target_regions(frame);
            } else {
                s.skill_error = true;
            }
        }
        let can_probe_cards = !s.turn
            && !s.order_change
            && s.target_count == 0
            && !s.skill_error
            && !s.skill_confirmation
            && !s.victory
            && !s.defeated;
        if can_probe_cards {
            let mut colors = [None; 5];
            for (i, detected) in colors.iter_mut().enumerate() {
                let mut best = None;
                for (name, color) in [
                    ("arts", Color::Arts),
                    ("quick", Color::Quick),
                    ("buster", Color::Buster),
                ] {
                    if let Some(m) = matcher.find(&format!("{name}_{}", i + 1))?
                        && best.is_none_or(|(score, _)| m.score < score)
                    {
                        best = Some((m.score, color));
                    }
                }
                *detected = best.map(|(_, color)| color);
                // Decorative card lettering is not reliably read by every OCR
                // model. Keep positive template evidence; use OCR only for a miss.
                #[cfg(feature = "maa")]
                if detected.is_none()
                    && let Some(reading) = self.maa_card_color(frame, i, control)?
                {
                    *detected = reading.value();
                    if facts.contains(RequiredBattleFacts::CARDS) {
                        s.card_color_recognition[i] = reading;
                    }
                }
            }
            s.command_cards = colors.iter().filter(|color| color.is_some()).count() >= 4;
            if facts.contains(RequiredBattleFacts::CARDS) {
                s.colors = colors;
            }
        }
        if facts.contains(RequiredBattleFacts::WAVE) && (s.turn || s.command_cards) {
            s.wave = self.battle_wave(&mut matcher)?;
        }
        if s.idle() {
            if facts.contains(RequiredBattleFacts::HP) {
                s.ally_hp = UI_LAYOUT_V1.battle.ally_hp.map(|roi| {
                    let reading = read_ally_hp(frame, roi);
                    let evidence = RecognitionEvidence {
                        detector: DetectorKind::AllyHp,
                        roi,
                        method: RecognitionMethod::BinaryGlyphDifference,
                        score: reading.map_or(1.0, |v| v.1),
                        threshold: 0.12,
                        direction: ScoreDirection::LowerIsBetter,
                        feature_floor: None,
                    };
                    match reading {
                        Some((value, _)) => Recognition::known(value, evidence),
                        None => Recognition::unknown(UnknownReason::NoCandidate, Some(evidence)),
                    }
                });
            }
            if facts.contains(RequiredBattleFacts::NP) {
                for (roi, reading) in UI_LAYOUT_V1
                    .battle
                    .np_percent
                    .into_iter()
                    .zip(&mut s.np_percent)
                {
                    #[cfg(feature = "maa")]
                    if let Some(native) = self.maa_np_percent(frame, roi, control)? {
                        *reading = native;
                        continue;
                    }
                    *reading = read_np_percent(frame, roi);
                }
            }
            if facts.contains(RequiredBattleFacts::MARKERS) {
                for name in marker_names {
                    if matcher.has(name)? {
                        s.markers.insert(name.to_owned());
                    }
                }
                s.iskandar = s.markers.contains("iskandar");
                s.taigong = s.markers.contains("taigong");
                s.reines = s.markers.contains("reines");
                s.support = s.markers.contains("support_slot_3");
            }

            if facts.contains(RequiredBattleFacts::ROSTER) {
                let mut support_fields = [false; 3];
                for (index, region) in UI_LAYOUT_V1.battle.support_marks.iter().enumerate() {
                    support_fields[index] =
                        matcher.find_in("support_slot_3", Some(*region))?.is_some();
                }
                s.roster = Some(RosterObservation {
                    portraits: UI_LAYOUT_V1
                        .battle
                        .field_portraits
                        .map(|region| portrait_fingerprint(frame, region)),
                    support: support_fields,
                });
            }

            if facts.contains(RequiredBattleFacts::ENEMIES) {
                let mut enemies = self.enemy_states_six(&mut matcher)?;
                if enemies.layout.is_none() {
                    enemies = self.enemy_states_standard(&mut matcher)?;
                }
                s.enemy_layout = enemies.layout;
                s.enemies = enemies.enemies;
                s.selected_enemy = enemies.selected;
            }

            if facts.contains(RequiredBattleFacts::SKILLS) {
                for i in 0..3 {
                    for j in 0..3 {
                        s.skills[i][j] = self.recognize_skill(
                            &mut matcher,
                            &format!("skill_cd_{}_{}", i + 1, j + 1),
                            UI_LAYOUT_V1.battle.servant_skills[i][j],
                            140.0 / 255.0,
                            DetectorKind::ServantSkillCooldown,
                        )?;
                    }
                    if s.master_open {
                        s.master_skills[i] = self.recognize_skill(
                            &mut matcher,
                            &format!("master_cd_{}", i + 1),
                            UI_LAYOUT_V1.battle.master_skills[i],
                            160.0 / 255.0,
                            DetectorKind::MasterSkillCooldown,
                        )?;
                    }
                }
            }
        }
        if s.cards() && facts.contains(RequiredBattleFacts::SPEED) {
            s.battle_speed = crate::battle_speed::read_battle_speed(frame);
        }
        // The gauge sample belongs to the command-card screen, not the skill
        // screen: a bright pixel there before opening cards is not proof of NP readiness.
        if s.cards() && facts.contains(RequiredBattleFacts::CARDS) {
            for i in 0..3 {
                let mut sum = 0u32;
                for y in 679..684 {
                    for x in 144 + 319 * i..156 + 319 * i {
                        let p = frame.get_pixel(x as u32, y);
                        sum += p[0] as u32 + p[1] as u32 + p[2] as u32;
                    }
                }
                s.np_glow[i] = sum as f64 / (12.0 * 5.0 * 3.0) > 55.0;
                if s.cards() {
                    s.np_blocked[i] = matcher.has(&format!("np_sealed_{}", i + 1))?
                        || matcher.has(&format!("np_chara_sealed_{}", i + 1))?;
                }
            }
        }
        if s.cards() && facts.contains(RequiredBattleFacts::CARDS) {
            s.card_groups = card_groups(frame);
            for i in 0..5 {
                s.support_cards[i] = matcher.has(&format!("support_card_{}", i + 1))?;
                s.sealed[i] = matcher.has(&format!("chara_sealed_{}", i + 1))?;
                for color in ["arts", "quick", "buster"] {
                    s.sealed[i] |= matcher.has(&format!("sealed_{color}_{}", i + 1))?;
                }
                let weak = matcher.find(&format!("weak_{}", i + 1))?;
                let resist = matcher.find(&format!("resist_{}", i + 1))?;
                s.resist[i] = match (weak, resist) {
                    (Some(a), Some(b)) => {
                        if a.score <= b.score {
                            1.7
                        } else {
                            0.6
                        }
                    }
                    (Some(_), None) => 1.7,
                    (None, Some(_)) => 0.6,
                    _ => 1.0,
                };
                let mut best = None;
                for n in 1..=10 {
                    if let Some(m) = matcher.find(&format!("critical_{n}_{}", i + 1))?
                        && best.is_none_or(|(score, _)| m.score < score)
                    {
                        best = Some((m.score, (n, m.threshold, m.region)));
                    }
                }
                s.critical[i] = best.map_or_else(
                    || Recognition::unknown(UnknownReason::NoCandidate, None),
                    |(score, (n, threshold, roi))| {
                        Recognition::known(
                            n as f64 / 10.0,
                            RecognitionEvidence {
                                detector: DetectorKind::CriticalRate,
                                roi,
                                method: RecognitionMethod::TemplateDifference,
                                score,
                                threshold,
                                direction: ScoreDirection::LowerIsBetter,
                                feature_floor: None,
                            },
                        )
                    },
                );
            }
            s.card_owners =
                self.card_owners(frame, &mut s.card_groups, &s.support_cards, control)?;
            s.support_identity = self.support_servant(frame, &s.support_cards, control)?;
            s.support_atlas_id = s.support_identity.value();
        }
        Ok(s)
    }

    /// Evaluate likely scene anchors first. Results stay in the frame-local
    /// matcher cache; the complete scene-safety pass below still runs so a hint
    /// cannot hide defeat, victory, dialogs or conflicting foreground scenes.
    fn prime_expected_battle_scenes(
        &self,
        matcher: &mut FrameMatcher<'_>,
        expected_scenes: &[Scene],
    ) -> Result<()> {
        for scene in expected_scenes {
            match scene {
                Scene::BattleTurn => {
                    matcher.has("turn_begin")?;
                }
                Scene::CommandCards => {
                    for index in 1..=5 {
                        for color in ["arts", "quick", "buster"] {
                            matcher.find(&format!("{color}_{index}"))?;
                        }
                    }
                }
                Scene::SkillConfirmation => {
                    matcher.has_optional("skill_confirm_header")?;
                    matcher.has_optional("skill_confirm_cancel")?;
                    matcher.has_optional("skill_confirm_button")?;
                }
                Scene::SkillTarget => {
                    matcher.has("target_cross")?;
                    matcher.has("target_header")?;
                }
                Scene::OrderChange => {
                    matcher.has("order_change")?;
                }
                Scene::Victory => {
                    matcher.has("victory")?;
                    matcher.has_optional("bond_result")?;
                }
                Scene::Defeated => {
                    matcher.has("defeated")?;
                }
                Scene::UnexpectedDialog => {
                    matcher.has("skill_error")?;
                    matcher.has("skill_none_1")?;
                    matcher.has("skill_none_2")?;
                }
                // BattleAnimation is established only by the bounded action
                // verification context. It has no visual detector yet.
                Scene::BattleAnimation | Scene::Unknown => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn recognize_skill(
        &self,
        matcher: &mut FrameMatcher<'_>,
        cooldown_template: &str,
        center: Point,
        available_threshold: f64,
        cooldown_detector: DetectorKind,
    ) -> Result<Recognition<SkillState>> {
        if let Some(found) = matcher.find(cooldown_template)? {
            return Ok(Recognition::known(
                SkillState::Cooldown,
                RecognitionEvidence {
                    detector: cooldown_detector,
                    roi: found.region,
                    method: RecognitionMethod::TemplateDifference,
                    score: found.score,
                    threshold: found.threshold,
                    direction: ScoreDirection::LowerIsBetter,
                    feature_floor: None,
                },
            ));
        }

        // Pale skill art can lack vivid pixels. A calibrated full icon (including
        // its cooldown overlay area) may prove availability after the cooldown
        // detector has already had priority; never lower the global color floor.
        let available_template = format!("{cooldown_template}_available");
        if self.has_template(&available_template)
            && let Some(found) = matcher.find(&available_template)?
        {
            return Ok(Recognition::known(
                SkillState::Available,
                RecognitionEvidence {
                    detector: cooldown_detector,
                    roi: found.region,
                    method: RecognitionMethod::TemplateDifference,
                    score: found.score,
                    threshold: found.threshold,
                    direction: ScoreDirection::LowerIsBetter,
                    feature_floor: None,
                },
            ));
        }

        // Some skills are visibly disabled without a numeric cooldown. A
        // slot-specific calibrated template is the only evidence strong
        // enough to classify that state; otherwise preserve Unknown so the
        // strategy can stop safely instead of guessing.
        let disabled_template = format!("{cooldown_template}_disabled");
        if self.has_template(&disabled_template)
            && let Some(found) = matcher.find(&disabled_template)?
        {
            return Ok(Recognition::known(
                SkillState::Disabled,
                RecognitionEvidence {
                    detector: cooldown_detector,
                    roi: found.region,
                    method: RecognitionMethod::TemplateDifference,
                    score: found.score,
                    threshold: found.threshold,
                    direction: ScoreDirection::LowerIsBetter,
                    feature_floor: None,
                },
            ));
        }

        let frame = matcher.frame();
        let chroma_floor = (available_threshold * 255.0).round() as u8;
        let mut vivid_pixels = 0_u32;
        for y in center.y.saturating_sub(20)..=center.y.saturating_add(20).min(frame.height() - 1) {
            for x in
                center.x.saturating_sub(20)..=center.x.saturating_add(20).min(frame.width() - 1)
            {
                let pixel = frame.get_pixel(x, y);
                let high = pixel[0].max(pixel[1]).max(pixel[2]);
                let low = pixel[0].min(pixel[1]).min(pixel[2]);
                vivid_pixels += u32::from(high - low >= chroma_floor);
            }
        }
        const MIN_VIVID_PIXELS: u32 = 3;
        let score = f64::from(vivid_pixels);
        let evidence = RecognitionEvidence {
            detector: DetectorKind::SkillIconChroma,
            roi: crate::vision::Rect {
                x: center.x.saturating_sub(20),
                y: center.y.saturating_sub(20),
                width: 41,
                height: 41,
            },
            method: RecognitionMethod::ColorChromaPixelCount,
            score,
            threshold: f64::from(MIN_VIVID_PIXELS),
            direction: ScoreDirection::HigherIsBetter,
            feature_floor: Some(available_threshold),
        };
        Ok(if vivid_pixels >= MIN_VIVID_PIXELS {
            Recognition::known(SkillState::Available, evidence)
        } else {
            Recognition::unknown(UnknownReason::NoCandidate, Some(evidence))
        })
    }

    fn support_servant(
        &self,
        frame: &RgbaImage,
        support_cards: &[bool; 5],
        control: &Control,
    ) -> Result<Recognition<u32>> {
        let mut results = Vec::new();
        for (index, support) in support_cards.iter().enumerate() {
            if *support {
                results.push(self.support_card_recognition(frame, index, control)?);
            }
        }
        Ok(resolve_support_identity(&results))
    }

    fn card_owners(
        &self,
        frame: &RgbaImage,
        groups: &mut [Option<CardGroup>; 5],
        support_cards: &[bool; 5],
        control: &Control,
    ) -> Result<[Option<PartySlot>; 5]> {
        let mut owners = [None; 5];
        for index in 0..5 {
            owners[index] = if support_cards[index] {
                self.support_party_slot()
            } else {
                self.card_identity(frame, index, control)?
            };
        }

        resolve_group_owners(groups, &mut owners);
        Ok(owners)
    }

    fn battle_wave(&self, matcher: &mut FrameMatcher<'_>) -> Result<Option<BattleWave>> {
        let mut digit = |position: &str| -> Result<Option<u8>> {
            let mut best = None;
            for value in 1..=3 {
                if let Some(found) = matcher.find(&format!("wave_{position}_{value}"))?
                    && best.is_none_or(|(score, _)| found.score < score)
                {
                    best = Some((found.score, value));
                }
            }
            Ok(best.map(|(_, value)| value))
        };
        let (Some(current), Some(total)) = (digit("current")?, digit("total")?) else {
            return Ok(None);
        };
        Ok(BattleWave::new(current, total).ok())
    }

    fn enemy_charge(
        &self,
        matcher: &mut FrameMatcher<'_>,
        region: crate::vision::Rect,
        templates: [&str; 3],
    ) -> Result<Option<EnemyCharge>> {
        let [empty_name, filled_name, full_name] = templates;
        let full = matcher.count_in(full_name, region)?;
        if full > 0 {
            return Ok(u8::try_from(full).ok().map(|value| EnemyCharge {
                current: value,
                total: value,
            }));
        }
        let current = matcher.count_in(filled_name, region)?;
        let empty = matcher.count_in(empty_name, region)?;
        Ok(
            match (u8::try_from(current), u8::try_from(current + empty)) {
                (Ok(current), Ok(total)) if total > 0 => Some(EnemyCharge { current, total }),
                _ => None,
            },
        )
    }

    fn enemy_hp(
        &self,
        matcher: &FrameMatcher<'_>,
        region: Rect,
        standard: bool,
    ) -> Result<Option<u32>> {
        #[cfg(feature = "maa")]
        if let Some(reading) = self.maa_enemy_hp(matcher.frame(), region, matcher.control())? {
            // A configured OCR miss must not silently switch recognizers.
            return Ok(reading.value());
        }
        Ok(if standard {
            read_large_white_digits(matcher.frame(), region, 5)
        } else {
            read_white_digits(matcher.frame(), region, 6)
        })
    }

    fn enemy_states_standard(&self, matcher: &mut FrameMatcher<'_>) -> Result<EnemyObservation> {
        let mut charges = [None; 3];
        for (index, charge) in charges.iter_mut().enumerate() {
            *charge = self.enemy_charge(
                matcher,
                UI_LAYOUT_V1.battle.enemy_charge_standard[index],
                [
                    "enemy_charge_empty",
                    "enemy_charge_filled",
                    "enemy_charge_full",
                ],
            )?;
        }

        let present = charges.map(|charge| charge.is_some());
        if !present.iter().any(|value| *value) {
            return Ok(EnemyObservation::default());
        }
        let mut hp = [None; 3];
        for (index, region) in UI_LAYOUT_V1
            .battle
            .enemy_hp_standard
            .into_iter()
            .enumerate()
        {
            hp[index] = self.enemy_hp(matcher, region, true)?;
        }
        if (0..3).any(|index| present[index] != hp[index].is_some()) {
            return Ok(EnemyObservation::default());
        }

        let mut enemies = [None; 6];
        for index in 0..3 {
            if present[index] {
                enemies[index] = Some(EnemyState {
                    slot: EnemySlot::try_from(index + 1).map_err(anyhow::Error::msg)?,
                    hp: hp[index].expect("presence requires an HP value"),
                    charge: charges[index],
                });
            }
        }
        Ok(EnemyObservation {
            layout: Some(EnemyLayout::Standard),
            enemies,
            selected: selected_enemy(
                matcher.frame(),
                &UI_LAYOUT_V1.battle.enemy_indicators_standard,
                &present,
            ),
        })
    }

    fn enemy_states_six(&self, matcher: &mut FrameMatcher<'_>) -> Result<EnemyObservation> {
        let mut charges = [None; 6];
        for (index, charge) in charges.iter_mut().enumerate() {
            *charge = self.enemy_charge(
                matcher,
                UI_LAYOUT_V1.battle.enemy_charge_six[index],
                [
                    "enemy_charge_empty_small",
                    "enemy_charge_filled_small",
                    "enemy_charge_full_small",
                ],
            )?;
        }

        let present = charges.map(|charge| charge.is_some());
        if !present.iter().any(|value| *value) {
            return Ok(EnemyObservation::default());
        }
        let mut hp = [None; 6];
        for (index, region) in UI_LAYOUT_V1.battle.enemy_hp_six.into_iter().enumerate() {
            hp[index] = self.enemy_hp(matcher, region, false)?;
        }
        if (0..6).any(|index| present[index] != hp[index].is_some()) {
            return Ok(EnemyObservation::default());
        }

        let mut enemies = [None; 6];
        for index in 0..6 {
            if present[index] {
                enemies[index] = Some(EnemyState {
                    slot: EnemySlot::try_from(index + 1).map_err(anyhow::Error::msg)?,
                    hp: hp[index].expect("presence requires an HP value"),
                    charge: charges[index],
                });
            }
        }

        Ok(EnemyObservation {
            layout: Some(EnemyLayout::Six),
            enemies,
            selected: selected_enemy(
                matcher.frame(),
                &UI_LAYOUT_V1.battle.enemy_indicators_six,
                &present,
            ),
        })
    }
}

// Catmull-Rom normalization makes some unrelated light-haired portraits score
// near FGO-py's one-way 0.025 OpenCV threshold. Requiring both directions keeps
// the two real same-servant samples (worst 0.0143) apart from the closest
// observed mismatch (best mutual maximum 0.0194).
const CARD_GROUP_MATCH_THRESHOLD: f64 = 0.016;

fn resolve_support_identity(results: &[Recognition<u32>]) -> Recognition<u32> {
    if results.is_empty() {
        return Recognition::unknown(UnknownReason::NoSupportMarker, None);
    }
    let mut identity = None;
    let mut ambiguous = false;
    for result in results {
        match *result {
            Recognition::Known { value, .. } => {
                if identity.is_some_and(|previous| previous != value) {
                    return Recognition::unknown(UnknownReason::ConflictingCandidates, None);
                }
                identity = Some(value);
            }
            Recognition::Unknown {
                reason: UnknownReason::Ambiguous,
                ..
            } => ambiguous = true,
            _ => {}
        }
    }
    if ambiguous {
        return Recognition::unknown(UnknownReason::Ambiguous, None);
    }
    identity.map_or(
        Recognition::unknown(UnknownReason::NoCandidate, None),
        Recognition::from,
    )
}

// Conflicting identities invalidate the relationship itself: keeping its group
// ID would let the face-only Brave Chain fallback reuse rejected evidence.
fn resolve_group_owners(groups: &mut [Option<CardGroup>; 5], owners: &mut [Option<PartySlot>; 5]) {
    for anchor in 0..5 {
        let Some(group) = groups[anchor] else {
            continue;
        };
        let mut group_owner = None;
        let mut conflict = false;
        for index in 0..5 {
            if groups[index] != Some(group) {
                continue;
            }
            if let Some(owner) = owners[index] {
                if group_owner.is_some_and(|current| current != owner) {
                    conflict = true;
                    break;
                }
                group_owner = Some(owner);
            }
        }
        let resolved = (!conflict).then_some(group_owner).flatten();
        for index in 0..5 {
            if groups[index] == Some(group) {
                owners[index] = resolved;
                if conflict {
                    groups[index] = None;
                }
            }
        }
    }
}

fn card_groups(frame: &RgbaImage) -> [Option<CardGroup>; 5] {
    let mut result = [None; 5];
    let mut next_group = 0;
    for anchor in 0..5 {
        if result[anchor].is_some() {
            continue;
        }
        let group = CardGroup::from_index(next_group);
        result[anchor] = Some(group);
        for (candidate, candidate_group) in result.iter_mut().enumerate().skip(anchor + 1) {
            if candidate_group.is_none() {
                let forward = patch_match_score(
                    frame,
                    UI_LAYOUT_V1.battle.card_group_patches[candidate],
                    UI_LAYOUT_V1.battle.card_group_searches[anchor],
                );
                let reverse = patch_match_score(
                    frame,
                    UI_LAYOUT_V1.battle.card_group_patches[anchor],
                    UI_LAYOUT_V1.battle.card_group_searches[candidate],
                );
                if forward.max(reverse) < CARD_GROUP_MATCH_THRESHOLD {
                    *candidate_group = Some(group);
                }
            }
        }
        next_group += 1;
    }
    result
}

fn patch_match_score(
    frame: &RgbaImage,
    patch: crate::vision::Rect,
    search: crate::vision::Rect,
) -> f64 {
    debug_assert!(patch.width <= search.width && patch.height <= search.height);
    let mut patch_energy = 0.0;
    for y in 0..patch.height {
        for x in 0..patch.width {
            for channel in 0..3 {
                let value = frame.get_pixel(patch.x + x, patch.y + y)[channel] as f64;
                patch_energy += value * value;
            }
        }
    }

    let mut best = f64::INFINITY;
    for offset_y in 0..=search.height - patch.height {
        for offset_x in 0..=search.width - patch.width {
            let mut difference = 0.0;
            let mut target_energy = 0.0;
            for y in 0..patch.height {
                for x in 0..patch.width {
                    let source = frame.get_pixel(patch.x + x, patch.y + y);
                    let target = frame.get_pixel(search.x + offset_x + x, search.y + offset_y + y);
                    for channel in 0..3 {
                        let left = source[channel] as f64;
                        let right = target[channel] as f64;
                        difference += (left - right) * (left - right);
                        target_energy += right * right;
                    }
                }
            }
            let denominator = (patch_energy * target_energy).sqrt();
            if denominator > 0.0 {
                best = best.min(difference / denominator);
            }
        }
    }
    best
}

fn selected_enemy(
    frame: &RgbaImage,
    regions: &[crate::vision::Rect],
    present: &[bool],
) -> Option<EnemySlot> {
    let mut ranked: Vec<_> = regions
        .iter()
        .zip(present)
        .enumerate()
        .filter(|(_, (_, present))| **present)
        .map(|(index, (region, _))| {
            let mut blue = 0;
            for y in region.y..region.y + region.height {
                for x in region.x..region.x + region.width {
                    let pixel = frame.get_pixel(x, y);
                    blue += usize::from(pixel[2] > 120 && pixel[2] as i16 > pixel[0] as i16 + 35);
                }
            }
            (index, blue)
        })
        .collect();
    ranked.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
    let (index, score) = *ranked.first()?;
    let confirmed = ranked.len() == 1 || (score >= 100 && score >= ranked[1].1.saturating_mul(2));
    confirmed.then(|| EnemySlot::try_from(index + 1).expect("fixed enemy slots"))
}

fn portrait_fingerprint(frame: &RgbaImage, region: crate::vision::Rect) -> PortraitFingerprint {
    let mut samples = [[0; 3]; 64];
    for sample_y in 0..8 {
        let y0 = region.y + sample_y * region.height / 8;
        let y1 = region.y + (sample_y + 1) * region.height / 8;
        for sample_x in 0..8 {
            let x0 = region.x + sample_x * region.width / 8;
            let x1 = region.x + (sample_x + 1) * region.width / 8;
            let mut sum = [0u32; 3];
            let count = (x1 - x0) * (y1 - y0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let pixel = frame.get_pixel(x, y);
                    for channel in 0..3 {
                        sum[channel] += pixel[channel] as u32;
                    }
                }
            }
            samples[(sample_y * 8 + sample_x) as usize] = sum.map(|value| (value / count) as u8);
        }
    }
    PortraitFingerprint { samples }
}

/// Port of the original CN horizontal projection and 66-pixel dilation.
/// Returns 255 for a visible dialog with no recognized target layout.
fn target_regions(frame: &RgbaImage) -> u8 {
    let raw: Vec<_> = (306..973)
        .map(|x| {
            (320..547).any(|y| {
                let p = frame.get_pixel(x, y);
                p[0].max(p[1]).max(p[2]) > 67
            })
        })
        .collect();
    let mut count = 0;
    let mut previous = false;
    for x in 0..raw.len() {
        let on = raw[x.saturating_sub(33)..(x + 33).min(raw.len())]
            .iter()
            .any(|v| *v);
        if on && !previous {
            count += 1;
        }
        previous = on;
    }
    if count == 0 { 255 } else { count }
}

#[cfg(test)]
mod reflow_tests {
    use super::*;
    use crate::vision::normalize;
    use image::GenericImage;
    use std::{path::PathBuf, time::Duration};

    #[test]
    fn six_enemy_layout_remains_complete_after_five_enemies_leave() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let raw = image::open(root.join("tests/fixtures/live-enemy-reflow-one.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        let control = Control::new(Duration::from_secs(10));
        let mut matcher = vision.frame_matcher(&frame, &control);
        let observation = vision.enemy_states_six(&mut matcher).unwrap();

        assert_eq!(observation.layout, Some(EnemyLayout::Six));
        assert_eq!(observation.selected, Some(EnemySlot::One));
        assert_eq!(observation.enemies.iter().flatten().count(), 1);
        assert_eq!(observation.enemies[0].unwrap().hp, 145_196);
        assert_eq!(
            observation.enemies[0].unwrap().charge,
            Some(EnemyCharge {
                current: 0,
                total: 5
            })
        );
    }

    #[cfg(feature = "maa")]
    #[test]
    #[ignore = "requires FGO_MAA_LIBRARY and FGO_MAA_OCR_MODEL; verifies sparse native enemy OCR"]
    fn six_enemy_reflow_with_native_numeric_ocr() {
        use crate::maa::MaaRecognizer;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let library =
            PathBuf::from(std::env::var_os("FGO_MAA_LIBRARY").expect("set FGO_MAA_LIBRARY"));
        let model =
            PathBuf::from(std::env::var_os("FGO_MAA_OCR_MODEL").expect("set FGO_MAA_OCR_MODEL"));
        let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        vision
            .set_maa_backend(
                MaaRecognizer::new(&library, &[root.join("assets/maa")], Some(&model)).unwrap(),
            )
            .unwrap();
        let raw = image::open(root.join("tests/fixtures/live-enemy-reflow-one.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        let control = Control::new(Duration::from_secs(30));
        for (index, roi) in UI_LAYOUT_V1.battle.enemy_hp_six.into_iter().enumerate() {
            let reading = vision.maa_enemy_hp(&frame, roi, &control).unwrap().unwrap();
            assert_eq!(
                reading.value(),
                if index == 0 { Some(145_196) } else { None }
            );
            assert_eq!(
                reading.evidence().unwrap().method,
                RecognitionMethod::MaaOcr
            );
        }
        let observation = vision
            .enemy_states_six(&mut vision.frame_matcher(&frame, &control))
            .unwrap();
        assert_eq!(observation.layout, Some(EnemyLayout::Six));
        assert_eq!(observation.selected, Some(EnemySlot::One));
        assert_eq!(
            observation.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [Some(145_196), None, None, None, None, None]
        );
    }

    #[test]
    fn standard_layout_reads_the_single_middle_enemy() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let raw = image::open(root.join("tests/fixtures/live-enemy-standard-one.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        let state = vision
            .battle_state(&frame, &Control::new(Duration::from_secs(10)))
            .unwrap();

        assert!(state.idle());
        assert_eq!(state.enemy_layout, Some(EnemyLayout::Standard));
        assert_eq!(state.selected_enemy, Some(EnemySlot::Two));
        assert_eq!(state.enemies.iter().flatten().count(), 1);
        assert_eq!(state.enemies[1].unwrap().hp, 252_064);
        assert_eq!(
            state.enemies[1].unwrap().charge,
            Some(EnemyCharge {
                current: 0,
                total: 5
            })
        );
    }

    #[test]
    fn standard_layout_reads_real_two_and_three_enemy_frames() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let inspect = |name: &str| {
            let raw = image::open(root.join("tests/fixtures").join(name))
                .unwrap()
                .to_rgba8();
            let frame = normalize(&raw, None).unwrap().0;
            vision
                .battle_state(&frame, &Control::new(Duration::from_secs(10)))
                .unwrap()
        };

        let two = inspect("live-enemy-standard-two.png");
        assert!(two.idle());
        assert_eq!(two.wave, BattleWave::new(3, 3).ok());
        assert_eq!(two.enemy_layout, Some(EnemyLayout::Standard));
        assert_eq!(two.selected_enemy, Some(EnemySlot::Three));
        assert_eq!(
            two.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [None, Some(1_414), Some(2_340), None, None, None]
        );
        assert_eq!(
            two.enemies[1].unwrap().charge,
            Some(EnemyCharge {
                current: 1,
                total: 4
            })
        );
        assert_eq!(
            two.enemies[2].unwrap().charge,
            Some(EnemyCharge {
                current: 1,
                total: 5
            })
        );

        let three = inspect("live-enemy-standard-three.png");
        assert!(three.idle());
        assert_eq!(three.wave, BattleWave::new(1, 3).ok());
        assert_eq!(three.enemy_layout, Some(EnemyLayout::Standard));
        assert_eq!(three.selected_enemy, Some(EnemySlot::Three));
        assert_eq!(
            three.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [Some(1_414), Some(1_344), Some(1_414), None, None, None]
        );
        assert_eq!(
            three
                .enemies
                .iter()
                .take(3)
                .map(|enemy| enemy.unwrap().charge.unwrap())
                .collect::<Vec<_>>(),
            vec![
                EnemyCharge {
                    current: 0,
                    total: 4
                },
                EnemyCharge {
                    current: 0,
                    total: 5
                },
                EnemyCharge {
                    current: 0,
                    total: 4
                }
            ]
        );
    }

    #[test]
    fn standard_layout_geometry_supports_all_three_positions() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let raw = image::open(root.join("tests/fixtures/live-enemy-standard-one.png"))
            .unwrap()
            .to_rgba8();
        let mut frame = normalize(&raw, None).unwrap().0;
        let hp = image::imageops::crop_imm(&frame, 350, 40, 122, 25).to_image();
        let charge = image::imageops::crop_imm(&frame, 410, 67, 90, 21).to_image();
        for index in [0, 2] {
            let hp_region = UI_LAYOUT_V1.battle.enemy_hp_standard[index];
            let charge_region = UI_LAYOUT_V1.battle.enemy_charge_standard[index];
            frame.copy_from(&hp, hp_region.x, hp_region.y).unwrap();
            frame
                .copy_from(&charge, charge_region.x, charge_region.y)
                .unwrap();
        }

        let state = vision
            .battle_state(&frame, &Control::new(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(state.enemy_layout, Some(EnemyLayout::Standard));
        assert_eq!(
            state.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [
                Some(252_064),
                Some(252_064),
                Some(252_064),
                None,
                None,
                None,
            ]
        );
        assert_eq!(state.selected_enemy, Some(EnemySlot::Two));
    }

    #[test]
    fn resumed_default_battle_reads_enemy_hp_target_and_np_together() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let raw = image::open(root.join("tests/fixtures/live-default-highest-hp.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        let state = vision
            .battle_state_with_request(
                &frame,
                &Control::new(Duration::from_secs(10)),
                &[],
                BattleObservationRequest::new(
                    RequiredBattleFacts::NP.union(RequiredBattleFacts::ENEMIES),
                    &[Scene::BattleTurn],
                ),
            )
            .unwrap();

        assert!(state.idle());
        assert!(state.markers.is_empty());
        assert_eq!(state.enemy_layout, Some(EnemyLayout::Standard));
        assert_eq!(state.selected_enemy, Some(EnemySlot::Three));
        assert_eq!(
            state.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [Some(56_453), Some(383_032), Some(45_484), None, None, None]
        );
        assert_eq!(
            state.np_percent.map(|np| np.value()),
            [Some(39), Some(74), Some(137)]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_identity_distinguishes_missing_ambiguous_and_conflicting_evidence() {
        let missing = Recognition::unknown(UnknownReason::NoCandidate, None);
        let ambiguous = Recognition::unknown(UnknownReason::Ambiguous, None);
        assert_eq!(
            resolve_support_identity(&[]),
            Recognition::unknown(UnknownReason::NoSupportMarker, None)
        );
        assert_eq!(resolve_support_identity(&[missing]), missing);
        assert_eq!(
            resolve_support_identity(&[704000.into(), missing]),
            704000.into()
        );
        assert_eq!(
            resolve_support_identity(&[704000.into(), ambiguous]),
            ambiguous
        );
        assert_eq!(
            resolve_support_identity(&[ambiguous, 704000.into()]),
            ambiguous
        );
        assert_eq!(
            resolve_support_identity(&[704000.into(), 704100.into()]),
            Recognition::unknown(UnknownReason::ConflictingCandidates, None)
        );
    }

    #[test]
    fn real_support_recognition_exposes_ambiguity_and_unrequested_state() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let frame = crate::vision::normalize(
            &image::open(root.join("tests/fixtures/step12.png"))
                .unwrap()
                .to_rgba8(),
            None,
        )
        .unwrap()
        .0;
        let template = image::imageops::crop_imm(&frame, 76, 431, 98, 47).to_image();
        let mut vision = vision();
        vision
            .configure_support_identities(vec![
                (704000, vec![template.clone()]),
                (704100, vec![template]),
            ])
            .unwrap();
        let control = Control::new(Duration::from_secs(10));
        let state = vision.battle_state(&frame, &control).unwrap();
        assert_eq!(state.support_atlas_id, None);
        assert_eq!(
            state.support_identity,
            Recognition::unknown(UnknownReason::Ambiguous, None)
        );
        let turn = crate::vision::normalize(
            &image::open(root.join("tests/fixtures/step11.png"))
                .unwrap()
                .to_rgba8(),
            None,
        )
        .unwrap()
        .0;
        assert_eq!(
            vision
                .battle_state(&turn, &control)
                .unwrap()
                .support_identity,
            Recognition::NotRequested
        );
    }

    #[test]
    fn conflicting_group_cannot_reach_brave_chain_fallback() {
        let mut groups = [0, 0, 0, 1, 1].map(CardGroup::from_id);
        let mut owners = [
            Some(PartySlot::One),
            None,
            Some(PartySlot::Two),
            Some(PartySlot::Three),
            None,
        ];
        resolve_group_owners(&mut groups, &mut owners);
        assert_eq!(&groups[..3], &[None; 3]);
        assert_eq!(&owners[..3], &[None; 3]);
        assert_eq!(groups[3], CardGroup::from_id(1));
        assert_eq!(groups[4], CardGroup::from_id(1));
        assert_eq!(&owners[3..], &[Some(PartySlot::Three); 2]);

        let state = BattleState {
            colors: [Some(Color::Arts); 5],
            resist: [0.1, 0.1, 0.1, 1.0, 1.0],
            card_groups: groups,
            card_owners: owners,
            ..BattleState::default()
        };
        let strategy = crate::profile::CardStrategyProfile::default();
        let selected = crate::battle::rank_cards(&state, [false; 3]).unwrap();
        assert!(selected.contains(&4) && selected.contains(&5));
        // Reproduce the old state: clearing owners alone retained a false bonus.
        let mut old = state.clone();
        old.card_groups = [0, 0, 0, 1, 1].map(CardGroup::from_id);
        let selected = crate::battle::rank_cards_for_party(
            &old,
            [false; 3],
            [
                Some(PartySlot::One),
                Some(PartySlot::Two),
                Some(PartySlot::Three),
            ],
            &strategy,
        )
        .unwrap();
        assert!(selected.iter().all(|card| *card <= 3));
        let mut repeated_groups = state.card_groups;
        let mut repeated_owners = state.card_owners;
        resolve_group_owners(&mut repeated_groups, &mut repeated_owners);
        assert_eq!(repeated_groups, groups);
        assert_eq!(repeated_owners, owners);
    }

    #[test]
    fn consistent_and_unidentified_groups_keep_their_relationships() {
        let original = [0, 0, 0, 1, 1].map(CardGroup::from_id);
        for known in [None, Some(PartySlot::One)] {
            let mut groups = original;
            let mut owners = [None, known, None, None, None];
            resolve_group_owners(&mut groups, &mut owners);
            assert_eq!(groups, original);
            assert_eq!(&owners[..3], &[known; 3]);
            assert_eq!(&owners[3..], &[None; 2]);
        }
    }
    use image::GenericImage;
    use std::{path::PathBuf, time::Duration};

    fn vision() -> Vision {
        Vision::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/templates.json"))
            .unwrap()
    }

    fn stage_asset(value: u8) -> RgbaImage {
        image::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("assets/common/stage{value}.png")),
        )
        .unwrap()
        .to_rgba8()
    }

    #[test]
    fn target_projection_preserves_separate_regions_and_rejects_empty_dialog() {
        let mut frame = RgbaImage::new(1280, 720);
        assert_eq!(target_regions(&frame), 255);
        frame.put_pixel(400, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 1);
        frame.put_pixel(650, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 2);
        frame.put_pixel(900, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 3);
    }

    #[test]
    fn wave_templates_recognize_supported_pairs_and_reject_invalid_order() {
        let vision = vision();
        let control = Control::new(Duration::from_secs(2));
        for (current, total) in [(1, 1), (1, 3), (2, 3), (3, 3)] {
            let mut frame = RgbaImage::new(1280, 720);
            frame.copy_from(&stage_asset(current), 884, 14).unwrap();
            frame.copy_from(&stage_asset(total), 912, 13).unwrap();
            let mut matcher = vision.frame_matcher(&frame, &control);
            assert_eq!(
                vision.battle_wave(&mut matcher).unwrap(),
                Some(BattleWave::new(current, total).unwrap())
            );
        }

        let mut invalid = RgbaImage::new(1280, 720);
        invalid.copy_from(&stage_asset(3), 884, 14).unwrap();
        invalid.copy_from(&stage_asset(2), 912, 13).unwrap();
        let mut matcher = vision.frame_matcher(&invalid, &control);
        assert_eq!(vision.battle_wave(&mut matcher).unwrap(), None);
    }

    #[test]
    fn portrait_fingerprint_separates_stable_and_changed_slots() {
        let vision = vision();
        let control = Control::new(Duration::from_secs(2));
        let state = |step| {
            let frame = image::open(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/step{step}.png")),
            )
            .unwrap()
            .to_rgba8();
            let frame = crate::vision::normalize(&frame, None).unwrap().0;
            vision.battle_state(&frame, &control).unwrap()
        };
        let before = state(4).roster.unwrap();
        let skill = state(5).roster.unwrap();
        let swapped = state(9).roster.unwrap();
        let stable = std::array::from_fn::<_, 3, _>(|index| {
            before.portraits[index].difference(&skill.portraits[index])
        });
        let changed = std::array::from_fn::<_, 3, _>(|index| {
            before.portraits[index].difference(&swapped.portraits[index])
        });
        assert_eq!(stable, [0.0; 3]);
        assert!(changed[0] > 0.19, "changed portrait distance: {changed:?}");
        assert_eq!(changed[1..], [0.0, 0.0]);
        assert_eq!(before.support, [false, false, true]);
        assert_eq!(swapped.support, [false, false, true]);
    }

    #[test]
    fn six_enemy_observation_reads_hp_charge_and_selected_target() {
        let vision = vision();
        let control = Control::new(Duration::from_secs(2));
        let frame =
            image::open(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/step4.png"))
                .unwrap()
                .to_rgba8();
        let frame = crate::vision::normalize(&frame, None).unwrap().0;
        let state = vision.battle_state(&frame, &control).unwrap();
        assert_eq!(
            state.enemies.map(|enemy| enemy.map(|enemy| enemy.hp)),
            [
                Some(704_487),
                Some(735_728),
                Some(351_884),
                Some(353_590),
                Some(355_298),
                Some(3_152_503),
            ]
        );
        assert_eq!(
            state.enemies.map(|enemy| enemy.unwrap().charge),
            [
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
                Some(EnemyCharge {
                    current: 0,
                    total: 5
                }),
            ]
        );
        assert_eq!(state.selected_enemy, Some(EnemySlot::Six));
        assert!(
            state
                .enemies
                .iter()
                .all(|enemy| enemy.unwrap().charge.is_some())
        );
    }
}
