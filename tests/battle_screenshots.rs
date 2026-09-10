use fgo_rs::{
    battle::Color,
    control::Control,
    vision::{Vision, normalize},
};
use std::{path::PathBuf, time::Duration};

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
fn original_screenshots_distinguish_unused_and_used_skills() {
    let s = state(4);
    assert!(s.idle() && s.first_turn && s.iskandar && s.taigong && s.support);
    assert_eq!(s.skill_ready, [[true; 3]; 3]);
    assert_eq!(
        s.np_glow, [false; 3],
        "skill-screen brightness is not a readiness check"
    );
    let s = state(5);
    assert_eq!(s.skill_ready, [[false, false, true], [true; 3], [true; 3]]);
    let s = state(10);
    assert!(s.reines && !s.iskandar && s.support);
    assert_eq!(s.skill_ready, [[false; 3]; 3]);
}

#[test]
fn original_screenshots_verify_order_change_and_master_cooldowns() {
    let s = state(6);
    assert!(s.master_open);
    assert_eq!(s.master_ready, [true; 3]);
    let s = state(7);
    assert!(s.order_change);
    assert_eq!(s.selected, [false; 6]);
    let s = state(8);
    assert!(s.order_change);
    assert_eq!(s.selected, [true, false, false, false, true, false]);
    let s = state(9);
    assert!(s.idle() && s.reines && s.support);
    assert_eq!(s.skill_ready, [[true; 3]; 3]);
    let s = state(11);
    assert!(s.master_open);
    assert_eq!(s.master_ready, [false; 3]);
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
    assert_eq!(s.np_glow, [true; 3]);
    assert_eq!(s.np_blocked, [false; 3]);
    assert_eq!(s.sealed, [false; 5]);
}
