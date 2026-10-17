use fgo_rs::{battle::Color, profile::PartyProfile, runtime::Runtime};
use std::sync::{Arc, Mutex};

const FARM: &str = include_str!("../scripts/assassin_farm.rhai");

fn profile() -> PartyProfile {
    serde_json::from_str(include_str!("../profiles/assassin-v1.json")).unwrap()
}

#[test]
fn real_assassin_cards_preserve_support_priority_and_np_order() {
    use fgo_rs::{
        battle::rank_cards_for_party, control::Control, domain::PartySlot, vision::Vision,
    };
    use std::{path::Path, time::Duration};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
    vision
        .configure_card_identities(vec![], Some(PartySlot::Two))
        .unwrap();
    let frame = image::open(root.join("tests/fixtures/live-assassin-cards.png"))
        .unwrap()
        .to_rgba8();
    let state = vision
        .battle_state(&frame, &Control::new(Duration::from_secs(30)))
        .unwrap();
    assert_eq!(
        state.colors,
        [
            Color::Quick,
            Color::Arts,
            Color::Buster,
            Color::Arts,
            Color::Quick
        ]
        .map(Some)
    );
    assert_eq!(state.support_cards, [true, false, true, true, false]);
    let frontline = [
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ];
    assert_eq!(
        rank_cards_for_party(&state, [false, true, true], frontline, &profile().cards).unwrap(),
        [8, 7, 3]
    );
    assert_eq!(
        rank_cards_for_party(&state, [false; 3], frontline, &profile().cards).unwrap(),
        [3, 4, 1]
    );
}

#[test]
fn assassin_strategy_recasts_after_cooldown_reduction_and_hands_off_after_opening() {
    let profile = profile();
    profile.validate().unwrap();
    assert_eq!(profile.support_candidates[0].atlas_id, 605100);
    assert_eq!(profile.support_field.index(), 1);
    assert_eq!(
        profile.cards.face_card_color_order,
        Some([Color::Buster, Color::Arts, Color::Quick])
    );
    Runtime::check(FARM).unwrap();
    for extra_turn in [false, true] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut engine = rhai::Engine::new();
        engine.register_fn("require_profile", |id: &str| {
            assert_eq!(id, "cn-assassin-grand-vii")
        });
        engine.register_fn("verify_opening", || {});
        engine.register_fn("verify_support", || {});
        engine.register_fn("log", |_: &str| {});
        engine.register_fn("fail", |message: &str| panic!("{message}"));
        let out = events.clone();
        engine.register_fn("cast_skill", move |slot: i64, skill: i64, strict: bool| {
            out.lock()
                .unwrap()
                .push(format!("skill:{slot}:{skill}:{strict}"));
            true
        });
        let out = events.clone();
        engine.register_fn("cast_master_skill", move |slot: i64, strict: bool| {
            assert!(strict);
            out.lock().unwrap().push(format!("master:{slot}"));
            true
        });
        let out = events.clone();
        engine.register_fn("attack_nps", move |slots: rhai::Array, required: bool| {
            assert!(required);
            assert_eq!(
                slots
                    .into_iter()
                    .map(|v| v.cast::<i64>())
                    .collect::<Vec<_>>(),
                [3, 2]
            );
            out.lock().unwrap().push("opening_attack".into());
        });
        let out = events.clone();
        engine.register_fn("attack", move |first: bool| {
            assert!(!first);
            out.lock().unwrap().push("auto_attack".into());
        });
        let turn = Arc::new(Mutex::new(0));
        engine.register_fn("wait_next_turn", move || {
            let mut turn = turn.lock().unwrap();
            *turn += 1;
            extra_turn && *turn == 1
        });
        let battle = Arc::new(Mutex::new(0));
        engine.register_fn("enter_battle", move || {
            let mut battle = battle.lock().unwrap();
            *battle += 1;
            *battle == 1
        });
        let out = events.clone();
        engine.register_fn("finish_battle", move || {
            out.lock().unwrap().push("finish".into())
        });
        engine.register_fn("farm_progress", || {
            let mut map = rhai::Map::new();
            map.insert("completed".into(), 1_i64.into());
            map.insert("apples_remaining".into(), 0_i64.into());
            map
        });
        engine.run(FARM).unwrap();
        let events = events.lock().unwrap();
        assert_eq!(
            &events[..13],
            [
                "skill:2:1:true",
                "skill:2:2:true",
                "skill:2:3:true",
                "skill:1:1:true",
                "skill:1:2:true",
                "skill:1:3:true",
                "skill:2:2:true",
                "skill:3:1:true",
                "skill:3:2:true",
                "skill:3:3:true",
                "master:1",
                "master:2",
                "opening_attack",
            ]
        );
        if extra_turn {
            let expected: Vec<_> = [2, 1, 3]
                .into_iter()
                .flat_map(|slot| (1..=3).map(move |skill| format!("skill:{slot}:{skill}:false")))
                .collect();
            assert_eq!(&events[13..22], expected);
            assert_eq!(events[22], "auto_attack");
        }
        assert_eq!(events.last().unwrap(), "finish");
    }
}
