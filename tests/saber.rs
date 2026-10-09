use fgo_rs::{profile::PartyProfile, runtime::Runtime};
use std::sync::{Arc, Mutex};

const BATTLE: &str = include_str!("../scripts/saber_battle.rhai");
const FARM: &str = include_str!("../scripts/saber_farm.rhai");

#[test]
fn saber_profile_and_scripts_validate() {
    let profile: PartyProfile =
        serde_json::from_str(include_str!("../profiles/saber-v1.json")).unwrap();
    profile.validate().unwrap();
    assert_eq!(profile.support_candidates[0].atlas_id, 106000);
    assert!(
        profile
            .required_templates()
            .iter()
            .any(|name| name == "support_iori_name")
    );
    Runtime::check(BATTLE).unwrap();
    Runtime::check(FARM).unwrap();
}

// Run the actual strategy against deterministic semantic callbacks. Device
// input and recognition are tested separately by the BattleDriver tests.
fn run_strategy(source: &str, ready_slot: i64, recharge: bool) -> Vec<String> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut engine = rhai::Engine::new();
    engine.register_fn("require_profile", |id: &str| {
        assert_eq!(id, "cn-saber-grand-vii")
    });
    engine.register_fn("verify_opening", || {});
    engine.register_fn("verify_support", || {});
    engine.register_fn("log", |_: &str| {});
    engine.register_fn("fail", |message: &str| panic!("{message}"));
    let out = events.clone();
    engine.register_fn("order_change", move || {
        out.lock().unwrap().push("swap".into())
    });
    let out = events.clone();
    engine.register_fn("cast_master_skill", move |slot: i64, _: bool| {
        out.lock().unwrap().push(format!("master:{slot}"));
        true
    });
    let out = events.clone();
    engine.register_fn("cast_skill", move |slot: i64, skill: i64, strict: bool| {
        out.lock()
            .unwrap()
            .push(format!("skill:{slot}:{skill}:{strict}"));
        true
    });
    let out = events.clone();
    engine.register_fn("attack_np", move |slot: i64, required: bool| {
        let mut events = out.lock().unwrap();
        events.push(format!("np:{slot}:{required}"));
        required
            || slot == ready_slot
            || (recharge
                && slot == 3
                && events.iter().filter(|e| *e == "skill:3:1:false").count() >= 2)
    });
    let out = events.clone();
    engine.register_fn("attack", move |_: bool| {
        out.lock().unwrap().push("fallback".into())
    });
    let out = events.clone();
    engine.register_fn("attack_other_nps", move || {
        out.lock().unwrap().push("other_nps".into());
        ready_slot == 1 || ready_slot == 2
    });
    let turn = Arc::new(Mutex::new(0));
    engine.register_fn("wait_next_turn", move || {
        let mut turn = turn.lock().unwrap();
        *turn += 1;
        *turn == 1
    });
    let battle = Arc::new(Mutex::new(0));
    engine.register_fn("enter_battle", move || {
        let mut battle = battle.lock().unwrap();
        *battle += 1;
        *battle == 1
    });
    engine.register_fn("finish_battle", || {});
    engine.register_fn("farm_progress", || {
        let mut map = rhai::Map::new();
        map.insert("completed".into(), 1_i64.into());
        map.insert("apples_remaining".into(), 0_i64.into());
        map
    });
    engine.run(source).unwrap();
    events.lock().unwrap().clone()
}

#[test]
fn saber_opening_and_follow_up_priorities_match_in_both_entry_points() {
    for source in [BATTLE, FARM] {
        for ready in [3, 1, 2] {
            let events = run_strategy(source, ready, false);
            assert_eq!(&events[..3], ["skill:1:1:true", "skill:1:3:true", "swap"]);
            let opening_end = events.iter().position(|e| e == "np:3:true").unwrap();
            assert_eq!(events[opening_end - 1], "master:1");
            let opening_skills: Vec<_> = events
                .iter()
                .filter(|e| e.starts_with("skill:") && e.ends_with("false"))
                .cloned()
                .collect();
            assert_eq!(opening_skills.len(), 9);
            assert_eq!(
                events.last().unwrap(),
                if ready == 3 {
                    "np:3:false"
                } else {
                    "other_nps"
                }
            );
        }
        let charged = run_strategy(source, 0, true);
        assert!(charged.ends_with(&["skill:3:1:false".into(), "np:3:false".into()]));
        let empty = run_strategy(source, 0, false);
        let skills: Vec<_> = empty
            .iter()
            .skip(empty.iter().position(|e| e == "np:3:true").unwrap() + 1)
            .filter(|e| e.starts_with("skill:") && e.ends_with("false"))
            .cloned()
            .collect();
        let expected: Vec<_> = [3, 1, 2]
            .into_iter()
            .flat_map(|slot| (1..=3).map(move |skill| format!("skill:{slot}:{skill}:false")))
            .collect();
        assert_eq!(skills, expected);
        assert_eq!(empty.last().unwrap(), "fallback");
    }
}
