use anyhow::Result;
use fgo_rs::{
    battle::{BattleDriver, BattlePort, BattleState, Color, rank_cards},
    vision::Point,
};
use std::collections::VecDeque;

#[derive(Default)]
struct Port {
    frames: VecDeque<BattleState>,
    last: BattleState,
    taps: Vec<Point>,
    time: u64,
    logs: Vec<String>,
}
impl Port {
    fn new(frames: Vec<BattleState>) -> Self {
        Self {
            frames: frames.into(),
            ..Default::default()
        }
    }
    fn driver(&mut self) -> BattleDriver<'_, Self> {
        BattleDriver { port: self }
    }
}
impl BattlePort for Port {
    fn observe(&mut self) -> Result<BattleState> {
        if let Some(s) = self.frames.pop_front() {
            self.last = s;
        }
        Ok(self.last.clone())
    }
    fn tap(&mut self, p: Point) -> Result<()> {
        self.taps.push(p);
        Ok(())
    }
    fn sleep(&mut self, ms: u64) -> Result<()> {
        self.time += ms;
        Ok(())
    }
    fn elapsed_ms(&self) -> u64 {
        self.time
    }
    fn log(&mut self, message: &str) -> Result<()> {
        self.logs.push(message.into());
        Ok(())
    }
}
fn idle() -> BattleState {
    BattleState {
        turn: true,
        first_turn: true,
        iskandar: true,
        taigong: true,
        support: true,
        skill_ready: [[true; 3]; 3],
        np_glow: [true; 3],
        ..Default::default()
    }
}
fn master() -> BattleState {
    BattleState {
        master_open: true,
        master_ready: [true; 3],
        ..idle()
    }
}
fn cards() -> BattleState {
    BattleState {
        colors: [Some(Color::Arts); 5],
        resist: [1.0; 5],
        np_glow: [true; 3],
        ..Default::default()
    }
}

#[test]
fn delayed_target_popup_is_handled_once_and_cooldown_is_required() {
    let popup = BattleState {
        target_count: 2,
        ..Default::default()
    };
    let mut done = idle();
    done.skill_ready[0][0] = false;
    let mut port = Port::new(vec![idle(), idle(), popup.clone(), popup, done]);
    assert!(port.driver().cast_skill(1, 1, true).unwrap());
    assert_eq!(
        port.taps,
        vec![Point { x: 73, y: 573 }, Point { x: 875, y: 440 }]
    );
}

#[test]
fn cooldown_failure_does_not_repeat_a_skill_click() {
    let mut port = Port::new(vec![idle()]);
    let error = port.driver().cast_skill(1, 1, true).unwrap_err();
    assert!(error.to_string().contains("did not enter cooldown"));
    assert_eq!(port.taps, vec![Point { x: 73, y: 573 }]);
    assert!(port.time >= 35000 && port.time < 37000);
}

#[test]
fn unknown_targets_and_skill_errors_stop_before_target_input() {
    for frame in [
        BattleState {
            target_count: 4,
            ..Default::default()
        },
        BattleState {
            skill_error: true,
            ..Default::default()
        },
    ] {
        let mut port = Port::new(vec![idle(), frame]);
        assert!(port.driver().cast_skill(3, 1, true).is_err());
        assert_eq!(port.taps.len(), 1);
    }
}

#[test]
fn required_skills_fail_and_optional_skills_skip_on_cooldown() {
    let mut s = idle();
    s.skill_ready[0][0] = false;
    let mut port = Port::new(vec![s]);
    assert!(!port.driver().cast_skill(1, 1, false).unwrap());
    assert!(port.driver().cast_skill(1, 1, true).is_err());
    assert!(port.driver().cast_skill(0, 1, false).is_err());
    assert!(port.taps.is_empty());
}

#[test]
fn opening_rejects_a_partially_used_or_wrong_team() {
    let mut s = idle();
    s.skill_ready[0][0] = false;
    let mut port = Port::new(vec![s]);
    assert!(port.driver().verify_opening().is_err());
    let mut s = idle();
    s.iskandar = false;
    let mut port = Port::new(vec![s]);
    assert!(port.driver().verify_opening().is_err());
}

#[test]
fn order_change_verifies_both_selections_and_reines_afterwards() {
    let dialog = BattleState {
        order_change: true,
        ..Default::default()
    };
    let mut first = dialog.clone();
    first.selected[0] = true;
    let mut both = first.clone();
    both.selected[4] = true;
    let after = BattleState {
        iskandar: false,
        reines: true,
        ..idle()
    };
    let mut port = Port::new(vec![master(), dialog, first, both, after]);
    port.driver().order_change().unwrap();
    assert_eq!(
        port.taps,
        vec![
            Point { x: 1084, y: 317 },
            Point { x: 140, y: 360 },
            Point { x: 940, y: 360 },
            Point { x: 640, y: 629 }
        ]
    );
}

#[test]
fn wrong_reserve_selection_is_not_confirmed() {
    let dialog = BattleState {
        order_change: true,
        ..Default::default()
    };
    let mut first = dialog.clone();
    first.selected[0] = true;
    first.selected[3] = true;
    let mut port = Port::new(vec![master(), dialog, first]);
    assert!(port.driver().order_change().is_err());
    assert!(!port.taps.contains(&Point { x: 640, y: 629 }));
}

#[test]
fn failed_order_change_does_not_repeat_the_swap() {
    let dialog = BattleState {
        order_change: true,
        ..Default::default()
    };
    let mut first = dialog.clone();
    first.selected[0] = true;
    let mut both = first.clone();
    both.selected[4] = true;
    let mut port = Port::new(vec![master(), dialog, first, both, idle()]);
    assert!(
        port.driver()
            .order_change()
            .unwrap_err()
            .to_string()
            .contains("Reines")
    );
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 1084, y: 317 })
            .count(),
        1
    );
}

#[test]
fn master_panel_retries_dropped_click_and_checks_cooldown() {
    let mut cooling = master();
    cooling.master_ready[0] = false;
    // Two open-panel clicks (first dropped), skill, open after animation, close.
    let mut port = Port::new(vec![
        idle(),
        idle(),
        master(),
        idle(),
        idle(),
        cooling.clone(),
        cooling,
        idle(),
    ]);
    assert!(port.driver().cast_master_skill(1, true).unwrap());
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 907, y: 317 })
            .count(),
        1
    );
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 1200, y: 317 })
            .count(),
        4
    );
}

#[test]
fn opening_chooses_exactly_three_nps_in_field_order() {
    let mut port = Port::new(vec![idle(), cards(), cards(), BattleState::default()]);
    port.driver().attack(true).unwrap();
    assert_eq!(
        port.taps,
        vec![
            Point { x: 1130, y: 600 },
            Point { x: 431, y: 203 },
            Point { x: 651, y: 203 },
            Point { x: 845, y: 203 }
        ]
    );
}

#[test]
fn missing_or_sealed_np_returns_without_selecting_any_cards() {
    let mut missing = cards();
    missing.np_glow[1] = false;
    let mut before = idle();
    before.np_glow[1] = false;
    let mut port = Port::new(vec![before, missing.clone(), missing]);
    assert!(port.driver().attack(true).is_err());
    assert_eq!(
        port.taps,
        vec![Point { x: 1130, y: 600 }, Point { x: 1210, y: 683 }]
    );
    let mut sealed = cards();
    sealed.np_blocked[0] = true;
    let mut port = Port::new(vec![idle(), cards(), sealed]);
    assert!(port.driver().attack(true).is_err());
    assert_eq!(port.taps.len(), 2);
}

#[test]
fn np_flicker_is_combined_across_frames_but_sealing_wins() {
    let mut a = cards();
    a.np_glow = [true, false, false];
    let mut b = cards();
    b.np_glow = [false, true, true];
    let mut before = idle();
    before.np_glow = [false; 3];
    let mut port = Port::new(vec![before, a, b, BattleState::default()]);
    port.driver().attack(true).unwrap();
}

#[test]
fn skill_screen_brightness_cannot_make_an_unready_np_available() {
    let mut unready = cards();
    unready.np_glow = [false; 3];
    let mut port = Port::new(vec![idle(), unready.clone(), unready]);
    assert!(port.driver().attack(true).is_err());
    assert_eq!(
        port.taps,
        vec![Point { x: 1130, y: 600 }, Point { x: 1210, y: 683 }]
    );
}

#[test]
fn follow_up_cards_prioritize_nps_and_unsealed_support_cards() {
    let mut s = cards();
    s.support_cards = [false, true, true, true, false];
    s.sealed[2] = true;
    s.colors = [
        Some(Color::Buster),
        Some(Color::Arts),
        Some(Color::Quick),
        Some(Color::Quick),
        Some(Color::Arts),
    ];
    assert_eq!(rank_cards(&s, [true, false, true]).unwrap(), [6, 8, 4]);
    assert_eq!(rank_cards(&s, [false; 3]).unwrap(), [4, 2, 1]);
    s.sealed = [true, true, true, true, false];
    assert!(rank_cards(&s, [false; 3]).is_err());
}

#[test]
fn support_loss_and_defeat_stop_the_battle() {
    let mut lost = idle();
    lost.support = false;
    let mut port = Port::new(vec![lost]);
    assert!(port.driver().verify_support().is_err());
    let mut port = Port::new(vec![BattleState {
        defeated: true,
        ..Default::default()
    }]);
    assert!(port.driver().wait_next().is_err());
    let mut port = Port::new(vec![BattleState {
        victory: true,
        ..Default::default()
    }]);
    assert!(!port.driver().wait_next().unwrap());
}

#[test]
fn battle_script_swaps_once_and_runs_a_follow_up_turn() {
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut engine = rhai::Engine::new();
    engine.set_max_operations(10000);
    for name in ["verify_opening", "verify_support", "order_change"] {
        let events = events.clone();
        engine.register_fn(name, move || events.lock().unwrap().push(name.into()));
    }
    let e = events.clone();
    engine.register_fn("cast_skill", move |a: i64, b: i64, strict: bool| {
        e.lock().unwrap().push(format!("skill:{a}:{b}:{strict}"));
        true
    });
    let e = events.clone();
    engine.register_fn("cast_master_skill", move |a: i64, strict: bool| {
        e.lock().unwrap().push(format!("master:{a}:{strict}"));
        true
    });
    let e = events.clone();
    engine.register_fn("attack", move |first: bool| {
        e.lock().unwrap().push(format!("attack:{first}"))
    });
    let outcomes = Mutex::new(VecDeque::from([true, false]));
    engine.register_fn("wait_next_turn", move || {
        outcomes.lock().unwrap().pop_front().unwrap()
    });
    engine.register_fn("log", |_: &str| ());
    engine.register_fn(
        "fail",
        |message: &str| -> Result<(), Box<rhai::EvalAltResult>> { Err(message.into()) },
    );
    engine
        .run(include_str!("../scripts/rider_battle.rhai"))
        .unwrap();
    let events = events.lock().unwrap();
    assert_eq!(
        &events[..4],
        [
            "verify_opening",
            "skill:1:1:true",
            "skill:1:2:true",
            "order_change"
        ]
    );
    assert_eq!(events.iter().filter(|e| *e == "order_change").count(), 1);
    assert_eq!(
        events.iter().filter(|e| e.starts_with("skill:")).count(),
        20
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.starts_with("attack:"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["attack:true", "attack:false"]
    );
    assert_eq!(events.last().unwrap(), "attack:false");
}
