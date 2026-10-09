use fgo_rs::{
    battle::{BattleObservationRequest, BattleTracker, Color, RequiredBattleFacts},
    control::Control,
    domain::PartySlot,
    vision::{Vision, normalize},
};
use std::{path::Path, time::Duration};

pub fn verify_card_facts(vision: &mut Vision) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let image = |name: &str| image::open(root.join(name)).unwrap().to_rgba8();
    vision
        .configure_card_identities(
            vec![
                (PartySlot::Two, vec![image("saber-benienma-card.png")]),
                (PartySlot::Five, vec![image("saber-nero-bride-card.png")]),
            ],
            Some(PartySlot::Three),
        )
        .unwrap();
    vision
        .configure_support_identities(vec![(106000, vec![image("saber-iori-card.png")])])
        .unwrap();
    let control = Control::new(Duration::from_secs(30));
    use Color::{Arts as A, Buster as B, Quick as Q};
    use PartySlot::{Five as F, Three as S, Two as T};
    for (name, colors, owners, badges) in [
        (
            "live-saber-cards1.png",
            [Q, A, B, Q, B],
            [F, F, T, T, F],
            [false; 5],
        ),
        (
            "live-saber-cards2.png",
            [A, A, A, Q, B],
            [T, F, S, S, F],
            [false, false, true, true, false],
        ),
        (
            "live-saber-cards3.png",
            [B, B, A, B, A],
            [S, T, T, S, S],
            [true, false, false, true, true],
        ),
        (
            "live-saber-retest-cards1.png",
            [Q, A, A, B, A],
            [F, S, F, F, T],
            [false, true, false, false, false],
        ),
        (
            "live-saber-retest-cards2.png",
            [A, B, B, A, A],
            [S, T, S, T, F],
            [true, false, true, false, false],
        ),
        (
            "live-saber-retest-cards3.png",
            [Q, B, B, B, Q],
            [S, S, F, T, T],
            [true, true, false, false, false],
        ),
    ] {
        let state = vision
            .battle_state_with_request(
                &image(name),
                &control,
                &[],
                BattleObservationRequest::new(RequiredBattleFacts::CARDS, &[]),
            )
            .unwrap();
        assert!(state.cards_complete(), "{name}");
        assert_eq!(state.colors, colors.map(Some), "{name}");
        assert_eq!(state.card_owners, owners.map(Some), "{name}");
        assert_eq!(state.support_cards, badges, "{name}");
        assert_eq!(
            state.support_atlas_id,
            badges.iter().any(|badge| *badge).then_some(106000),
            "{name}: {:?}",
            state.support_identity
        );
    }

    // With both visible badges erased, neither artwork nor the Profile alone
    // may turn these cards into a confirmed support slot.
    let mut hidden = image("live-saber-cards2.png");
    for index in [2, 3] {
        for y in 395..446 {
            for x in 125 + 257 * index..238 + 257 * index {
                hidden.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
    }
    let state = vision
        .battle_state_with_request(
            &hidden,
            &control,
            &[],
            BattleObservationRequest::new(RequiredBattleFacts::CARDS, &[]),
        )
        .unwrap();
    assert_eq!(state.support_cards, [false; 5]);
    assert_eq!(state.card_owners[2..4], [None, None]);
    assert!(state.support_identity.is_unknown());
}

pub fn verify_retest_facts(vision: &Vision) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let frame = |name: &str| image::open(root.join(name)).unwrap().to_rgba8();
    let control = Control::new(Duration::from_secs(30));
    let last = frame("live-saber-last-vii.png");
    let found = vision
        .find(&last, "saber_quest_vii", &control)
        .unwrap()
        .unwrap();
    assert!((180..200).contains(&found.y), "{found:?}");
    // Losing the full numeral must not leave enough Chinese title evidence.
    let mut damaged = last;
    for y in 175..204 {
        for x in 950..985 {
            damaged.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    assert!(
        vision
            .find(&damaged, "saber_quest_vii", &control)
            .unwrap()
            .is_none()
    );

    for name in ["live-bond-level-up.png", "live-saber-bond-level-up.png"] {
        let original = frame(name);
        assert!(
            vision.farm_state(&original, &control).unwrap().next,
            "{name}"
        );
        for (x, y, width, height) in [(495, 94, 720, 112), (495, 622, 290, 56)] {
            let mut hidden = original.clone();
            for y in y..y + height {
                for x in x..x + width {
                    hidden.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
            assert!(
                !vision.farm_state(&hidden, &control).unwrap().next,
                "{name}"
            );
        }
    }
    for (name, hp, np) in [
        (
            "live-saber-retest-turn2.png",
            [15268, 10848, 17462],
            [34, 53, 92],
        ),
        (
            "live-saber-retest-turn3.png",
            [8427, 9668, 16372],
            [76, 77, 220],
        ),
    ] {
        let state = vision
            .battle_state_with_request(
                &frame(name),
                &control,
                &[],
                BattleObservationRequest::new(
                    RequiredBattleFacts::HP.union(RequiredBattleFacts::NP),
                    &[],
                ),
            )
            .unwrap();
        assert!(state.turn);
        assert_eq!(
            state.ally_hp.map(|reading| reading.value()),
            hp.map(Some),
            "{name}"
        );
        assert_eq!(
            state.np_percent.map(|reading| reading.value()),
            np.map(Some),
            "{name}"
        );
    }
}

pub fn verify_roster_fade(vision: &mut Vision) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let control = Control::new(Duration::from_secs(30));
    let state = |name: &str| {
        let raw = image::open(root.join("tests/fixtures").join(name))
            .unwrap()
            .to_rgba8();
        vision
            .battle_state_with_markers_and_facts(
                &normalize(&raw, None).unwrap().0,
                &control,
                &[],
                RequiredBattleFacts::TRACKING,
            )
            .unwrap()
    };
    let stable = state("live-saber-roster-stable.png");
    let fade = state("live-saber-roster-fade.png");
    assert!(stable.idle() && fade.idle());
    let mut tracker = BattleTracker::default();
    tracker.observe(&stable).unwrap();
    tracker
        .order_change_committed(PartySlot::One, PartySlot::Five)
        .unwrap();
    tracker.observe(&stable).unwrap();
    let expected = tracker.progress().positions;
    for turn in 2..=3 {
        assert!(tracker.attack_committed());
        tracker.observe(&fade).unwrap();
        assert_eq!(tracker.progress().positions, expected, "fading turn {turn}");
        assert!(!tracker.roster_ready());
        tracker.observe(&stable).unwrap();
        assert_eq!(tracker.progress().positions, expected, "stable turn {turn}");
        assert!(tracker.roster_ready());
        assert_eq!(tracker.progress().turns_seen, turn);
    }
    vision
        .configure_card_identities(
            vec![
                (
                    PartySlot::Two,
                    vec![
                        image::open(root.join("tests/fixtures/saber-benienma-card.png"))
                            .unwrap()
                            .to_rgba8(),
                    ],
                ),
                (
                    PartySlot::Five,
                    vec![
                        image::open(root.join("tests/fixtures/saber-nero-bride-card.png"))
                            .unwrap()
                            .to_rgba8(),
                    ],
                ),
            ],
            Some(PartySlot::Three),
        )
        .unwrap();
    let cards = vision
        .battle_state_with_markers_and_facts(
            &image::open(root.join("tests/fixtures/live-saber-roster-cards.png"))
                .unwrap()
                .to_rgba8(),
            &control,
            &[],
            RequiredBattleFacts::CARDS,
        )
        .unwrap();
    assert_eq!(cards.np_glow, [true; 3]);
    assert_eq!(
        cards.card_owners,
        [
            Some(PartySlot::Two),
            Some(PartySlot::Two),
            Some(PartySlot::Five),
            Some(PartySlot::Three),
            Some(PartySlot::Three)
        ]
    );
    let profile: fgo_rs::profile::PartyProfile =
        serde_json::from_str(include_str!("../../profiles/saber-v1.json")).unwrap();
    let selected = fgo_rs::battle::rank_cards_with_party_data(
        &cards,
        [false, false, true],
        tracker.progress().frontline(),
        [None, None, Some(Color::Buster)],
        &profile.cards,
    )
    .unwrap();
    assert_eq!(selected[0], 8);
    assert!(
        selected[1..].iter().all(|card| [4, 5].contains(card)),
        "{selected:?}"
    );
}
