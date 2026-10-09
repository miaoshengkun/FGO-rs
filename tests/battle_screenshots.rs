use fgo_rs::{
    battle::{
        BattleObservationRequest, BattleTracker, Color, EnemyLayout, RequiredBattleFacts,
        SkillState,
    },
    control::Control,
    domain::{PartySlot, Scene},
    vision::{Vision, normalize},
};
use std::{path::PathBuf, time::Duration};

#[derive(serde::Deserialize)]
struct VisionCases {
    schema: u32,
    cases: Vec<VisionCase>,
}

#[derive(serde::Deserialize)]
struct VisionCase {
    file: String,
    scene: fgo_rs::domain::Scene,
    cards_complete: bool,
}

fn state(step: u32) -> fgo_rs::battle::BattleState {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let v = Vision::load(&root.join("assets/templates.json")).unwrap();
    let raw = image::open(root.join(format!("tests/fixtures/step{step}.png")))
        .unwrap()
        .to_rgba8();
    v.battle_state(
        &normalize(&raw, None).unwrap().0,
        &Control::new(Duration::from_secs(10)),
    )
    .unwrap()
}

#[test]
fn two_surviving_servants_keep_the_empty_frontline_np_unknown() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let frame = image::open(root.join("tests/fixtures/live-default-two-servants.png"))
        .unwrap()
        .to_rgba8();
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    let state = vision
        .battle_state_with_request(
            &frame,
            &Control::new(Duration::from_secs(10)),
            &[],
            BattleObservationRequest::new(RequiredBattleFacts::NP, &[Scene::BattleTurn]),
        )
        .unwrap();
    assert!(state.idle());
    assert_eq!(state.np_percent[0].value(), Some(85));
    assert_eq!(state.np_percent[1].value(), Some(19));
    assert!(state.np_percent[2].is_unknown());
}

#[test]
fn live_buff_cards_keep_groups_and_report_missing_identity_candidates() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let raw = image::open(root.join("tests/fixtures/live-owner-buffs.png"))
        .unwrap()
        .to_rgba8();
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    let state = vision
        .battle_state(
            &normalize(&raw, None).unwrap().0,
            &Control::new(Duration::from_secs(10)),
        )
        .unwrap();
    assert_eq!(state.scene(), Scene::CommandCards);
    assert_eq!(state.support_cards, [false, true, true, false, false]);
    assert_eq!(
        state.card_groups.map(|group| group.map(|group| group.id())),
        [Some(0), Some(1), Some(1), Some(2), Some(0)]
    );
    // Markers are visible, but this offline Vision has no Atlas candidates.
    assert_eq!(
        state.support_identity,
        fgo_rs::observation::Recognition::unknown(
            fgo_rs::observation::UnknownReason::NoCandidate,
            None
        )
    );
}

#[test]
fn versioned_vision_case_manifest_matches_real_fixtures() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest: VisionCases = serde_json::from_slice(
        &std::fs::read(root.join("tests/fixtures/vision-cases.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.schema, 1);
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    for case in manifest.cases {
        let raw = image::open(root.join("tests/fixtures").join(&case.file))
            .unwrap()
            .to_rgba8();
        let state = vision
            .battle_state(
                &normalize(&raw, None).unwrap().0,
                &Control::new(Duration::from_secs(10)),
            )
            .unwrap();
        assert_eq!(state.scene(), case.scene, "{}", case.file);
        assert_eq!(state.cards_complete(), case.cards_complete, "{}", case.file);
    }
}

#[test]
fn original_screenshots_distinguish_unused_and_used_skills() {
    let s = state(4);
    assert!(s.idle() && s.first_turn && s.iskandar && s.taigong && s.support);
    assert_eq!(s.wave.unwrap().current, 1);
    assert_eq!(s.wave.unwrap().total, 1);
    assert_eq!(
        s.skills.map(|row| row.map(|state| state.value())),
        [[Some(SkillState::Available); 3]; 3]
    );
    assert_eq!(
        s.np_glow, [false; 3],
        "skill-screen brightness is not a readiness check"
    );
    let s = state(5);
    assert_eq!(
        s.skills.map(|row| row.map(|state| state.value())),
        [
            [
                Some(SkillState::Cooldown),
                Some(SkillState::Cooldown),
                Some(SkillState::Available),
            ],
            [Some(SkillState::Available); 3],
            [Some(SkillState::Available); 3],
        ]
    );
    let s = state(10);
    assert!(s.reines && !s.iskandar && s.support);
    assert_eq!(
        s.skills.map(|row| row.map(|state| state.value())),
        [[Some(SkillState::Cooldown); 3]; 3]
    );
}

#[test]
fn missing_cooldown_and_color_evidence_keeps_skill_unknown() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let raw = image::open(root.join("tests/fixtures/step4.png"))
        .unwrap()
        .to_rgba8();
    let mut frame = normalize(&raw, None).unwrap().0;
    for y in 553..594 {
        for x in 53..94 {
            frame.put_pixel(x, y, image::Rgba([64, 64, 64, 255]));
        }
    }
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();

    let state = vision
        .battle_state(&frame, &Control::new(Duration::from_secs(10)))
        .unwrap();

    assert!(state.idle());
    assert!(state.skills[0][0].is_unknown());
    assert_eq!(state.skills[0][1].value(), Some(SkillState::Available));
}

#[test]
fn scene_only_request_skips_expensive_idle_and_card_facts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    let control = Control::new(Duration::from_secs(10));
    let markers = vec!["iskandar".to_owned(), "support_slot_3".to_owned()];

    let raw = image::open(root.join("tests/fixtures/step4.png"))
        .unwrap()
        .to_rgba8();
    let state = vision
        .battle_state_with_markers_and_facts(
            &normalize(&raw, None).unwrap().0,
            &control,
            &markers,
            RequiredBattleFacts::default(),
        )
        .unwrap();
    assert_eq!(state.scene(), fgo_rs::domain::Scene::BattleTurn);
    assert!(state.wave.is_none());
    assert!(state.markers.is_empty());
    assert!(state.enemy_layout.is_none());
    assert!(state.enemies.iter().all(Option::is_none));
    assert!(state.roster.is_none());
    assert!(
        state
            .skills
            .iter()
            .flatten()
            .all(|skill| matches!(skill, fgo_rs::observation::Recognition::NotRequested))
    );

    let raw = image::open(root.join("tests/fixtures/step12.png"))
        .unwrap()
        .to_rgba8();
    let state = vision
        .battle_state_with_markers_and_facts(
            &normalize(&raw, None).unwrap().0,
            &control,
            &markers,
            RequiredBattleFacts::default(),
        )
        .unwrap();
    assert_eq!(state.scene(), fgo_rs::domain::Scene::CommandCards);
    assert!(!state.cards_complete());
    assert!(state.card_groups.iter().all(Option::is_none));
}

#[test]
fn expected_scene_scheduling_does_not_override_visual_evidence() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    let control = Control::new(Duration::from_secs(10));
    let markers = vec!["iskandar".to_owned(), "support_slot_3".to_owned()];

    for (step, expected, actual) in [
        (4, Scene::CommandCards, Scene::BattleTurn),
        (12, Scene::BattleTurn, Scene::CommandCards),
    ] {
        let raw = image::open(root.join(format!("tests/fixtures/step{step}.png")))
            .unwrap()
            .to_rgba8();
        let state = vision
            .battle_state_with_request(
                &normalize(&raw, None).unwrap().0,
                &control,
                &markers,
                BattleObservationRequest::new(RequiredBattleFacts::default(), &[expected]),
            )
            .unwrap();
        assert_eq!(state.scene(), actual, "step{step}");
    }
}

#[test]
fn original_screenshots_verify_order_change_and_master_cooldowns() {
    let s = state(6);
    assert!(s.master_open);
    assert_eq!(
        s.master_skills.map(|state| state.value()),
        [Some(SkillState::Available); 3]
    );
    let s = state(7);
    assert!(s.order_change);
    assert_eq!(s.selected, [false; 6]);
    let s = state(8);
    assert!(s.order_change);
    assert_eq!(s.selected, [true, false, false, false, true, false]);
    let s = state(9);
    assert!(s.idle() && s.reines && s.support);
    assert_eq!(
        s.skills.map(|row| row.map(|state| state.value())),
        [[Some(SkillState::Available); 3]; 3]
    );
    let s = state(11);
    assert!(s.master_open);
    assert_eq!(
        s.master_skills.map(|state| state.value()),
        [Some(SkillState::Cooldown); 3]
    );
}

#[test]
fn idle_screenshots_keep_the_same_complete_enemy_observation() {
    let expected = [704_487, 735_728, 351_884, 353_590, 355_298, 3_152_503];
    for step in [4, 5, 6, 9, 10, 11] {
        let state = state(step);
        assert_eq!(
            state.enemies.map(|enemy| enemy.unwrap().hp),
            expected,
            "step {step}"
        );
        assert_eq!(state.enemy_layout, Some(EnemyLayout::Six));
        assert_eq!(state.selected_enemy, Some(fgo_rs::domain::EnemySlot::Six));
    }
    assert!(state(7).enemies.iter().all(Option::is_none));
    assert!(state(12).enemies.iter().all(Option::is_none));
}

#[test]
fn real_portrait_change_reaches_tracker_without_misclassifying_order_change() {
    let before = state(4);
    let changed = state(9);

    let mut replacement = BattleTracker::default();
    assert!(replacement.observe(&before).unwrap());
    assert!(replacement.attack_committed());
    assert!(!replacement.observe(&changed).unwrap());
    assert!(!replacement.roster_ready());
    assert!(replacement.observe(&changed).unwrap());
    assert_eq!(
        replacement.progress().frontline(),
        [
            Some(PartySlot::Four),
            Some(PartySlot::Two),
            Some(PartySlot::Three)
        ]
    );

    let mut deliberate_swap = BattleTracker::default();
    assert!(deliberate_swap.observe(&before).unwrap());
    assert!(
        deliberate_swap
            .order_change_committed(PartySlot::One, PartySlot::Five)
            .unwrap()
    );
    assert!(!deliberate_swap.observe(&changed).unwrap());
    assert_eq!(
        deliberate_swap.progress().frontline(),
        [
            Some(PartySlot::Five),
            Some(PartySlot::Two),
            Some(PartySlot::Three)
        ]
    );
}

#[test]
fn card_screenshot_recognizes_colors_support_and_three_ready_nps() {
    let s = state(12);
    assert!(s.cards());
    assert!(!s.idle());
    assert_eq!(
        s.colors,
        [
            Some(Color::Arts),
            Some(Color::Buster),
            Some(Color::Quick),
            Some(Color::Quick),
            Some(Color::Buster)
        ]
    );
    assert_eq!(s.support_cards, [true, false, true, true, false]);
    assert_eq!(
        s.card_groups.map(|group| group.map(|group| group.id())),
        [Some(0), Some(1), Some(0), Some(0), Some(2)]
    );
    assert_eq!(s.np_glow, [true; 3]);
    assert_eq!(s.np_blocked, [false; 3]);
    assert_eq!(s.sealed, [false; 5]);
    assert!(s.critical.iter().all(|value| value.is_unknown()));
}

#[test]
fn four_card_anchors_confirm_the_scene_without_claiming_complete_details() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let raw = image::open(root.join("tests/fixtures/step12.png"))
        .unwrap()
        .to_rgba8();
    let mut frame = normalize(&raw, None).unwrap().0;
    for y in 537..581 {
        for x in 80..131 {
            frame.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();

    let state = vision
        .battle_state(&frame, &Control::new(Duration::from_secs(10)))
        .unwrap();

    assert!(state.cards());
    assert!(!state.cards_complete());
    assert_eq!(state.scene(), fgo_rs::domain::Scene::CommandCards);
}

#[test]
fn card_identity_match_and_group_propagation_reach_battle_state() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let raw = image::open(root.join("tests/fixtures/step12.png"))
        .unwrap()
        .to_rgba8();
    let frame = normalize(&raw, None).unwrap().0;
    let support = image::imageops::crop_imm(&frame, 76, 431, 98, 47).to_image();
    let reines = image::imageops::crop_imm(&frame, 333, 431, 98, 47).to_image();
    let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    vision
        .configure_card_identities(
            vec![(PartySlot::Five, vec![reines])],
            Some(PartySlot::Three),
        )
        .unwrap();
    vision
        .configure_support_identities(vec![(704000, vec![support])])
        .unwrap();

    let state = vision
        .battle_state(&frame, &Control::new(Duration::from_secs(10)))
        .unwrap();
    assert_eq!(
        state.card_owners,
        [
            Some(PartySlot::Three),
            Some(PartySlot::Five),
            Some(PartySlot::Three),
            Some(PartySlot::Three),
            None,
        ]
    );
    assert_eq!(state.support_atlas_id, Some(704000));
}

#[test]
fn equally_good_fixed_party_identity_candidates_remain_unknown() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let raw = image::open(root.join("tests/fixtures/step12.png"))
        .unwrap()
        .to_rgba8();
    let frame = normalize(&raw, None).unwrap().0;
    let reines = image::imageops::crop_imm(&frame, 333, 431, 98, 47).to_image();
    let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    vision
        .configure_card_identities(
            vec![
                (PartySlot::Four, vec![reines.clone()]),
                (PartySlot::Five, vec![reines]),
            ],
            Some(PartySlot::Three),
        )
        .unwrap();

    let state = vision
        .battle_state(&frame, &Control::new(Duration::from_secs(10)))
        .unwrap();

    assert_eq!(state.card_owners[1], None);
}

#[test]
fn np_percent_is_requested_only_on_the_idle_battle_scene_with_evidence() {
    use fgo_rs::observation::{DetectorKind, Recognition, SceneFacts};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    for step in [4, 7, 12] {
        let raw = image::open(root.join(format!("tests/fixtures/step{step}.png")))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        for requested in [false, true] {
            let state = vision
                .battle_state_with_request(
                    &frame,
                    &Control::new(Duration::from_secs(10)),
                    &[],
                    BattleObservationRequest::new(
                        if requested {
                            RequiredBattleFacts::NP
                        } else {
                            RequiredBattleFacts::default()
                        },
                        &[],
                    ),
                )
                .unwrap();
            if step == 4 && requested {
                assert_eq!(
                    state.np_percent.map(Recognition::value),
                    [Some(0), Some(70), Some(78)]
                );
                assert_eq!(
                    state
                        .recognition_evidence()
                        .iter()
                        .filter(|e| e.evidence.detector == DetectorKind::NpPercent)
                        .count(),
                    3
                );
            } else {
                assert_eq!(state.np_percent, [Recognition::NotRequested; 3]);
                assert!(
                    serde_json::to_value(&state)
                        .unwrap()
                        .get("np_percent")
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn ally_hp_is_opt_in_and_never_decoded_from_cards_or_popups() {
    use fgo_rs::observation::{DetectorKind, Recognition, SceneFacts};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    for step in [4, 7, 12] {
        let raw = image::open(root.join(format!("tests/fixtures/step{step}.png")))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        for requested in [false, true] {
            let state = vision
                .battle_state_with_request(
                    &frame,
                    &Control::new(Duration::from_secs(10)),
                    &[],
                    BattleObservationRequest::new(
                        if requested {
                            RequiredBattleFacts::HP
                        } else {
                            RequiredBattleFacts::default()
                        },
                        &[],
                    ),
                )
                .unwrap();
            if step == 4 && requested {
                assert_eq!(
                    state.ally_hp.map(Recognition::value),
                    [Some(14219), Some(16057), Some(21455)]
                );
                assert_eq!(
                    state
                        .recognition_evidence()
                        .iter()
                        .filter(|e| e.evidence.detector == DetectorKind::AllyHp)
                        .count(),
                    3
                );
            } else {
                assert_eq!(state.ally_hp, [Recognition::NotRequested; 3]);
                assert!(
                    serde_json::to_value(state)
                        .unwrap()
                        .get("ally_hp")
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn skill_confirmation_requires_title_and_both_buttons_and_suppresses_battle_facts() {
    use fgo_rs::observation::Recognition;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    let original = image::open(root.join("tests/fixtures/live-skill-confirm.png"))
        .unwrap()
        .to_rgba8();
    for missing in [
        None,
        Some((388, 401, 76, 42)),
        Some((811, 401, 76, 42)),
        Some((556, 162, 135, 34)),
        Some((1093, 164, 33, 32)),
    ] {
        let mut frame = original.clone();
        if let Some((x, y, w, h)) = missing {
            for y in y..y + h {
                for x in x..x + w {
                    frame.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        let state = vision
            .battle_state(&frame, &Control::new(Duration::from_secs(10)))
            .unwrap();
        if missing.is_none() {
            assert_eq!(state.scene(), Scene::SkillConfirmation);
            assert!(!state.skill_error && !state.idle() && !state.cards());
            assert_eq!(state.target_count, 0);
            assert_eq!(state.ally_hp, [Recognition::NotRequested; 3]);
            assert_eq!(state.np_percent, [Recognition::NotRequested; 3]);
        } else {
            assert!(!state.skill_confirmation && state.skill_error);
        }
    }
}

#[test]
fn speed_is_requested_only_on_command_cards() {
    let v = Vision::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/templates.json"))
        .unwrap();
    for (step, facts, expected) in [
        (
            12,
            RequiredBattleFacts::SPEED,
            Some(fgo_rs::battle::BattleSpeed::Double),
        ),
        (12, RequiredBattleFacts::CARDS, None),
        (4, RequiredBattleFacts::SPEED, None),
    ] {
        let raw = image::open(format!("tests/fixtures/step{step}.png"))
            .unwrap()
            .to_rgba8();
        let state = v
            .battle_state_with_markers_and_facts(
                &normalize(&raw, None).unwrap().0,
                &Control::new(Duration::from_secs(10)),
                &[],
                facts,
            )
            .unwrap();
        assert_eq!(state.battle_speed.value(), expected);
        if expected.is_none() {
            assert_eq!(
                state.battle_speed,
                fgo_rs::observation::Recognition::NotRequested
            );
        }
    }
}
