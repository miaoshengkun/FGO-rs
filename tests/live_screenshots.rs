use fgo_rs::{control::Control, vision::Vision};
use std::{path::PathBuf, time::Duration};

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
fn live_refresh_dialog_blocks_background_support_selection() {
    let (v, frame, c) = fixture("refresh");
    let s = v.farm_state(&frame, &c).unwrap();
    assert!(s.refresh_confirm);
    assert!(!s.support && s.friend.is_none() && !s.quest_confirm);
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
