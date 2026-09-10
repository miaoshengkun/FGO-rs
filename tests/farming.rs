use anyhow::Result;
use fgo_rs::{
    battle::{BattlePort, BattleState},
    farming::{Apple, FarmDriver, FarmPort, FarmSession, FarmState},
    vision::Point,
};
use std::collections::VecDeque;

#[derive(Default)]
struct Port {
    frames: VecDeque<FarmState>,
    last: FarmState,
    taps: Vec<Point>,
    time: u64,
    swipes: usize,
}
impl Port {
    fn new(frames: Vec<FarmState>) -> Self {
        Self {
            frames: frames.into(),
            ..Default::default()
        }
    }
}
impl BattlePort for Port {
    fn observe(&mut self) -> Result<BattleState> {
        unreachable!()
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
    fn log(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
}
impl FarmPort for Port {
    fn observe_farm(&mut self) -> Result<FarmState> {
        if let Some(s) = self.frames.pop_front() {
            self.last = s;
        }
        Ok(self.last.clone())
    }
    fn swipe(&mut self, _: Point, _: Point, _: u64) -> Result<()> {
        self.swipes += 1;
        Ok(())
    }
}
fn turn() -> FarmState {
    FarmState {
        turn: true,
        ..Default::default()
    }
}
fn repeat() -> FarmState {
    FarmState {
        repeat: true,
        ..Default::default()
    }
}
fn formation() -> FarmState {
    FarmState {
        formation: true,
        level120: true,
        ..Default::default()
    }
}
fn list() -> FarmState {
    FarmState {
        support: true,
        rider_tab: true,
        support_end: true,
        ..Default::default()
    }
}

#[test]
fn exhausted_ap_budget_closes_dialog_without_selecting_a_resource() {
    let mut port = Port::new(vec![
        repeat(),
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        FarmState::default(),
    ]);
    let mut session = FarmSession::default();
    assert!(
        !FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(
        port.taps,
        vec![Point { x: 801, y: 573 }, Point { x: 640, y: 629 }]
    );
    assert_eq!(session.apples_remaining, 0);
    assert!(!session.active);
}

#[test]
fn apple_is_reserved_once_and_only_after_the_right_confirmation() {
    let mut port = Port::new(vec![
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        FarmState {
            apple_confirm: Some(Apple::Gold),
            apple_count: Some(1),
            ..Default::default()
        },
        formation(),
        formation(),
        turn(),
    ]);
    let mut session = FarmSession {
        apples_remaining: 1,
        ..Default::default()
    };
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(session.apples_remaining, 0);
    assert!(session.active);
    assert_eq!(
        port.taps,
        vec![
            Point { x: 907, y: 317 },
            Point { x: 891, y: 573 },
            Point { x: 1231, y: 687 }
        ]
    );
}

#[test]
fn wrong_apple_confirmation_never_spends_or_decrements_budget() {
    let mut port = Port::new(vec![
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        FarmState {
            apple_confirm: Some(Apple::Silver),
            ..Default::default()
        },
    ]);
    let mut session = FarmSession {
        apples_remaining: 1,
        ..Default::default()
    };
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .is_err()
    );
    assert_eq!(session.apples_remaining, 1);
    assert_eq!(port.taps.len(), 1);
}

#[test]
fn partial_ap_recovery_uses_at_most_the_remaining_apple_budget() {
    let empty = FarmState {
        ap_empty: true,
        ..Default::default()
    };
    let confirm = FarmState {
        apple_confirm: Some(Apple::Gold),
        apple_count: Some(1),
        ..Default::default()
    };
    let mut port = Port::new(vec![
        empty.clone(),
        confirm.clone(),
        empty.clone(),
        empty.clone(),
        confirm,
        empty.clone(),
        empty,
        FarmState::default(),
    ]);
    let mut session = FarmSession {
        apples_remaining: 2,
        ..Default::default()
    };
    assert!(
        !FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(session.apples_remaining, 0);
    assert!(!session.active);
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 891, y: 573 })
            .count(),
        2
    );
    assert_eq!(port.taps.last(), Some(&Point { x: 640, y: 629 }));
}

#[test]
fn quest_can_open_ap_recovery_without_a_confirmation_screen() {
    let mut port = Port::new(vec![
        FarmState {
            event: true,
            menu: true,
            quest: Some(Point { x: 800, y: 200 }),
            ..Default::default()
        },
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        FarmState::default(),
    ]);
    let mut session = FarmSession::default();
    assert!(
        !FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(
        port.taps,
        vec![Point { x: 800, y: 200 }, Point { x: 640, y: 629 }]
    );
}

#[test]
fn minimum_batch_is_cancelled_when_budget_is_too_small() {
    let empty = FarmState {
        ap_empty: true,
        ..Default::default()
    };
    let confirm = FarmState {
        apple_confirm: Some(Apple::Bronze),
        apple_count: Some(2),
        ..Default::default()
    };
    let mut port = Port::new(vec![
        empty.clone(),
        confirm.clone(),
        confirm,
        empty,
        FarmState::default(),
    ]);
    let mut session = FarmSession {
        apple: Apple::Bronze,
        apples_remaining: 1,
        ..Default::default()
    };
    assert!(
        !FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(session.apples_remaining, 1);
    assert!(!port.taps.contains(&Point { x: 891, y: 573 }));
    assert_eq!(port.taps.last(), Some(&Point { x: 640, y: 629 }));
}

#[test]
fn minimum_batch_reserves_actual_quantity_instead_of_one() {
    let confirm = FarmState {
        apple_confirm: Some(Apple::Bronze),
        apple_count: Some(3),
        ..Default::default()
    };
    let mut port = Port::new(vec![
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        confirm.clone(),
        confirm,
        turn(),
        turn(),
        turn(),
    ]);
    let mut session = FarmSession {
        apple: Apple::Bronze,
        apples_remaining: 4,
        ..Default::default()
    };
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(session.apples_remaining, 1);
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 891, y: 573 })
            .count(),
        1
    );
}

#[test]
fn ambiguous_recovery_failure_does_not_retry_consumption() {
    let mut port = Port::new(vec![
        FarmState {
            ap_empty: true,
            ..Default::default()
        },
        FarmState {
            apple_confirm: Some(Apple::Gold),
            apple_count: Some(1),
            ..Default::default()
        },
    ]);
    let mut session = FarmSession {
        apples_remaining: 2,
        ..Default::default()
    };
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .is_err()
    );
    assert_eq!(session.apples_remaining, 1);
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 891, y: 573 })
            .count(),
        1
    );
}

#[test]
fn refresh_limit_and_spacing_are_enforced_without_picking_random_supports() {
    let mut frames = Vec::new();
    for _ in 0..2 {
        frames.extend([
            list(),
            list(),
            list(),
            FarmState {
                refresh_confirm: true,
                ..Default::default()
            },
        ]);
    }
    frames.extend([list(), list()]);
    let mut port = Port::new(frames);
    let mut session = FarmSession {
        refresh_limit: 2,
        ..Default::default()
    };
    let error = FarmDriver {
        port: &mut port,
        session: &mut session,
    }
    .select_support()
    .unwrap_err();
    assert!(error.to_string().contains("after 2 refreshes"));
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 927, y: 131 })
            .count(),
        2
    );
    assert_eq!(
        port.taps
            .iter()
            .filter(|p| **p == Point { x: 801, y: 573 })
            .count(),
        2
    );
    assert!(port.time >= 10000);
    assert_eq!(port.swipes, 0);
}

#[test]
fn selecting_support_checks_formation_level_before_start() {
    let candidate = FarmState {
        friend: Some(Point { x: 800, y: 273 }),
        ..list()
    };
    let mut port = Port::new(vec![list(), candidate, formation()]);
    let mut session = FarmSession::default();
    FarmDriver {
        port: &mut port,
        session: &mut session,
    }
    .select_support()
    .unwrap();
    assert_eq!(
        port.taps,
        vec![Point { x: 363, y: 130 }, Point { x: 800, y: 273 }]
    );
    let mut port = Port::new(vec![FarmState {
        formation: true,
        ..Default::default()
    }]);
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .is_err()
    );
    assert!(port.taps.is_empty());
}

#[test]
fn known_result_pages_advance_then_count_one_battle() {
    let mut port = Port::new(vec![
        FarmState {
            next: true,
            ..Default::default()
        },
        FarmState {
            reward: true,
            ..Default::default()
        },
        repeat(),
    ]);
    let mut session = FarmSession {
        active: true,
        ..Default::default()
    };
    FarmDriver {
        port: &mut port,
        session: &mut session,
    }
    .finish()
    .unwrap();
    assert_eq!(session.completed, 1);
    assert!(!session.active);
    assert_eq!(port.taps.len(), 2);
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .finish()
        .is_err()
    );
    assert!(
        !FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(port.taps.len(), 2);
}

#[test]
fn unknown_settlement_screens_timeout_without_random_taps() {
    let mut port = Port::default();
    let mut session = FarmSession {
        active: true,
        ..Default::default()
    };
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .finish()
        .is_err()
    );
    assert!(port.taps.is_empty());
    assert_eq!(session.completed, 0);
}

#[test]
fn new_quest_requires_event_context_and_confirmation_before_start() {
    let mut port = Port::new(vec![
        FarmState {
            event: true,
            menu: true,
            quest: Some(Point { x: 876, y: 205 }),
            ..Default::default()
        },
        FarmState {
            quest_confirm: true,
            ..Default::default()
        },
        formation(),
        turn(),
    ]);
    let mut session = FarmSession::default();
    assert!(
        FarmDriver {
            port: &mut port,
            session: &mut session
        }
        .enter()
        .unwrap()
    );
    assert_eq!(
        port.taps,
        vec![
            Point { x: 876, y: 205 },
            Point { x: 801, y: 573 },
            Point { x: 1231, y: 687 }
        ]
    );
}
