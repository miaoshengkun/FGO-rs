use fgo_rs::{control::Control, farming::RequiredFarmFacts, vision::Vision};
use std::{path::PathBuf, time::Duration};

#[path = "support/saber_frames.rs"]
mod saber_frames;

#[test]
fn saber_retest_entry_settlement_and_numbers_reject_incomplete_evidence() {
    let vision =
        Vision::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/templates.json"))
            .unwrap();
    saber_frames::verify_retest_facts(&vision);
}

#[test]
fn saber_card_badges_survive_buff_occlusion_without_guessing_hidden_badges() {
    let mut vision =
        Vision::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/templates.json"))
            .unwrap();
    saber_frames::verify_card_facts(&mut vision);
}

fn fixture(name: &str) -> (Vision, image::RgbaImage, Control) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    (
        Vision::load(&root.join("assets/templates.json")).unwrap(),
        image::open(root.join(format!("tests/fixtures/live-{name}.png")))
            .unwrap()
            .to_rgba8(),
        Control::new(Duration::from_secs(30)),
    )
}

#[test]
fn saber_quest_vii_rejects_v_and_vi_titles_on_real_frames() {
    let (vision, without_vii, control) = fixture("saber-quests-v");
    assert!(
        vision
            .find(&without_vii, "saber_quest_vii", &control)
            .unwrap()
            .is_none()
    );
    let (_, with_vii, _) = fixture("saber-quests-vi-vii");
    let found = vision
        .find(&with_vii, "saber_quest_vii", &control)
        .unwrap()
        .unwrap();
    assert!(
        (376..431).contains(&found.y),
        "must select VII, not the VI row: {found:?}"
    );
    assert!(
        vision
            .find_in(
                &with_vii,
                "saber_quest_vii",
                Some(fgo_rs::vision::Rect {
                    x: 760,
                    y: 190,
                    width: 260,
                    height: 55,
                }),
                &control
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn scrolling_quest_frames_do_not_confuse_vi_with_vii() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vision = fgo_rs::vision::Vision::load(&root.join("assets/templates.json")).unwrap();
    for (file, expected) in [
        (
            "quest-moving-vii-visible.png",
            Some(fgo_rs::vision::Point { x: 876, y: 562 }),
        ),
        ("quest-moving-vii-outside.png", None),
    ] {
        let raw = image::open(root.join("tests/fixtures").join(file))
            .unwrap()
            .to_rgba8();
        let frame = fgo_rs::vision::normalize(&raw, None).unwrap().0;
        let state = vision
            .farm_state(
                &frame,
                &fgo_rs::control::Control::new(std::time::Duration::from_secs(10)),
            )
            .unwrap();
        assert!(state.event && state.menu);
        assert_eq!(state.quest, expected, "{file}");
    }
}

#[test]
fn bond_level_up_requires_both_title_and_tap_prompt() {
    let (vision, frame, control) = fixture("bond-level-up");
    assert!(vision.farm_state(&frame, &control).unwrap().next);
    for (x, y, width, height) in [(505, 100, 700, 100), (505, 630, 270, 40)] {
        let mut covered = frame.clone();
        for cy in y..y + height {
            for cx in x..x + width {
                covered.put_pixel(cx, cy, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!vision.farm_state(&covered, &control).unwrap().next);
    }
}

#[test]
fn ce_reward_requires_label_and_completion_prompt() {
    let (vision, frame, control) = fixture("bond-ce-reward");
    let state = vision.farm_state(&frame, &control).unwrap();
    assert!(state.special_drop && !state.next);
    for (x, y, width, height) in [(596, 601, 88, 28), (505, 674, 270, 40)] {
        let mut covered = frame.clone();
        for cy in y..y + height {
            for cx in x..x + width {
                covered.put_pixel(cx, cy, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(!vision.farm_state(&covered, &control).unwrap().special_drop);
    }
}

#[test]
fn live_quest_and_confirmation_are_distinct() {
    let (v, frame, c) = fixture("quest");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.event && s.menu && s.quest.is_some());
    assert!(!s.quest_confirm && !s.refresh_confirm && !s.ap_empty);
    let (v, frame, c) = fixture("quest-confirm");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.quest_confirm && s.quest.is_none());
    assert!(!s.support && !s.formation && !s.refresh_confirm);
}

#[test]
fn farm_scene_only_request_skips_workflow_details() {
    let (vision, frame, control) = fixture("quest");
    let state = vision
        .farm_state_with_facts(&frame, &control, RequiredFarmFacts::default())
        .unwrap();
    assert_eq!(state.scene(), fgo_rs::domain::Scene::QuestList);
    assert!(state.quest.is_none());

    let (vision, frame, control) = fixture("formation");
    let state = vision
        .farm_state_with_facts(&frame, &control, RequiredFarmFacts::default())
        .unwrap();
    assert_eq!(state.scene(), fgo_rs::domain::Scene::Formation);
    assert!(!state.level120);

    let (vision, frame, control) = fixture("apple-count-2");
    let state = vision
        .farm_state_with_facts(&frame, &control, RequiredFarmFacts::default())
        .unwrap();
    assert_eq!(state.scene(), fgo_rs::domain::Scene::AppleConfirmation);
    assert_eq!(state.apple_confirm, Some(fgo_rs::farming::Apple::Bronze));
    assert_eq!(state.apple_count, None);
}

#[test]
fn live_refresh_dialog_blocks_background_support_selection() {
    let (v, frame, c) = fixture("refresh");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.refresh_confirm);
    assert!(!s.support && s.friend.is_none() && !s.quest_confirm);
}

#[test]
fn scrolled_rider_support_list_keeps_its_scene_without_the_first_row_corner() {
    let (vision, frame, control) = fixture("support-scrolled");
    let state = vision.farm_state(&frame, &control).unwrap();

    assert_eq!(state.scene(), fgo_rs::domain::Scene::SupportList);
    assert!(state.support && state.rider_tab);
}

#[test]
fn live_formation_requires_level120_in_slot_three() {
    let (v, mut frame, c) = fixture("formation");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.formation && s.level120);
    for y in 522..554 {
        for x in 443..498 {
            frame.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.formation && !s.level120);
}

#[test]
fn enemy_details_are_not_a_skill_target_dialog() {
    let (v, frame, c) = fixture("enemy-details");
    let s = v.battle_state(&frame, &c).unwrap();
    assert!(s.skill_error && s.target_count == 0 && !s.idle());
    let (v, frame, c) = fixture("target");
    let s = v.battle_state(&frame, &c).unwrap();
    assert!(!s.skill_error && s.target_count > 0);
}

#[test]
fn live_bond_and_master_results_can_advance_settlement() {
    for name in ["bond-result", "master-result"] {
        let (v, frame, c) = fixture(name);
        let s = v.farm_state(&frame, &c).unwrap();
        assert!(s.next && !s.repeat && !s.turn && !s.ap_empty, "{name}");
        if name == "bond-result" {
            assert!(v.battle_state(&frame, &c).unwrap().victory);
        }
    }
}

#[test]
fn loot_and_repeat_are_separate_settlement_stages() {
    let (v, frame, c) = fixture("loot");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.reward && !s.repeat);
    let (v, frame, c) = fixture("repeat");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.repeat && !s.reward && !s.ap_empty && !s.quest_confirm);
}

#[test]
fn live_ap_confirmation_recognizes_kind_and_exact_batch_size() {
    use fgo_rs::farming::Apple;
    let (v, frame, c) = fixture("apple-gold");
    let s = v.farm_state(&frame, &c).unwrap();
    assert_eq!(s.apple_confirm, Some(Apple::Gold));
    assert_eq!(s.apple_count, Some(1));
    for count in 1..=4 {
        let (v, mut frame, c) = fixture(&format!("apple-count-{count}"));
        let s = v.farm_state(&frame, &c).unwrap();
        assert_eq!(
            s.apple_confirm,
            Some(if count == 1 {
                Apple::Silver
            } else {
                Apple::Bronze
            })
        );
        assert_eq!(s.apple_count, Some(count));
        assert!(!s.support && !s.formation && !s.turn && !s.quest_confirm);
        for y in 348..391 {
            for x in 707..793 {
                frame.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert_eq!(v.farm_state(&frame, &c).unwrap().apple_count, None);
    }
    for name in ["ap-empty", "ap-bottom"] {
        let (v, frame, c) = fixture(name);
        let s = v.farm_state(&frame, &c).unwrap();
        assert!(s.ap_empty && s.apple_confirm.is_none() && s.apple_count.is_none());
    }
}
