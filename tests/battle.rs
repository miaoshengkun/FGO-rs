use anyhow::Result;
use fgo_rs::{
    battle::{
        BattleDriver, BattleObservationRequest, BattlePort, BattleState, CardGroup, Color,
        EnemyCharge, EnemyLayout, EnemyState, RequiredBattleFacts, SkillState, rank_cards,
        rank_cards_for_party, rank_cards_with_party_data,
    },
    domain::{Command, EnemySlot, FieldSlot, PartySlot, Scene, SkillSlot},
    executor::Executor,
    observation::{Recognition, UnknownReason},
    profile::{
        CardStrategyProfile, EnemyTargetStrategy, FaceCardPriority, NpBelowCondition,
        OrderChangeProfile, PartyProfile, SkillRuleConditionAlternative,
    },
    vision::Point,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
struct Port {
    frames: VecDeque<BattleState>,
    last: BattleState,
    taps: Vec<Point>,
    time: u64,
    logs: Vec<String>,
    commands: Vec<Command>,
    requested_facts: Vec<RequiredBattleFacts>,
    expected_scenes: Vec<Vec<fgo_rs::domain::Scene>>,
    frame_id: u64,
    roster_ready_after: u64,
    profile: PartyProfile,
    progress: Option<fgo_rs::battle::BattleProgress>,
    frontline: Option<[Option<PartySlot>; 3]>,
    atlas_np_colors: BTreeMap<u32, Color>,
}
impl Port {
    fn new(frames: Vec<BattleState>) -> Self {
        let profile = PartyProfile {
            enemy_target: EnemyTargetStrategy::Current,
            ..PartyProfile::default()
        };
        Self {
            frames: frames.into(),
            profile,
            frontline: Some([
                Some(PartySlot::Five),
                Some(PartySlot::Two),
                Some(PartySlot::Three),
            ]),
            ..Default::default()
        }
    }
    fn driver(&mut self) -> BattleDriver<'_, Self> {
        BattleDriver { port: self }
    }
}
impl Executor for Port {
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
    fn current_frame_id(&self) -> u64 {
        self.frame_id
    }
    fn verification_origin(&self) -> fgo_rs::verification::VerificationOrigin {
        fgo_rs::verification::VerificationOrigin {
            last_action: self.commands.last().copied(),
            action_frame: self.commands.last().map(|_| self.frame_id),
        }
    }
    fn log(&mut self, message: &str) -> Result<()> {
        self.logs.push(message.into());
        Ok(())
    }
    fn command(&mut self, command: Command) -> Result<()> {
        self.commands.push(command);
        Ok(())
    }
}
impl BattlePort for Port {
    fn observe(&mut self) -> Result<BattleState> {
        self.frame_id += 1;
        if let Some(s) = self.frames.pop_front() {
            self.last = s;
        }
        Ok(self.last.clone())
    }
    fn observe_required(&mut self, facts: RequiredBattleFacts) -> Result<BattleState> {
        self.requested_facts.push(facts);
        self.observe()
    }
    fn observe_for(&mut self, request: BattleObservationRequest<'_>) -> Result<BattleState> {
        self.expected_scenes.push(request.expected_scenes.to_vec());
        self.observe_required(request.facts)
    }
    fn battle_progress(&self) -> Option<fgo_rs::battle::BattleProgress> {
        self.progress.clone()
    }
    fn roster_ready(&self) -> bool {
        self.frame_id > self.roster_ready_after
    }
    fn profile(&self) -> &PartyProfile {
        &self.profile
    }
    fn frontline(&self) -> [Option<PartySlot>; 3] {
        self.frontline.unwrap_or([
            Some(PartySlot::One),
            Some(PartySlot::Two),
            Some(PartySlot::Three),
        ])
    }
    fn atlas_noble_phantasm_color(&self, atlas_id: u32) -> Option<Color> {
        self.atlas_np_colors.get(&atlas_id).copied()
    }
}
fn idle() -> BattleState {
    BattleState {
        turn: true,
        first_turn: true,
        iskandar: true,
        taigong: true,
        support: true,
        skills: [[SkillState::Available.into(); 3]; 3],
        np_glow: [true; 3],
        ..Default::default()
    }
}
fn master() -> BattleState {
    BattleState {
        master_open: true,
        master_skills: [SkillState::Available.into(); 3],
        ..idle()
    }
}
fn cards() -> BattleState {
    BattleState {
        battle_speed: fgo_rs::battle::BattleSpeed::Double.into(),
        colors: [Some(Color::Arts); 5],
        resist: [1.0; 5],
        np_glow: [true; 3],
        ..Default::default()
    }
}

fn marked(mut state: BattleState, markers: &[&str]) -> BattleState {
    state.markers = markers.iter().map(|name| (*name).to_owned()).collect();
    state
}

#[test]
fn default_battle_casts_current_frontline_skills_before_nps_and_cards_without_profile_markers() {
    let mut turn = BattleState {
        turn: true,
        skills: [[SkillState::Cooldown.into(); 3]; 3],
        np_percent: [0.into(), 100.into(), 0.into()],
        ..Default::default()
    };
    turn.skills[0][0] = SkillState::Available.into();
    let mut cooling = turn.clone();
    cooling.skills[0][0] = SkillState::Cooldown.into();
    let cards = BattleState {
        np_glow: [false, true, false],
        ..cards()
    };
    let mut frames = vec![turn.clone(), turn, cooling.clone()];
    frames.extend(vec![cooling; 9]);
    frames.extend([
        cards.clone(),
        cards,
        BattleState::default(),
        BattleState::default(),
        BattleState {
            victory: true,
            ..Default::default()
        },
    ]);
    let mut port = Port::new(frames);
    // A configured Rider strategy must not leak into this independent mode.
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;
    port.driver().fight_default().unwrap();
    assert_eq!(
        port.commands,
        vec![
            Command::CastServantSkill {
                servant: FieldSlot::One,
                skill: SkillSlot::One,
                target: fgo_rs::domain::Target::Ally {
                    slot: FieldSlot::One
                },
                strict: false,
            },
            Command::Attack { opening: false },
            Command::SelectCards,
        ]
    );
    assert_eq!(port.taps.len(), 5);
    assert_eq!(
        port.taps[0],
        fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[0][0]
    );
    assert_eq!(port.taps[1], fgo_rs::layout::UI_LAYOUT_V1.battle.attack);
    assert_eq!(
        port.taps[2],
        fgo_rs::layout::UI_LAYOUT_V1.battle.noble_phantasms[1]
    );
    assert!(
        port.requested_facts
            .iter()
            .all(|facts| !facts.contains(RequiredBattleFacts::MARKERS)
                && !facts.contains(RequiredBattleFacts::ENEMIES))
    );
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::SKILLS))
    );
}

#[test]
fn default_battle_ignores_bright_np_slots_without_confirmed_charge() {
    let turn = BattleState {
        turn: true,
        skills: [[SkillState::Cooldown.into(); 3]; 3],
        // Two surviving servants and one empty slot. NP glow alone must not
        // turn any of these into a ready NP, even with stale party identities.
        np_percent: [
            85.into(),
            19.into(),
            Recognition::unknown(UnknownReason::NoCandidate, None),
        ],
        ..Default::default()
    };
    let mut frames = vec![turn; 11];
    frames.extend([
        cards(),
        cards(),
        BattleState::default(),
        BattleState::default(),
        BattleState {
            victory: true,
            ..Default::default()
        },
    ]);
    let mut port = Port::new(frames);
    port.driver().fight_default().unwrap();
    assert_eq!(
        port.commands,
        [Command::Attack { opening: false }, Command::SelectCards]
    );
    assert_eq!(port.taps.len(), 4);
    assert!(port.taps[1..].iter().all(|point| {
        fgo_rs::layout::UI_LAYOUT_V1
            .battle
            .face_cards
            .contains(point)
    }));
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::NP))
    );
}

#[test]
fn default_skills_skip_unknown_and_disabled_and_target_each_current_caster() {
    let mut state = BattleState {
        turn: true,
        skills: [[SkillState::Cooldown.into(); 3]; 3],
        ..Default::default()
    };
    state.skills[0][0] = Recognition::unknown(UnknownReason::NoCandidate, None);
    state.skills[0][1] = SkillState::Disabled.into();
    for slot in 0..3 {
        state.skills[slot][2] = SkillState::Available.into();
    }
    let mut frames = Vec::new();
    for slot in 0..3 {
        frames.extend(vec![state.clone(); 3]);
        frames.push(BattleState {
            target_count: 3,
            ..Default::default()
        });
        state.skills[slot][2] = SkillState::Cooldown.into();
        frames.push(state.clone());
    }
    let mut port = Port::new(frames);
    // Automatic skills use visible field positions, including replacements,
    // rather than original party identities or the profile's support target.
    port.frontline = Some([None; 3]);
    assert_eq!(port.driver().cast_default_skills().unwrap(), 3);
    for (index, command) in port.commands.iter().enumerate() {
        let slot = FieldSlot::try_from(index + 1).unwrap();
        assert_eq!(
            *command,
            Command::CastServantSkill {
                servant: slot,
                skill: SkillSlot::Three,
                target: fgo_rs::domain::Target::Ally { slot },
                strict: false,
            }
        );
        assert_eq!(
            port.taps[index * 2 + 1],
            fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[index]
        );
    }
    assert!(
        port.logs
            .iter()
            .any(|line| line.contains("servant 1 skill 1: readiness unknown"))
    );
}

#[test]
fn default_skills_refresh_readiness_after_a_cooldown_change() {
    let mut state = BattleState {
        turn: true,
        skills: [[SkillState::Cooldown.into(); 3]; 3],
        ..Default::default()
    };
    state.skills[0][0] = SkillState::Available.into();
    let mut changed = state.clone();
    changed.skills[0][0] = SkillState::Cooldown.into();
    changed.skills[0][1] = SkillState::Available.into();
    let mut cooling = changed.clone();
    cooling.skills[0][1] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![state, changed.clone(), changed, cooling]);
    assert_eq!(port.driver().cast_default_skills().unwrap(), 2);
    assert!(matches!(
        port.commands[1],
        Command::CastServantSkill {
            skill: SkillSlot::Two,
            ..
        }
    ));
}

#[test]
fn default_skills_confirm_once_then_select_caster_and_verify_cooldown() {
    let state = BattleState {
        turn: true,
        skills: [[SkillState::Available.into(); 3]; 3],
        ..Default::default()
    };
    let cooling = BattleState {
        skills: [[SkillState::Cooldown.into(); 3]; 3],
        ..state.clone()
    };
    let mut port = Port::new(vec![
        state,
        skill_confirmation(),
        skill_confirmation(),
        BattleState {
            target_count: 3,
            ..Default::default()
        },
        cooling,
    ]);
    assert!(!port.profile.confirm_skill_use);
    assert_eq!(port.driver().cast_default_skills().unwrap(), 1);
    assert_eq!(
        port.commands
            .iter()
            .filter(|command| **command == Command::ConfirmSkill)
            .count(),
        1
    );
    assert_eq!(
        port.taps,
        vec![
            fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[0][0],
            fgo_rs::layout::UI_LAYOUT_V1.battle.skill_confirm,
            fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[0],
        ]
    );
}

#[test]
fn default_skill_failure_stops_before_attack_and_never_retries_cast_or_confirmation() {
    for after_click in [
        BattleState {
            turn: true,
            skills: [[SkillState::Available.into(); 3]; 3],
            ..Default::default()
        },
        skill_confirmation(),
    ] {
        let before = BattleState {
            turn: true,
            skills: [[SkillState::Available.into(); 3]; 3],
            ..Default::default()
        };
        let mut port = Port::new(vec![before.clone(), before, after_click]);
        assert!(port.driver().fight_default().is_err());
        assert_eq!(
            port.commands
                .iter()
                .filter(|command| matches!(command, Command::CastServantSkill { .. }))
                .count(),
            1
        );
        assert!(
            port.commands
                .iter()
                .filter(|command| **command == Command::ConfirmSkill)
                .count()
                <= 1
        );
        assert!(
            !port
                .commands
                .iter()
                .any(|command| matches!(command, Command::Attack { .. }))
        );
    }
    let mut port = Port::new(vec![skill_confirmation()]);
    assert!(port.driver().cast_default_skills().is_err());
    assert!(port.taps.is_empty());
    assert!(port.commands.is_empty());
}

#[test]
fn default_battle_keeps_unknown_card_and_defeat_stops() {
    let mut incomplete = cards();
    incomplete.colors[0] = None;
    let mut frames = vec![
        BattleState {
            turn: true,
            ..Default::default()
        };
        11
    ];
    frames.push(incomplete);
    let mut port = Port::new(frames);
    assert!(port.driver().fight_default().is_err());
    assert_eq!(port.taps, vec![fgo_rs::layout::UI_LAYOUT_V1.battle.attack]);
    let mut port = Port::new(vec![BattleState {
        defeated: true,
        ..Default::default()
    }]);
    assert!(port.driver().fight_default().is_err());
    assert!(port.taps.is_empty());
}

#[test]
fn delayed_target_popup_is_handled_once_and_cooldown_is_required() {
    let popup = BattleState {
        target_count: 2,
        ..Default::default()
    };
    let mut done = idle();
    done.skills[0][0] = SkillState::Cooldown.into();
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
    s.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![s]);
    assert!(!port.driver().cast_skill(1, 1, false).unwrap());
    assert!(port.driver().cast_skill(1, 1, true).is_err());
    assert!(port.driver().cast_skill(0, 1, false).is_err());
    assert!(port.taps.is_empty());
}

#[test]
fn unknown_skill_state_stops_before_input_even_when_optional() {
    let mut state = idle();
    state.skills[0][0] = Recognition::unknown(UnknownReason::NoCandidate, None);
    let mut port = Port::new(vec![state]);

    let error = port.driver().cast_skill(1, 1, false).unwrap_err();

    assert!(error.to_string().contains("state is unknown"));
    assert!(port.taps.is_empty());
    assert!(port.commands.is_empty());
}

#[test]
fn unsettled_roster_times_out_before_skill_or_attack_input() {
    for attack in [false, true] {
        let mut port = Port::new(vec![idle()]);
        port.roster_ready_after = u64::MAX;
        let result = if attack {
            port.driver().attack(false)
        } else {
            port.driver().cast_skill(1, 1, true).map(|_| ())
        };
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(port.taps.is_empty());
        assert!(port.commands.is_empty());
        assert!(port.time >= 45000);
    }
}

#[test]
fn waiting_for_next_turn_waits_for_roster_confirmation() {
    let mut port = Port::new(vec![idle()]);
    port.roster_ready_after = 2;
    assert!(port.driver().wait_next().unwrap());
    assert_eq!(port.frame_id, 3);
    assert_eq!(port.time, 300);
    assert!(port.taps.is_empty());
}

#[test]
fn opening_rejects_a_partially_used_or_wrong_team() {
    let mut s = idle();
    s.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![s]);
    assert!(port.driver().verify_opening().is_err());
    let mut s = idle();
    s.iskandar = false;
    let mut port = Port::new(vec![s]);
    assert!(port.driver().verify_opening().is_err());
}

#[test]
fn opening_retries_a_single_missing_marker_before_any_input() {
    let mut transient = idle();
    transient.iskandar = false;
    let mut port = Port::new(vec![transient, idle()]);

    port.driver().verify_opening().unwrap();

    assert_eq!(port.frame_id, 2);
    assert!(port.time >= 150);
    assert!(port.taps.is_empty());
    assert!(port.commands.is_empty());
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
fn a_second_profile_changes_markers_and_order_slots_without_driver_changes() {
    let profile = PartyProfile {
        id: "virtual-second-party".into(),
        opening_markers: vec!["alpha".into(), "friend_slot_2".into()],
        support_marker: "friend_slot_2".into(),
        support_field: FieldSlot::Two,
        order_change: Some(OrderChangeProfile {
            master_skill: SkillSlot::Two,
            field: PartySlot::Two,
            reserve: PartySlot::Six,
            before_markers: vec!["alpha".into(), "friend_slot_2".into()],
            after_markers: vec!["omega".into(), "friend_slot_2".into()],
        }),
        ..PartyProfile::default()
    };
    profile.validate().unwrap();

    let opening = marked(idle(), &["alpha", "friend_slot_2"]);
    let mut port = Port {
        profile: profile.clone(),
        ..Port::new(vec![opening])
    };
    port.driver().verify_opening().unwrap();

    let mut before = marked(master(), &["alpha", "friend_slot_2"]);
    before.master_skills = [
        SkillState::Cooldown.into(),
        SkillState::Available.into(),
        SkillState::Cooldown.into(),
    ];
    let dialog = BattleState {
        order_change: true,
        ..Default::default()
    };
    let mut field = dialog.clone();
    field.selected[1] = true;
    let mut both = field.clone();
    both.selected[5] = true;
    let after = marked(
        BattleState {
            turn: true,
            ..Default::default()
        },
        &["omega", "friend_slot_2"],
    );
    let mut port = Port {
        profile,
        ..Port::new(vec![before, dialog, field, both, after])
    };
    port.driver().order_change().unwrap();
    assert_eq!(
        port.taps,
        vec![
            Point { x: 995, y: 317 },
            Point { x: 340, y: 360 },
            Point { x: 1140, y: 360 },
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
            .contains("confirming order change result")
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
    cooling.master_skills[0] = SkillState::Cooldown.into();
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
    assert_eq!(
        port.requested_facts,
        vec![
            RequiredBattleFacts::MARKERS,
            RequiredBattleFacts::CARDS.union(RequiredBattleFacts::SPEED),
            RequiredBattleFacts::CARDS.union(RequiredBattleFacts::SPEED),
            RequiredBattleFacts::default(),
            RequiredBattleFacts::default(),
        ]
    );
    assert_eq!(
        port.expected_scenes,
        vec![
            vec![Scene::BattleTurn],
            vec![Scene::CommandCards],
            vec![Scene::CommandCards],
            vec![Scene::BattleAnimation],
            vec![Scene::BattleAnimation],
        ]
    );
}

#[test]
fn selected_np_does_not_require_or_select_other_nps() {
    for glow in [[false, false, true], [true; 3]] {
        let mut screen = cards();
        screen.np_glow = glow;
        let mut port = Port::new(vec![idle(), screen.clone(), screen, BattleState::default()]);
        assert!(port.driver().attack_np(3, true).unwrap());
        assert!(port.taps.contains(&Point { x: 845, y: 203 }));
        assert!(!port.taps.contains(&Point { x: 431, y: 203 }));
        assert!(!port.taps.contains(&Point { x: 651, y: 203 }));
        assert_eq!(port.taps.len(), 4);
    }
}

#[test]
fn unavailable_np_returns_to_skills_without_committing_attack() {
    for required in [false, true] {
        let mut screen = cards();
        screen.np_blocked[2] = true;
        let mut port = Port::new(vec![idle(), screen.clone(), screen, idle()]);
        let result = port.driver().attack_np(3, required);
        if required {
            assert!(result.is_err());
        } else {
            assert!(!result.unwrap());
        }
        assert_eq!(
            port.taps,
            vec![Point { x: 1130, y: 600 }, Point { x: 1210, y: 683 }]
        );
        assert!(!port.commands.contains(&Command::SelectCards));
    }
}

#[test]
fn other_np_branch_selects_both_ready_allies_but_excludes_support() {
    let mut port = Port::new(vec![idle(), cards(), cards(), BattleState::default()]);
    assert!(port.driver().attack_other_nps().unwrap());
    assert!(port.taps.contains(&Point { x: 431, y: 203 }));
    assert!(port.taps.contains(&Point { x: 651, y: 203 }));
    assert!(!port.taps.contains(&Point { x: 845, y: 203 }));
}

#[test]
fn highest_hp_target_is_selected_and_verified_before_attack() {
    let enemies = std::array::from_fn(|index| {
        Some(EnemyState {
            slot: EnemySlot::try_from(index + 1).unwrap(),
            hp: if index == 5 { 3_152_503 } else { 735_728 },
            charge: None,
        })
    });
    let mut before = idle();
    before.enemy_layout = Some(EnemyLayout::Six);
    before.enemies = enemies;
    before.selected_enemy = Some(EnemySlot::One);
    let mut selected = before.clone();
    selected.selected_enemy = Some(EnemySlot::Six);
    let mut port = Port::new(vec![
        before,
        selected,
        cards(),
        cards(),
        BattleState::default(),
    ]);
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;
    port.driver().attack(true).unwrap();
    assert_eq!(
        port.commands,
        vec![
            Command::SelectEnemy {
                slot: EnemySlot::Six
            },
            Command::Attack { opening: true },
            Command::SelectCards
        ]
    );
    assert_eq!(
        port.taps,
        vec![
            Point { x: 545, y: 142 },
            Point { x: 1130, y: 600 },
            Point { x: 431, y: 203 },
            Point { x: 651, y: 203 },
            Point { x: 845, y: 203 }
        ]
    );
}

#[test]
fn highest_hp_target_does_not_touch_an_already_selected_enemy() {
    let mut before = idle();
    before.enemy_layout = Some(EnemyLayout::Six);
    before.enemies = std::array::from_fn(|index| {
        Some(EnemyState {
            slot: EnemySlot::try_from(index + 1).unwrap(),
            hp: if index == 5 { 3_152_503 } else { 735_728 },
            charge: None,
        })
    });
    before.selected_enemy = Some(EnemySlot::Six);
    let mut port = Port::new(vec![before, cards(), cards(), BattleState::default()]);
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;
    port.driver().attack(true).unwrap();
    assert_eq!(
        port.commands,
        vec![Command::Attack { opening: true }, Command::SelectCards]
    );
    assert_eq!(port.taps[0], Point { x: 1130, y: 600 });
}

#[test]
fn highest_hp_target_accepts_a_complete_partially_empty_six_enemy_layout() {
    let mut before = idle();
    before.enemy_layout = Some(EnemyLayout::Six);
    before.enemies[0] = Some(EnemyState {
        slot: EnemySlot::One,
        hp: 145_196,
        charge: None,
    });
    before.selected_enemy = Some(EnemySlot::One);
    let mut port = Port::new(vec![before, cards(), cards(), BattleState::default()]);
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;

    port.driver().attack(true).unwrap();

    assert_eq!(
        port.commands,
        vec![Command::Attack { opening: true }, Command::SelectCards]
    );
    assert_eq!(port.taps[0], Point { x: 1130, y: 600 });
}

#[test]
fn highest_hp_target_uses_standard_layout_slots() {
    let mut before = idle();
    before.enemy_layout = Some(EnemyLayout::Standard);
    for index in 0..3 {
        before.enemies[index] = Some(EnemyState {
            slot: EnemySlot::try_from(index + 1).unwrap(),
            hp: 100_000 * (index as u32 + 1),
            charge: None,
        });
    }
    before.selected_enemy = Some(EnemySlot::One);
    let mut selected = before.clone();
    selected.selected_enemy = Some(EnemySlot::Three);
    let mut port = Port::new(vec![
        before,
        selected,
        cards(),
        cards(),
        BattleState::default(),
    ]);
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;

    port.driver().attack(true).unwrap();

    assert_eq!(
        port.commands.first(),
        Some(&Command::SelectEnemy {
            slot: EnemySlot::Three
        })
    );
    assert_eq!(port.taps[0], Point { x: 542, y: 40 });
}

#[test]
fn highest_hp_target_stops_before_input_when_observation_is_incomplete() {
    let mut port = Port::new(vec![idle()]);
    port.profile.enemy_target = EnemyTargetStrategy::HighestHp;
    let error = port.driver().attack(true).unwrap_err().to_string();
    assert!(error.contains("complete supported enemy observation"));
    assert!(port.taps.is_empty());
    assert!(port.commands.is_empty());
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
    assert_eq!(rank_cards(&s, [false; 3]).unwrap(), [1, 2, 4]);
    s.sealed = [true, true, true, true, false];
    assert!(rank_cards(&s, [false; 3]).is_err());
}

#[test]
fn profile_can_infer_unresolved_face_cards_as_support_cards() {
    let mut state = cards();
    state.colors = [
        Some(Color::Arts),
        Some(Color::Buster),
        Some(Color::Buster),
        Some(Color::Quick),
        Some(Color::Buster),
    ];
    state.card_owners = [
        None,
        Some(PartySlot::Five),
        Some(PartySlot::Two),
        Some(PartySlot::Five),
        None,
    ];
    state.support_cards = [false; 5];
    let frontline = [
        Some(PartySlot::Five),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ];
    let strategy = CardStrategyProfile {
        infer_unknown_support_cards: true,
        ..CardStrategyProfile::default()
    };

    assert_eq!(
        rank_cards_for_party(&state, [false, false, true], frontline, &strategy).unwrap(),
        [8, 1, 5]
    );
}

#[test]
fn follow_up_cards_prefer_a_same_servant_brave_chain() {
    let mut s = cards();
    s.card_groups = [0, 1, 0, 0, 2].map(CardGroup::from_id);

    assert_eq!(rank_cards(&s, [false; 3]).unwrap(), [1, 3, 4]);
}

#[test]
fn np_order_uses_original_party_identity_after_order_change() {
    let mut port = Port::new(vec![idle(), cards(), cards(), BattleState::default()]);
    port.frontline = Some([
        Some(PartySlot::Five),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ]);
    port.profile.cards.noble_phantasm_order = Some([
        PartySlot::Three,
        PartySlot::Five,
        PartySlot::Two,
        PartySlot::One,
        PartySlot::Four,
        PartySlot::Six,
    ]);

    port.driver().attack(true).unwrap();

    assert_eq!(
        &port.taps[1..4],
        &[
            Point { x: 845, y: 203 },
            Point { x: 431, y: 203 },
            Point { x: 651, y: 203 },
        ]
    );
    assert!(
        port.logs
            .iter()
            .any(|message| message.contains("[8, 6, 7]"))
    );
}

#[test]
fn profile_changes_face_card_priority_and_order_score() {
    let mut state = cards();
    state.colors = [
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Buster),
        Some(Color::Buster),
    ];
    state.card_groups = [0, 1, 2, 3, 4].map(CardGroup::from_id);
    let frontline = [
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ];
    let mut strategy = CardStrategyProfile::default();

    assert_eq!(
        rank_cards_for_party(&state, [false; 3], frontline, &strategy).unwrap(),
        [1, 2, 3]
    );

    strategy.face_card_priority = [
        FaceCardPriority::Damage,
        FaceCardPriority::SupportCount,
        FaceCardPriority::BraveChain,
        FaceCardPriority::ColorChain,
    ];
    assert_eq!(
        rank_cards_for_party(&state, [false; 3], frontline, &strategy).unwrap(),
        [4, 1, 5]
    );
}

#[test]
fn departed_or_swapped_out_owners_cannot_create_a_false_brave_chain() {
    let mut state = cards();
    state.colors = [
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Buster),
        Some(Color::Buster),
    ];
    state.support_cards = [false; 5];
    state.card_groups = [0, 1, 2, 3, 4].map(CardGroup::from_id);
    state.card_owners = [None; 5];
    let strategy = CardStrategyProfile {
        face_card_priority: [
            FaceCardPriority::BraveChain,
            FaceCardPriority::Damage,
            FaceCardPriority::ColorChain,
            FaceCardPriority::SupportCount,
        ],
        ..CardStrategyProfile::default()
    };
    // Slot one has left the field; a reserve member occupies its position.
    for replacement in [PartySlot::Four, PartySlot::Five] {
        let frontline = [
            Some(replacement),
            Some(PartySlot::Two),
            Some(PartySlot::Three),
        ];
        let fallback = rank_cards_for_party(&state, [false; 3], frontline, &strategy).unwrap();
        assert!(fallback.contains(&4) && fallback.contains(&5));
        let mut stale = state.clone();
        stale.card_owners[..3].fill(Some(PartySlot::One));
        assert_eq!(
            rank_cards_for_party(&stale, [false; 3], frontline, &strategy).unwrap(),
            fallback
        );
        // Confirmed current identities must still contribute a Brave Chain.
        stale.card_owners[..3].fill(Some(replacement));
        let selected = rank_cards_for_party(&stale, [false; 3], frontline, &strategy).unwrap();
        assert!(selected.iter().all(|card| *card <= 3));
    }
}

#[test]
fn mixed_np_chains_use_atlas_color_and_party_identity() {
    let mut state = cards();
    state.colors = [
        Some(Color::Buster),
        Some(Color::Quick),
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Buster),
    ];
    state.card_owners = [
        Some(PartySlot::One),
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ];
    let frontline = [
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
    ];

    assert_eq!(
        rank_cards_with_party_data(
            &state,
            [true, false, false],
            frontline,
            [Some(Color::Arts), None, None],
            &CardStrategyProfile::default(),
        )
        .unwrap(),
        [6, 2, 1],
        "NP and two face cards from slot one form a Brave Chain"
    );

    state.colors = [
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Buster),
        Some(Color::Buster),
        Some(Color::Quick),
    ];
    state.card_owners = [
        Some(PartySlot::Two),
        Some(PartySlot::Three),
        Some(PartySlot::Two),
        Some(PartySlot::Three),
        Some(PartySlot::Two),
    ];
    assert_eq!(
        rank_cards_with_party_data(
            &state,
            [true, false, false],
            frontline,
            [Some(Color::Arts), None, None],
            &CardStrategyProfile::default(),
        )
        .unwrap(),
        [6, 1, 2],
        "Atlas NP color completes an Arts chain"
    );
}

#[test]
fn attack_uses_the_observed_dynamic_support_np_color() {
    let mut command_cards = cards();
    command_cards.np_glow = [false, false, true];
    command_cards.colors = [
        Some(Color::Buster),
        Some(Color::Buster),
        Some(Color::Arts),
        Some(Color::Arts),
        Some(Color::Quick),
    ];
    command_cards.card_owners = [
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::One),
        Some(PartySlot::Two),
        Some(PartySlot::One),
    ];
    command_cards.support_atlas_id = Some(704000);
    let mut port = Port::new(vec![
        idle(),
        command_cards.clone(),
        command_cards,
        BattleState::default(),
    ]);
    port.profile.cards.color_score = [200, 100, 10];
    port.atlas_np_colors.insert(704000, Color::Buster);

    port.driver().attack(false).unwrap();

    assert!(
        port.logs
            .iter()
            .any(|message| message.contains("[8, 1, 2]")),
        "a confirmed Buster support NP should complete the Buster chain: {:?}",
        port.logs
    );
}

#[test]
fn support_np_color_does_not_reuse_a_rejected_previous_frame_identity() {
    for (previous, current, expected) in [
        (Some(704000), None, "[8, 3, 4]"),
        (Some(704000), Some(704100), "[8, 3, 4]"),
        (None, Some(704000), "[8, 1, 2]"),
    ] {
        let mut first = cards();
        first.np_glow = [false, false, true];
        first.colors = [
            Some(Color::Buster),
            Some(Color::Buster),
            Some(Color::Arts),
            Some(Color::Arts),
            Some(Color::Quick),
        ];
        first.support_atlas_id = previous;
        let mut latest = first.clone();
        latest.support_atlas_id = current;
        let mut port = Port::new(vec![idle(), first, latest, BattleState::default()]);
        port.profile.cards.color_score = [200, 100, 10];
        port.atlas_np_colors.insert(704000, Color::Buster);
        port.atlas_np_colors.insert(704100, Color::Buster);
        port.driver().attack(false).unwrap();
        assert!(
            port.logs.iter().any(|line| line.contains(expected)),
            "{previous:?} -> {current:?}: {:?}",
            port.logs
        );
        if previous.is_some() {
            assert!(
                port.logs
                    .iter()
                    .any(|line| line.contains("discard previous-frame identity"))
            );
        }
    }
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

fn skill_rule() -> fgo_rs::profile::ServantSkillRule {
    fgo_rs::profile::ServantSkillRule {
        atlas_skill_id: None,
        servant: PartySlot::Five,
        skill: SkillSlot::One,
        target: PartySlot::Two.into(),
        hp_below: None,
        np_below: None,
        enemy_charge_full: false,
        any_of: Vec::new(),
        earliest: None,
    }
}

#[test]
fn profile_skill_resolves_swapped_identity_and_verifies_explicit_target() {
    let popup = BattleState {
        target_count: 3,
        ..Default::default()
    };
    let mut done = idle();
    done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![idle(), idle(), popup, done]);
    port.profile.skill_rules = vec![skill_rule()];
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
    assert_eq!(
        port.commands,
        vec![Command::CastServantSkill {
            servant: FieldSlot::One,
            skill: SkillSlot::One,
            target: fgo_rs::domain::Target::Ally {
                slot: FieldSlot::Two
            },
            strict: false,
        }]
    );
    assert_eq!(
        port.taps[1],
        fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[1]
    );
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 0);
    assert_eq!(port.commands.len(), 1);
}

#[test]
fn profile_skill_selects_the_lowest_np_frontline_member() {
    let mut measured = idle();
    measured.np_percent = [80u16.into(), 20u16.into(), 60u16.into()];
    let mut popup = measured.clone();
    popup.target_count = 3;
    let mut done = measured.clone();
    done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![idle(), measured, popup, done]);
    let mut rule = skill_rule();
    rule.target = fgo_rs::profile::SkillRuleTarget::LowestNp {
        lowest_np: fgo_rs::profile::LowestNpTarget { below: 100 },
    };
    port.profile.skill_rules = vec![rule];

    assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
    assert!(matches!(
        port.commands.first(),
        Some(Command::CastServantSkill {
            target: fgo_rs::domain::Target::Ally {
                slot: FieldSlot::Two
            },
            ..
        })
    ));
    assert_eq!(
        port.taps[1],
        fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[1]
    );
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::NP))
    );
}

#[test]
fn profile_dynamic_target_stops_before_input_when_a_frontline_value_is_unknown() {
    let mut measured = idle();
    measured.np_percent = [80u16.into(), Default::default(), 60u16.into()];
    let mut port = Port::new(vec![idle(), measured]);
    let mut rule = skill_rule();
    rule.target = fgo_rs::profile::SkillRuleTarget::LowestNp {
        lowest_np: fgo_rs::profile::LowestNpTarget { below: 100 },
    };
    port.profile.skill_rules = vec![rule];

    let error = port.driver().cast_profile_skills().unwrap_err();
    assert!(error.to_string().contains("confirmed NP"));
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());
}

#[test]
fn profile_master_skill_selects_lowest_hp_and_skips_at_the_threshold() {
    let mut measured = master();
    measured.ally_hp = [10_000u32.into(), 5_000u32.into(), 7_000u32.into()];
    let mut popup = measured.clone();
    popup.target_count = 3;
    let mut done = measured.clone();
    done.master_skills[0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![master(), measured, popup, done.clone(), done, idle()]);
    let mut rule = master_rule(SkillSlot::One);
    rule.target = fgo_rs::profile::MasterSkillTarget::LowestHp {
        lowest_hp: fgo_rs::profile::LowestHpTarget { below: 6_000 },
    };
    port.profile.master_skill_rules = vec![rule];
    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 1);
    assert!(matches!(
        port.commands.first(),
        Some(Command::CastMasterSkill {
            target: fgo_rs::domain::Target::Ally {
                slot: FieldSlot::Two
            },
            ..
        })
    ));

    let mut at_threshold = master();
    at_threshold.ally_hp = [10_000u32.into(), 6_000u32.into(), 7_000u32.into()];
    let mut port = Port::new(vec![master(), at_threshold, idle()]);
    let mut rule = master_rule(SkillSlot::One);
    rule.target = fgo_rs::profile::MasterSkillTarget::LowestHp {
        lowest_hp: fgo_rs::profile::LowestHpTarget { below: 6_000 },
    };
    port.profile.master_skill_rules = vec![rule];
    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 0);
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());
}

fn with_enemy_charge(mut state: BattleState, charge: Option<EnemyCharge>) -> BattleState {
    state.enemy_layout = Some(EnemyLayout::Standard);
    state.enemies[0] = Some(EnemyState {
        slot: EnemySlot::One,
        hp: 10_000,
        charge,
    });
    state
}

#[test]
fn profile_skill_casts_only_after_confirming_a_full_enemy_charge() {
    let mut full = with_enemy_charge(
        idle(),
        Some(EnemyCharge {
            current: 3,
            total: 3,
        }),
    );
    full.np_percent = [80u16.into(), 80u16.into(), 80u16.into()];
    let mut done = idle();
    done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![full.clone(), full, done]);
    let mut rule = skill_rule();
    rule.enemy_charge_full = true;
    port.profile.skill_rules = vec![rule];

    assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
    assert_eq!(port.commands.len(), 1);
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::ENEMIES))
    );
}

#[test]
fn profile_skill_any_of_conditions_casts_on_one_confirmed_branch() {
    let mut full = with_enemy_charge(
        idle(),
        Some(EnemyCharge {
            current: 3,
            total: 3,
        }),
    );
    full.np_percent = [80u16.into(), 80u16.into(), 80u16.into()];
    let mut done = full.clone();
    done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![full.clone(), full, done]);
    let mut rule = skill_rule();
    rule.any_of = vec![
        SkillRuleConditionAlternative {
            np_below: Some(NpBelowCondition {
                servant: PartySlot::Five,
                percent: 50,
            }),
            ..Default::default()
        },
        SkillRuleConditionAlternative {
            enemy_charge_full: true,
            ..Default::default()
        },
    ];
    port.profile.skill_rules = vec![rule];

    assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
    assert!(
        port.commands
            .iter()
            .any(|command| matches!(command, Command::CastServantSkill { .. }))
    );
    assert!(port.requested_facts.iter().any(|facts| {
        facts.contains(RequiredBattleFacts::NP) && facts.contains(RequiredBattleFacts::ENEMIES)
    }));
}

#[test]
fn profile_skill_any_of_stops_when_an_alternative_fact_is_unknown() {
    let mut state = with_enemy_charge(
        idle(),
        Some(EnemyCharge {
            current: 3,
            total: 3,
        }),
    );
    state.np_percent[0] = Default::default();
    let mut port = Port::new(vec![state.clone(), state]);
    let mut rule = skill_rule();
    rule.any_of = vec![
        SkillRuleConditionAlternative {
            np_below: Some(NpBelowCondition {
                servant: PartySlot::Five,
                percent: 50,
            }),
            ..Default::default()
        },
        SkillRuleConditionAlternative {
            enemy_charge_full: true,
            ..Default::default()
        },
    ];
    port.profile.skill_rules = vec![rule];

    let error = port.driver().cast_profile_skills().unwrap_err();
    assert!(error.to_string().contains("percentage is unknown"));
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());
}

#[test]
fn profile_skill_skips_known_non_full_charge_and_stops_on_unknown_charge() {
    let non_full = with_enemy_charge(
        idle(),
        Some(EnemyCharge {
            current: 2,
            total: 3,
        }),
    );
    let mut port = Port::new(vec![non_full.clone(), non_full]);
    let mut rule = skill_rule();
    rule.enemy_charge_full = true;
    port.profile.skill_rules = vec![rule.clone()];
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 0);
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());

    let unknown = with_enemy_charge(idle(), None);
    let mut port = Port::new(vec![unknown.clone(), unknown]);
    port.profile.skill_rules = vec![rule];
    let error = port.driver().cast_profile_skills().unwrap_err();
    assert!(error.to_string().contains("unknown gauge"));
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());
}

#[test]
fn profile_skill_timing_is_lexicographic_and_clamped_to_last_wave() {
    for (wave, turn, should_cast) in [(1, 3, false), (2, 1, false), (2, 2, true), (2, 3, true)] {
        let mut done = idle();
        done.skills[0][0] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![idle(), idle(), done]);
        let mut rule = skill_rule();
        rule.earliest = Some(fgo_rs::profile::SkillTiming { wave: 3, turn: 2 });
        port.profile.skill_rules = vec![rule];
        port.progress = Some(fgo_rs::battle::BattleProgress {
            wave: Some(fgo_rs::battle::BattleWave::new(wave, 2).unwrap()),
            wave_turn: turn,
            ..Default::default()
        });
        assert_eq!(
            port.driver().cast_profile_skills().unwrap(),
            usize::from(should_cast)
        );
        assert_eq!(port.taps.len(), usize::from(should_cast));
    }
}

#[test]
fn profile_skill_preflight_rejects_unknown_timing_or_missing_target_before_casts() {
    for unknown_timing in [true, false] {
        let mut port = Port::new(vec![idle()]);
        let mut rule = skill_rule();
        rule.skill = SkillSlot::Two;
        if unknown_timing {
            rule.earliest = Some(fgo_rs::profile::SkillTiming { wave: 1, turn: 1 });
            port.progress = Some(Default::default());
        } else {
            rule.target = PartySlot::Six.into();
        }
        port.profile.skill_rules = vec![skill_rule(), rule];
        assert!(port.driver().cast_profile_skills().is_err());
        assert!(port.taps.is_empty());
        assert!(port.commands.is_empty());
    }
}

#[test]
fn profile_skills_preserve_priority_skip_reserves_and_reject_unknown_readiness() {
    let mut first_done = idle();
    first_done.skills[1][0] = SkillState::Cooldown.into();
    let mut all_done = first_done.clone();
    all_done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![
        idle(),
        idle(),
        first_done.clone(),
        first_done,
        all_done,
    ]);
    let mut first = skill_rule();
    first.servant = PartySlot::Two;
    let mut reserve = skill_rule();
    reserve.servant = PartySlot::Six;
    port.profile.skill_rules = vec![reserve, first, skill_rule()];
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 2);
    assert_eq!(
        port.taps,
        vec![
            fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[1][0],
            fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[0][0],
        ]
    );
    let mut unknown = idle();
    unknown.skills[0][0] = Default::default();
    let mut port = Port::new(vec![unknown]);
    port.profile.skill_rules = vec![skill_rule()];
    assert!(port.driver().cast_profile_skills().is_err());
    assert!(port.taps.is_empty());
}

fn master_rule(skill: SkillSlot) -> fgo_rs::profile::MasterSkillRule {
    fgo_rs::profile::MasterSkillRule {
        skill,
        target: PartySlot::Five.into(),
        hp_below: None,
        np_below: None,
        enemy_charge_full: false,
        any_of: Vec::new(),
        earliest: None,
    }
}

#[test]
fn profile_master_skills_resolve_targets_preserve_order_and_verify_cooldown() {
    let mut cooling_two = master();
    cooling_two.master_skills[1] = SkillState::Cooldown.into();
    let mut cooling_both = cooling_two.clone();
    cooling_both.master_skills[0] = SkillState::Cooldown.into();
    let popup = BattleState {
        target_count: 3,
        ..Default::default()
    };
    let mut port = Port::new(vec![
        master(),
        master(),
        popup,
        cooling_two.clone(),
        cooling_two.clone(),
        cooling_two.clone(),
        idle(),
        cooling_two,
        cooling_both.clone(),
        cooling_both.clone(),
        cooling_both,
        idle(),
    ]);
    port.profile.master_skill_rules =
        vec![master_rule(SkillSlot::Two), master_rule(SkillSlot::One)];
    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 2);
    assert_eq!(
        port.commands,
        vec![
            Command::CastMasterSkill {
                skill: SkillSlot::Two,
                target: fgo_rs::domain::Target::Ally {
                    slot: FieldSlot::One
                },
                strict: false
            },
            Command::CastMasterSkill {
                skill: SkillSlot::One,
                target: fgo_rs::domain::Target::Ally {
                    slot: FieldSlot::One
                },
                strict: false
            },
        ]
    );
    assert_eq!(
        port.taps[1],
        fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[0]
    );
}

#[test]
fn profile_master_skill_reuses_confirmed_enemy_charge_condition() {
    let full = with_enemy_charge(
        master(),
        Some(EnemyCharge {
            current: 5,
            total: 5,
        }),
    );
    let mut cooling = master();
    cooling.master_skills[0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![
        full.clone(),
        full,
        cooling.clone(),
        cooling.clone(),
        cooling,
        idle(),
    ]);
    let mut rule = master_rule(SkillSlot::One);
    rule.enemy_charge_full = true;
    port.profile.master_skill_rules = vec![rule];

    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 1);
    assert!(matches!(
        port.commands.first(),
        Some(Command::CastMasterSkill {
            skill: SkillSlot::One,
            ..
        })
    ));
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::ENEMIES))
    );
}

#[test]
fn master_rules_skip_cooldown_and_reject_unknown_readiness_without_casting() {
    for readiness in [Some(SkillState::Cooldown), Some(SkillState::Disabled), None] {
        let mut state = master();
        state.master_skills[0] = readiness.map(Into::into).unwrap_or_default();
        let mut port = Port::new(vec![state.clone(), state.clone(), state, idle()]);
        port.profile.master_skill_rules = vec![master_rule(SkillSlot::One)];
        let result = port.driver().cast_profile_master_skills();
        if readiness.is_some() {
            assert_eq!(result.unwrap(), 0);
        } else {
            assert!(result.unwrap_err().to_string().contains("unknown"));
        }
        assert!(port.commands.is_empty());
        assert_eq!(
            port.taps,
            vec![fgo_rs::layout::UI_LAYOUT_V1.battle.master_toggle]
        );
    }
}

#[test]
fn master_rule_preflight_stops_unknown_timing_missing_targets_and_order_change() {
    for kind in 0..3 {
        let mut port = Port::new(vec![idle()]);
        let mut rule = master_rule(SkillSlot::Two);
        match kind {
            0 => rule.earliest = Some(fgo_rs::profile::SkillTiming { wave: 1, turn: 1 }),
            1 => rule.target = PartySlot::Six.into(),
            _ => rule.skill = SkillSlot::Three,
        }
        port.profile.master_skill_rules = vec![master_rule(SkillSlot::One), rule];
        assert!(port.driver().cast_profile_master_skills().is_err());
        assert!(port.taps.is_empty());
        assert!(port.commands.is_empty());
    }
}

#[test]
fn master_rule_waits_for_timing_and_does_not_retry_failed_casts() {
    let mut port = Port::new(vec![master()]);
    let mut rule = master_rule(SkillSlot::One);
    rule.earliest = Some(fgo_rs::profile::SkillTiming { wave: 3, turn: 2 });
    port.profile.master_skill_rules = vec![rule];
    port.progress = Some(fgo_rs::battle::BattleProgress {
        wave: Some(fgo_rs::battle::BattleWave::new(2, 2).unwrap()),
        wave_turn: 1,
        ..Default::default()
    });
    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 0);
    assert!(port.taps.is_empty());
    port.progress.as_mut().unwrap().wave_turn = 2;
    assert!(
        port.driver()
            .cast_profile_master_skills()
            .unwrap_err()
            .to_string()
            .contains("did not enter cooldown")
    );
    assert_eq!(port.commands.len(), 1);
    assert_eq!(
        port.taps,
        vec![fgo_rs::layout::UI_LAYOUT_V1.battle.master_skills[0]]
    );
}

#[test]
fn ordinary_third_master_skill_is_verified_and_swap_is_disabled() {
    let mut cooling = master();
    cooling.master_skills[2] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![
        master(),
        master(),
        cooling.clone(),
        cooling.clone(),
        cooling,
        idle(),
    ]);
    port.profile.order_change = None;
    port.profile.master_skill_rules = vec![master_rule(SkillSlot::Three)];
    port.profile.validate().unwrap();
    assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 1);
    assert!(matches!(
        port.commands[0],
        Command::CastMasterSkill {
            skill: SkillSlot::Three,
            ..
        }
    ));
    assert_eq!(
        port.taps[0],
        fgo_rs::layout::UI_LAYOUT_V1.battle.master_skills[2]
    );
    let taps = port.taps.len();
    assert!(port.driver().order_change().is_err());
    assert_eq!(port.taps.len(), taps);
}

#[test]
fn configured_swap_slot_is_rejected_before_input_in_ordinary_skill_api() {
    let mut port = Port::new(vec![master()]);
    port.profile.order_change.as_mut().unwrap().master_skill = SkillSlot::Two;
    assert!(port.driver().cast_master_skill(2, true).is_err());
    assert!(port.driver().cast_master_skill(0, true).is_err());
    assert!(port.driver().cast_master_skill(4, true).is_err());
    assert!(port.taps.is_empty());
}

#[test]
fn unexpected_swap_dialog_stops_without_selecting_members() {
    let dialog = BattleState {
        order_change: true,
        ..Default::default()
    };
    let mut port = Port::new(vec![master(), dialog]);
    port.profile.order_change = None;
    assert!(
        port.driver()
            .cast_master_skill(3, true)
            .unwrap_err()
            .to_string()
            .contains("unexpected order-change")
    );
    assert_eq!(
        port.taps,
        vec![fgo_rs::layout::UI_LAYOUT_V1.battle.master_skills[2]]
    );
}

fn enemy_turn(layout: EnemyLayout, selected: EnemySlot) -> BattleState {
    let mut state = idle();
    state.enemy_layout = Some(layout);
    state.selected_enemy = Some(selected);
    let count = if layout == EnemyLayout::Standard {
        3
    } else {
        6
    };
    for i in 0..count {
        state.enemies[i] = Some(EnemyState {
            slot: EnemySlot::try_from(i + 1).unwrap(),
            hp: 100,
            charge: None,
        });
    }
    state
}

#[test]
fn enemy_master_rule_verifies_selection_before_skill_and_cooldown_afterwards() {
    for (layout, target) in [
        (EnemyLayout::Standard, EnemySlot::Three),
        (EnemyLayout::Six, EnemySlot::Six),
    ] {
        let before = enemy_turn(layout, EnemySlot::One);
        let selected = enemy_turn(layout, target);
        let mut open = selected.clone();
        open.master_open = true;
        open.master_skills = [SkillState::Available.into(); 3];
        let mut cooling = open.clone();
        cooling.master_skills[1] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![
            master(),
            master(),
            master(),
            before,
            selected.clone(),
            selected.clone(),
            open,
            cooling.clone(),
            cooling.clone(),
            cooling,
            selected,
        ]);
        port.profile.master_skill_rules = vec![fgo_rs::profile::MasterSkillRule {
            skill: SkillSlot::Two,
            target: fgo_rs::profile::MasterSkillTarget::Enemy { enemy: target },
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        }];
        assert_eq!(port.driver().cast_profile_master_skills().unwrap(), 1);
        assert_eq!(
            port.commands,
            vec![
                Command::SelectEnemy { slot: target },
                Command::CastMasterSkill {
                    skill: SkillSlot::Two,
                    target: fgo_rs::domain::Target::Enemy { slot: target },
                    strict: false,
                }
            ]
        );
        assert!(port.requested_facts.contains(&RequiredBattleFacts::ENEMIES));
    }
}

#[test]
fn enemy_master_skill_never_casts_without_a_verified_target() {
    for case in 0..3 {
        let mut before = enemy_turn(EnemyLayout::Standard, EnemySlot::One);
        if case == 0 {
            before.enemies[2] = None;
        }
        if case == 1 {
            before.selected_enemy = None;
        }
        let mut port = Port::new(vec![master(), master(), before]);
        assert!(
            port.driver()
                .cast_master_skill_on_enemy(2, EnemySlot::Three, false)
                .is_err()
        );
        assert!(
            !port
                .commands
                .iter()
                .any(|c| matches!(c, Command::CastMasterSkill { .. }))
        );
        assert!(
            !port
                .taps
                .contains(&fgo_rs::layout::UI_LAYOUT_V1.battle.master_skills[1])
        );
    }
}

#[test]
fn enemy_master_skill_rechecks_after_opening_panel_and_rejects_ally_dialog() {
    for dialog in [false, true] {
        let selected = enemy_turn(EnemyLayout::Standard, EnemySlot::Three);
        let mut open = selected.clone();
        open.master_open = true;
        open.master_skills = [SkillState::Available.into(); 3];
        if !dialog {
            open.selected_enemy = Some(EnemySlot::One);
        }
        let mut port = Port::new(vec![
            master(),
            master(),
            selected.clone(),
            selected,
            open,
            BattleState {
                target_count: 3,
                ..Default::default()
            },
        ]);
        assert!(
            port.driver()
                .cast_master_skill_on_enemy(2, EnemySlot::Three, false)
                .is_err()
        );
        assert_eq!(
            port.commands
                .iter()
                .filter(|c| matches!(c, Command::CastMasterSkill { .. }))
                .count(),
            usize::from(dialog)
        );
        for target in fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets {
            assert!(!port.taps.contains(&target));
        }
    }
}

#[test]
fn servant_enemy_rule_resolves_caster_and_confirms_selection_before_casting() {
    for (layout, target) in [
        (EnemyLayout::Standard, EnemySlot::Three),
        (EnemyLayout::Six, EnemySlot::Six),
    ] {
        let before = enemy_turn(layout, EnemySlot::One);
        let selected = enemy_turn(layout, target);
        let mut done = selected.clone();
        done.skills[0][0] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![
            before.clone(),
            before,
            selected.clone(),
            selected,
            done,
        ]);
        let mut rule = skill_rule();
        rule.target = fgo_rs::profile::SkillRuleTarget::Enemy { enemy: target };
        port.profile.skill_rules = vec![rule];
        assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
        assert_eq!(
            port.commands,
            vec![
                Command::SelectEnemy { slot: target },
                Command::CastServantSkill {
                    servant: FieldSlot::One,
                    skill: SkillSlot::One,
                    target: fgo_rs::domain::Target::Enemy { slot: target },
                    strict: false,
                }
            ]
        );
    }
}

#[test]
fn servant_enemy_skill_rejects_missing_target_drift_and_ally_popup() {
    for case in 0..3 {
        let mut selected = enemy_turn(EnemyLayout::Standard, EnemySlot::Three);
        if case == 0 {
            selected.enemies[2] = None;
        }
        let mut confirmed = selected.clone();
        if case == 1 {
            confirmed.selected_enemy = Some(EnemySlot::One);
        }
        let mut port = Port::new(vec![
            selected,
            confirmed,
            BattleState {
                target_count: 3,
                ..Default::default()
            },
        ]);
        assert!(
            port.driver()
                .cast_skill_on_enemy(1, 1, EnemySlot::Three, false)
                .is_err()
        );
        assert_eq!(
            port.commands
                .iter()
                .filter(|c| matches!(c, Command::CastServantSkill { .. }))
                .count(),
            usize::from(case == 2)
        );
        for point in fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets {
            assert!(!port.taps.contains(&point));
        }
    }
}

#[test]
fn unavailable_servant_enemy_skill_does_not_select_a_target() {
    for ready in [Some(SkillState::Cooldown), Some(SkillState::Disabled), None] {
        let mut state = enemy_turn(EnemyLayout::Standard, EnemySlot::One);
        state.skills[0][0] = ready.map(Into::into).unwrap_or_default();
        let mut port = Port::new(vec![state]);
        let result = port
            .driver()
            .cast_skill_on_enemy(1, 1, EnemySlot::Three, false);
        if ready.is_some() {
            assert!(!result.unwrap());
        } else {
            assert!(result.is_err());
        }
        assert!(port.taps.is_empty());
        assert!(port.commands.is_empty());
    }
}

fn np_condition(servant: PartySlot) -> fgo_rs::profile::NpBelowCondition {
    fgo_rs::profile::NpBelowCondition {
        servant,
        percent: 100,
    }
}

#[test]
fn np_skill_condition_uses_swapped_identity_and_strict_threshold() {
    for value in [0, 99, 100, 148, 300] {
        let mut ready = idle();
        ready.np_percent = [value.into(), 300.into(), 300.into()];
        let mut done = ready.clone();
        done.skills[0][0] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![ready.clone(), ready, done]);
        let mut rule = skill_rule();
        rule.np_below = Some(np_condition(PartySlot::Five));
        port.profile.skill_rules = vec![rule];
        assert_eq!(
            port.driver().cast_profile_skills().unwrap(),
            usize::from(value < 100)
        );
        assert_eq!(port.commands.len(), usize::from(value < 100));
        assert!(
            port.requested_facts
                .iter()
                .any(|facts| facts.contains(RequiredBattleFacts::NP))
        );
    }
}

#[test]
fn np_conditions_refresh_between_skills_and_skip_after_charge() {
    let mut before = idle();
    before.np_percent[2] = 70.into();
    let mut after = idle();
    after.skills[0][0] = SkillState::Cooldown.into();
    after.np_percent[2] = 100.into();
    let mut port = Port::new(vec![before.clone(), before, after.clone(), after]);
    let mut first = skill_rule();
    first.np_below = Some(np_condition(PartySlot::Three));
    let mut second = first.clone();
    second.skill = SkillSlot::Two;
    port.profile.skill_rules = vec![first, second];
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 1);
    assert_eq!(
        port.taps,
        vec![fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[0][0]]
    );
    assert_eq!(
        port.requested_facts
            .iter()
            .filter(|facts| facts.contains(RequiredBattleFacts::NP))
            .count(),
        2
    );
}

#[test]
fn np_condition_unknown_missing_member_or_out_of_range_stops_before_skill_input() {
    for (servant, reading) in [
        (PartySlot::Five, Recognition::NotRequested),
        (
            PartySlot::Five,
            Recognition::unknown(UnknownReason::Ambiguous, None),
        ),
        (PartySlot::Five, 301.into()),
        (PartySlot::Six, 70.into()),
    ] {
        let mut state = idle();
        state.np_percent[0] = reading;
        let mut port = Port::new(vec![state]);
        let mut rule = skill_rule();
        rule.np_below = Some(np_condition(servant));
        port.profile.skill_rules = vec![rule];
        assert!(
            port.driver()
                .cast_profile_skills()
                .unwrap_err()
                .to_string()
                .contains("NP")
        );
        assert!(port.commands.is_empty());
        assert!(port.taps.is_empty());
    }
    // A cooling skill requires no NP decision and must remain skippable.
    let mut state = idle();
    state.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![state]);
    let mut rule = skill_rule();
    rule.np_below = Some(np_condition(PartySlot::Five));
    port.profile.skill_rules = vec![rule];
    assert_eq!(port.driver().cast_profile_skills().unwrap(), 0);
}

#[test]
fn np_master_condition_checks_the_current_open_panel_and_closes_on_skip() {
    for value in [0, 99, 100, 300] {
        let mut ready = master();
        ready.np_percent[0] = value.into();
        let mut done = ready.clone();
        done.master_skills[0] = SkillState::Cooldown.into();
        let frames = if value < 100 {
            vec![
                ready.clone(),
                ready,
                done.clone(),
                done.clone(),
                done,
                idle(),
            ]
        } else {
            vec![ready.clone(), ready.clone(), ready, idle()]
        };
        let mut port = Port::new(frames);
        let mut rule = master_rule(SkillSlot::One);
        rule.np_below = Some(np_condition(PartySlot::Five));
        port.profile.master_skill_rules = vec![rule];
        assert_eq!(
            port.driver().cast_profile_master_skills().unwrap(),
            usize::from(value < 100)
        );
        assert_eq!(port.commands.len(), usize::from(value < 100));
        assert!(!port.last.master_open);
        assert!(
            port.requested_facts
                .iter()
                .any(|facts| facts.contains(RequiredBattleFacts::NP))
        );
    }
    let mut port = Port::new(vec![master()]);
    let mut rule = master_rule(SkillSlot::One);
    rule.np_below = Some(np_condition(PartySlot::Five));
    port.profile.master_skill_rules = vec![rule];
    assert!(
        port.driver()
            .cast_profile_master_skills()
            .unwrap_err()
            .to_string()
            .contains("unknown")
    );
    assert!(port.commands.is_empty());
    assert!(port.taps.is_empty());
}

fn hp_condition(servant: PartySlot) -> fgo_rs::profile::HpBelowCondition {
    fgo_rs::profile::HpBelowCondition { servant, hp: 6600 }
}

#[test]
fn hp_conditions_use_swapped_identity_strict_threshold_and_current_health() {
    for hp in [0, 6599, 6600, 17793] {
        let mut before = idle();
        before.ally_hp[0] = hp.into();
        let mut healed = before.clone();
        healed.ally_hp[0] = 8000.into();
        healed.skills[0][0] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![before.clone(), before, healed.clone(), healed]);
        let mut first = skill_rule();
        first.hp_below = Some(hp_condition(PartySlot::Five));
        let mut second = first.clone();
        second.skill = SkillSlot::Two;
        port.profile.skill_rules = vec![first, second];
        assert_eq!(
            port.driver().cast_profile_skills().unwrap(),
            usize::from(hp < 6600)
        );
        assert_eq!(port.commands.len(), usize::from(hp < 6600));
        assert!(
            port.requested_facts
                .iter()
                .any(|f| f.contains(RequiredBattleFacts::HP))
        );
        assert!(
            !port
                .requested_facts
                .iter()
                .any(|f| f.contains(RequiredBattleFacts::NP))
        );
    }
}

#[test]
fn hp_unknown_or_missing_identity_blocks_cast_and_hp_np_conditions_are_conjunctive() {
    for absent in [false, true] {
        let mut port = Port::new(vec![idle()]);
        let mut rule = skill_rule();
        rule.hp_below = Some(hp_condition(if absent {
            PartySlot::Six
        } else {
            PartySlot::Five
        }));
        port.profile.skill_rules = vec![rule];
        assert!(
            port.driver()
                .cast_profile_skills()
                .unwrap_err()
                .to_string()
                .contains("HP")
        );
        assert!(port.taps.is_empty());
    }
    for (hp, np, cast) in [(5000, 70, true), (8000, 70, false), (5000, 100, false)] {
        let mut before = idle();
        before.ally_hp[0] = hp.into();
        before.np_percent[2] = np.into();
        let mut done = before.clone();
        done.skills[0][0] = SkillState::Cooldown.into();
        let mut port = Port::new(vec![before.clone(), before, done]);
        let mut rule = skill_rule();
        rule.hp_below = Some(hp_condition(PartySlot::Five));
        rule.np_below = Some(np_condition(PartySlot::Three));
        port.profile.skill_rules = vec![rule];
        assert_eq!(
            port.driver().cast_profile_skills().unwrap(),
            usize::from(cast)
        );
    }
}

#[test]
fn master_hp_condition_skips_at_threshold_and_stops_on_unknown() {
    for hp in [Some(6600), None] {
        let mut state = master();
        state.ally_hp[0] = hp.map(Into::into).unwrap_or_default();
        let mut port = Port::new(vec![state.clone(), state.clone(), state, idle()]);
        let mut rule = master_rule(SkillSlot::One);
        rule.hp_below = Some(hp_condition(PartySlot::Five));
        port.profile.master_skill_rules = vec![rule];
        let result = port.driver().cast_profile_master_skills();
        if hp.is_some() {
            assert_eq!(result.unwrap(), 0);
            assert!(!port.last.master_open);
        } else {
            assert!(result.unwrap_err().to_string().contains("HP"));
        }
        assert!(port.commands.is_empty());
    }
}

fn skill_confirmation() -> BattleState {
    BattleState {
        skill_confirmation: true,
        ..Default::default()
    }
}

#[test]
fn skill_confirmation_is_opt_in_and_only_valid_after_issuing_a_skill() {
    let mut port = Port::new(vec![idle(), skill_confirmation()]);
    assert!(
        port.driver()
            .cast_skill(1, 1, true)
            .unwrap_err()
            .to_string()
            .contains("confirmation")
    );
    assert_eq!(port.taps.len(), 1);
    let mut port = Port::new(vec![skill_confirmation()]);
    port.profile.confirm_skill_use = true;
    assert!(port.driver().cast_skill(1, 1, true).is_err());
    assert!(port.taps.is_empty());
}

#[test]
fn servant_confirmation_clicks_once_then_verifies_target_and_cooldown() {
    let target = BattleState {
        target_count: 3,
        ..Default::default()
    };
    let mut done = idle();
    done.skills[0][0] = SkillState::Cooldown.into();
    let mut port = Port::new(vec![
        idle(),
        skill_confirmation(),
        skill_confirmation(),
        target,
        done,
    ]);
    port.profile.confirm_skill_use = true;
    assert!(port.driver().cast_skill(1, 1, true).unwrap());
    assert_eq!(
        port.commands
            .iter()
            .filter(|c| **c == Command::ConfirmSkill)
            .count(),
        1
    );
    assert_eq!(
        port.taps,
        vec![
            fgo_rs::layout::UI_LAYOUT_V1.battle.servant_skills[0][0],
            fgo_rs::layout::UI_LAYOUT_V1.battle.skill_confirm,
            fgo_rs::layout::UI_LAYOUT_V1.battle.ally_targets[2]
        ]
    );
}

#[test]
fn confirmation_reappearance_or_timeout_never_retries_confirm() {
    for reappears in [true, false] {
        let mut frames = vec![idle(), skill_confirmation()];
        if reappears {
            frames.push(BattleState::default());
        }
        frames.push(skill_confirmation());
        let mut port = Port::new(frames);
        port.profile.confirm_skill_use = true;
        assert!(port.driver().cast_skill(1, 1, true).is_err());
        assert_eq!(
            port.commands
                .iter()
                .filter(|c| **c == Command::ConfirmSkill)
                .count(),
            1
        );
        assert_eq!(port.taps.len(), 2);
        if !reappears {
            assert!(port.time >= 35000 && port.time < 38000);
        }
    }
    let mut port = Port::new(vec![
        idle(),
        BattleState {
            target_count: 3,
            ..Default::default()
        },
        skill_confirmation(),
    ]);
    port.profile.confirm_skill_use = true;
    assert!(port.driver().cast_skill(1, 1, true).is_err());
    assert!(!port.commands.contains(&Command::ConfirmSkill));
}

#[test]
fn master_confirmation_requires_cooldown_after_confirming() {
    for cooled in [true, false] {
        let mut done = master();
        if cooled {
            done.master_skills[1] = SkillState::Cooldown.into();
        }
        let mut port = Port::new(vec![
            master(),
            skill_confirmation(),
            done.clone(),
            done.clone(),
            done,
            idle(),
        ]);
        port.profile.confirm_skill_use = true;
        assert_eq!(port.driver().cast_master_skill(2, true).is_ok(), cooled);
        assert_eq!(
            port.commands
                .iter()
                .filter(|c| **c == Command::ConfirmSkill)
                .count(),
            1
        );
    }
}

#[test]
fn order_change_confirmation_retains_selection_and_final_identity_checks() {
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
    let mut port = Port::new(vec![
        master(),
        skill_confirmation(),
        skill_confirmation(),
        dialog,
        first,
        both,
        after,
    ]);
    port.profile.confirm_skill_use = true;
    port.driver().order_change().unwrap();
    assert_eq!(
        port.commands
            .iter()
            .filter(|c| **c == Command::ConfirmSkill)
            .count(),
        1
    );
    assert_eq!(port.taps.len(), 5);
}

#[test]
fn default_double_speed_toggles_normal_once_and_waits_for_stable_confirmation() {
    use fgo_rs::battle::BattleSpeed;
    let normal = BattleState {
        battle_speed: BattleSpeed::Normal.into(),
        ..cards()
    };
    let mut port = Port::new(vec![
        idle(),
        normal.clone(),
        normal.clone(),
        normal,
        cards(),
        cards(),
        BattleState::default(),
    ]);
    port.driver().attack(true).unwrap();
    assert_eq!(
        port.commands
            .iter()
            .filter(|c| **c == Command::SetDoubleSpeed)
            .count(),
        1
    );
    assert_eq!(
        port.taps[1],
        fgo_rs::layout::UI_LAYOUT_V1.battle.speed_toggle
    );
    assert_eq!(port.taps.len(), 5);
    assert!(
        port.requested_facts
            .iter()
            .any(|facts| facts.contains(RequiredBattleFacts::SPEED))
    );
}

#[test]
fn unknown_or_unstable_speed_never_toggles_or_selects_cards() {
    let normal = BattleState {
        battle_speed: fgo_rs::battle::BattleSpeed::Normal.into(),
        ..cards()
    };
    let unknown = BattleState {
        battle_speed: Recognition::unknown(UnknownReason::NoCandidate, None),
        ..cards()
    };
    for (first, current) in [(unknown.clone(), unknown), (cards(), normal)] {
        let mut port = Port::new(vec![idle(), first, current]);
        assert!(port.driver().attack(true).is_err());
        assert_eq!(port.taps, vec![fgo_rs::layout::UI_LAYOUT_V1.battle.attack]);
    }
}

#[test]
fn speed_confirmation_timeout_does_not_toggle_again_or_select_cards() {
    let normal = BattleState {
        battle_speed: fgo_rs::battle::BattleSpeed::Normal.into(),
        ..cards()
    };
    let mut port = Port::new(vec![idle(), normal.clone(), normal.clone(), normal]);
    let error = port.driver().attack(true).unwrap_err().to_string();
    assert!(error.contains("confirming double battle speed"), "{error}");
    assert_eq!(
        port.taps,
        vec![
            fgo_rs::layout::UI_LAYOUT_V1.battle.attack,
            fgo_rs::layout::UI_LAYOUT_V1.battle.speed_toggle
        ]
    );
    assert!(port.time < 8000);
}

#[test]
fn unchanged_speed_policy_keeps_legacy_input_and_observation_path() {
    let unknown = BattleState {
        battle_speed: Recognition::NotRequested,
        ..cards()
    };
    let mut port = Port::new(vec![
        idle(),
        unknown.clone(),
        unknown,
        BattleState::default(),
    ]);
    port.profile.battle_speed = fgo_rs::profile::BattleSpeedPolicy::Unchanged;
    port.driver().attack(true).unwrap();
    assert_eq!(port.taps.len(), 4);
    assert!(!port.commands.contains(&Command::SetDoubleSpeed));
    assert!(
        port.requested_facts
            .iter()
            .all(|facts| !facts.contains(RequiredBattleFacts::SPEED))
    );
}
