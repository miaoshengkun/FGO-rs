#![cfg(feature = "maa")]

use fgo_rs::{
    battle::{BattleObservationRequest, RequiredBattleFacts},
    control::Control,
    maa::MaaRecognizer,
    vision::{Vision, normalize},
};
use image::RgbaImage;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[path = "support/saber_frames.rs"]
mod saber_frames;

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; verifies second-run failures with real native recognition"]
fn saber_retest_entry_settlement_and_numbers_with_native_recognition() {
    let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
    vision.set_maa_backend(backend(None)).unwrap();
    saber_frames::verify_retest_facts(&vision);
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; verifies buff-occluded card badges on live frames"]
fn saber_card_badges_survive_buff_occlusion_with_native_recognition() {
    let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
    vision.set_maa_backend(backend(None)).unwrap();
    saber_frames::verify_card_facts(&mut vision);
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn backend(model: Option<&std::path::Path>) -> MaaRecognizer {
    let library =
        std::env::var_os("FGO_MAA_LIBRARY").expect("set FGO_MAA_LIBRARY to the v5.14.2 library");
    MaaRecognizer::new(&PathBuf::from(library), &[root().join("assets/maa")], model).unwrap()
}

fn frame(name: &str) -> RgbaImage {
    let raw = image::open(root().join("tests/fixtures").join(name))
        .unwrap()
        .to_rgba8();
    normalize(&raw, None).unwrap().0
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; checks native rejection of adjacent quest levels"]
fn saber_quest_titles_reject_adjacent_levels_on_live_frames() {
    let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
    vision.set_maa_backend(backend(None)).unwrap();
    let control = Control::new(Duration::from_secs(30));
    assert!(
        vision
            .find(
                &frame("live-saber-quests-v.png"),
                "saber_quest_vii",
                &control
            )
            .unwrap()
            .is_none()
    );
    let with_vii = frame("live-saber-quests-vi-vii.png");
    let found = vision
        .find(&with_vii, "saber_quest_vii", &control)
        .unwrap()
        .unwrap();
    assert!(
        (376..431).contains(&found.y),
        "must select VII, not VI: {found:?}"
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
#[ignore = "requires FGO_MAA_LIBRARY; executes the actual native recognition engine"]
fn manifest_matches_agree_with_the_rust_reference_on_real_screenshots() {
    let reference = Vision::load(&root().join("assets/templates.json")).unwrap();
    let mut native = Vision::load(&root().join("assets/templates.json")).unwrap();
    native.set_maa_backend(backend(None)).unwrap();
    let control = Control::new(Duration::from_secs(120));
    let (mut count, mut reference_time, mut native_time) = (0, Duration::ZERO, Duration::ZERO);
    for file in [
        "step4.png",
        "step7.png",
        "step12.png",
        "live-target.png",
        "live-bond-result.png",
        "live-support-scrolled.png",
        "live-formation.png",
        "live-saber-quests-v.png",
        "live-saber-quests-vi-vii.png",
        "live-saber-support-iori120.png",
        "live-saber-cards1.png",
        "live-saber-cards2.png",
        "live-saber-cards3.png",
        "live-saber-turn1.png",
        "live-saber-turn1-charged.png",
        "live-saber-turn3.png",
        "live-saber-last-vii.png",
        "live-saber-bond-level-up.png",
        "live-saber-retest-turn2.png",
        "live-saber-retest-turn3.png",
        "live-saber-retest-cards1.png",
        "live-saber-retest-cards2.png",
        "live-saber-retest-cards3.png",
    ] {
        let image = frame(file);
        for name in reference.names() {
            let start = Instant::now();
            let expected = reference.find(&image, &name, &control).unwrap();
            reference_time += start.elapsed();
            let start = Instant::now();
            let actual = native.find(&image, &name, &control).unwrap();
            native_time += start.elapsed();
            assert_eq!(
                actual.is_some(),
                expected.is_some(),
                "{file}: {name} acceptance differs"
            );
            if let (Some(actual), Some(expected)) = (actual, expected) {
                assert_eq!(
                    (actual.x, actual.y),
                    (expected.x, expected.y),
                    "{file}: {name} location differs"
                );
                // OpenCV uses float32; the reference accumulates in float64.
                assert!(
                    (actual.score - expected.score).abs() < 0.00002,
                    "{file}: {name} score differs: {} / {}",
                    actual.score,
                    expected.score
                );
            }
            count += 1;
        }
    }
    eprintln!("{count} real-frame comparisons: reference {reference_time:?}, Maa {native_time:?}");
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; executes the actual native recognition engine"]
fn battle_scene_corpus_keeps_the_same_facts() {
    let reference = Vision::load(&root().join("assets/templates.json")).unwrap();
    let mut native = Vision::load(&root().join("assets/templates.json")).unwrap();
    native.set_maa_backend(backend(None)).unwrap();
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/vision-cases.json")).unwrap();
    let control = Control::new(Duration::from_secs(120));
    for case in cases["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let image = frame(file);
        let expected = reference.battle_state(&image, &control).unwrap();
        let actual = native.battle_state(&image, &control).unwrap();
        assert_eq!(actual.scene(), expected.scene(), "{file}");
        assert_eq!(actual.cards_complete(), expected.cards_complete(), "{file}");
        assert_eq!(actual.colors, expected.colors, "{file}");
        assert_eq!(
            actual.skills.map(|row| row.map(|skill| skill.value())),
            expected.skills.map(|row| row.map(|skill| skill.value())),
            "{file}"
        );
        assert_eq!(actual.support_cards, expected.support_cards, "{file}");
        assert_eq!(actual.markers, expected.markers, "{file}");
        assert_eq!(actual.wave, expected.wave, "{file}");
    }
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; executes the actual native recognition engine"]
fn imported_nodes_and_errors_do_not_execute_actions() {
    let backend = backend(None);
    let control = Control::new(Duration::from_secs(30));
    assert!(
        backend
            .recognize_node(&frame("step4.png"), "战斗_主界面", &control)
            .unwrap()
            .hit
    );
    assert!(
        backend
            .recognize_node(&frame("step12.png"), "战斗_选卡场景", &control)
            .unwrap()
            .hit
    );
    let blank = RgbaImage::new(1280, 720);
    assert!(
        !backend
            .recognize_node(&blank, "战斗_主界面", &control)
            .unwrap()
            .hit
    );
    assert!(
        backend
            .recognize_node(&blank, "missing-node", &control)
            .is_err()
    );
    assert!(
        backend
            .recognize_node(&blank, "战斗_卡1", &control)
            .is_err()
    );
    assert!(
        backend
            .recognize(
                &blank,
                "TemplateMatch",
                &serde_json::json!({"template": 123}),
                &control
            )
            .is_err()
    );
    assert!(
        backend
            .recognize(&blank, "Custom", &serde_json::json!({}), &control)
            .is_err()
    );
    control.stop();
    assert!(
        backend
            .recognize_node(&blank, "战斗_主界面", &control)
            .is_err()
    );
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY and FGO_MAA_OCR_MODEL; executes real OCR"]
fn card_ocr_fallback_keeps_weak_and_missing_results_unknown() {
    let model =
        PathBuf::from(std::env::var_os("FGO_MAA_OCR_MODEL").expect("set FGO_MAA_OCR_MODEL"));
    let recognizer = backend(Some(&model));
    let control = Control::new(Duration::from_secs(30));
    let image = frame("step12.png");
    for (slot, expected) in [(1, "技击"), (3, "迅击"), (4, "迅击")] {
        let result = recognizer
            .recognize_node(&image, &format!("战斗_卡{slot}"), &control)
            .unwrap();
        assert!(result.hit);
        assert_eq!(result.detail["best"]["text"], expected);
        assert!(result.detail["best"]["score"].as_f64().unwrap() >= 0.90);
    }
    let weak = recognizer
        .recognize_node(&image, "战斗_卡2", &control)
        .unwrap();
    assert_eq!(weak.detail["best"]["text"], "力击");
    assert!(weak.detail["best"]["score"].as_f64().unwrap() < 0.90);
    assert!(
        !recognizer
            .recognize_node(&image, "战斗_卡5", &control)
            .unwrap()
            .hit
    );

    let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
    vision.set_maa_backend(recognizer).unwrap();
    for file in [
        "step12.png",
        "live-speed-normal.png",
        "live-speed-double.png",
    ] {
        let image = frame(file);
        let reference = Vision::load(&root().join("assets/templates.json"))
            .unwrap()
            .battle_state(&image, &control)
            .unwrap();
        let actual = vision.battle_state(&image, &control).unwrap();
        assert_eq!(actual.scene(), reference.scene(), "{file}");
        assert_eq!(actual.colors, reference.colors, "{file}");
    }
    let mut covered = image.clone();
    for y in 520..620 {
        for x in 36..240 {
            covered.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let result = vision.battle_state(&covered, &control).unwrap();
    assert_eq!(result.colors[0], None);
    assert!(result.card_color_recognition[0].is_unknown());
    assert!(!result.cards_complete());

    // Force the same real letters down the OCR fallback path, without changing
    // their pixels. Strong Arts succeeds; weak/missing Buster stays unknown.
    let mut manifest: serde_json::Value =
        serde_json::from_str(include_str!("../assets/templates.json")).unwrap();
    for spec in manifest.as_object_mut().unwrap().values_mut() {
        spec["file"] = root()
            .join("assets")
            .join(spec["file"].as_str().unwrap())
            .to_str()
            .unwrap()
            .into();
        if let Some(alternatives) = spec
            .get_mut("alternatives")
            .and_then(serde_json::Value::as_array_mut)
        {
            for file in alternatives {
                *file = root()
                    .join("assets")
                    .join(file.as_str().unwrap())
                    .to_str()
                    .unwrap()
                    .into();
            }
        }
    }
    for name in ["arts_1", "buster_2", "buster_5"] {
        manifest[name]["threshold"] = 0.000000001.into();
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("templates.json");
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut fallback = Vision::load(&path).unwrap();
    fallback.set_maa_backend(backend(Some(&model))).unwrap();
    let result = fallback.battle_state(&image, &control).unwrap();
    assert_eq!(result.colors[0], Some(fgo_rs::battle::Color::Arts));
    assert!(result.card_color_recognition[0].value().is_some());
    assert!(result.card_color_recognition[1].is_unknown());
    assert!(result.card_color_recognition[4].is_unknown());
    assert_eq!(result.colors[1], None);
    assert_eq!(result.colors[4], None);
    assert!(!result.cards_complete());
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY and FGO_MAA_OCR_MODEL; executes real numeric OCR"]
fn numeric_ocr_diagnostics_do_not_bypass_whole_number_validation() {
    let model =
        PathBuf::from(std::env::var_os("FGO_MAA_OCR_MODEL").expect("set FGO_MAA_OCR_MODEL"));
    let recognizer = backend(Some(&model));
    let control = Control::new(Duration::from_secs(60));
    for (file, expected) in [
        ("step4.png", [Some(0), Some(70), Some(78)]),
        ("step5.png", [Some(25), Some(70), Some(78)]),
        ("step9.png", [Some(82), Some(70), Some(78)]),
        ("step10.png", [Some(100), Some(100), Some(148)]),
        ("live-enemy-standard-one.png", [Some(300), None, None]),
        (
            "live-enemy-standard-two.png",
            [Some(90), Some(64), Some(74)],
        ),
        (
            "live-enemy-standard-three.png",
            [Some(60), Some(24), Some(0)],
        ),
        ("live-enemy-reflow-one.png", [Some(8), None, Some(32)]),
        ("owner-mac-turn.png", [Some(100), Some(100), Some(148)]),
        (
            "owner-continuous-turn.png",
            [Some(100), Some(100), Some(148)],
        ),
    ] {
        let image = frame(file);
        for (index, expected) in expected.into_iter().enumerate() {
            let result = recognizer
                .recognize_node(&image, &format!("战斗_NP卡{}", index + 1), &control)
                .unwrap();
            let text = result.detail["best"]["text"].as_str();
            let value = text.and_then(|text| text.strip_suffix('%')?.parse::<u16>().ok());
            assert_eq!(result.hit, expected.is_some(), "{file}: NP {}", index + 1);
            assert_eq!(value, expected, "{file}: NP {}", index + 1);
        }
    }
    // HP OCR also rejects two otherwise clear numbers because of punctuation.
    // Do not weaken its format/score to turn these diagnostic misses into values.
    for (file, expected) in [
        (
            "step4.png",
            [Some("14,219"), Some("16,057"), Some("21,455")],
        ),
        ("step10.png", [Some("17,793"), Some("16,057"), None]),
        (
            "owner-mac-turn.png",
            [Some("17,793"), Some("16,057"), Some("20,455")],
        ),
        (
            "live-enemy-standard-three.png",
            [Some("16,457"), Some("14,985"), None],
        ),
        (
            "live-enemy-standard-two.png",
            [Some("16,457"), Some("14,985"), Some("15,363")],
        ),
        (
            "owner-continuous-turn.png",
            [Some("17,793"), Some("16,057"), Some("20,455")],
        ),
    ] {
        let image = frame(file);
        for (index, expected) in expected.into_iter().enumerate() {
            let result = recognizer
                .recognize_node(&image, &format!("战斗_HP{}", index + 1), &control)
                .unwrap();
            assert_eq!(result.hit, expected.is_some(), "{file}: HP {}", index + 1);
            assert_eq!(
                result.detail["best"]["text"].as_str(),
                expected,
                "{file}: HP {}",
                index + 1
            );
        }
    }

    let source = frame("live-enemy-standard-one.png");
    let mut missing_digit = source.clone();
    for y in 655..672 {
        for x in 246..255 {
            missing_digit.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    let result = recognizer
        .recognize_node(&missing_digit, "战斗_NP卡1", &control)
        .unwrap();
    assert!(result.hit);
    assert_eq!(result.detail["best"]["text"], "30%");
    assert!(result.detail["best"]["score"].as_f64().unwrap() > 0.99);

    let mut native = Vision::load(&root().join("assets/templates.json")).unwrap();
    native.set_maa_backend(recognizer).unwrap();
    let observation = native
        .battle_state_with_request(
            &missing_digit,
            &control,
            &[],
            BattleObservationRequest::new(RequiredBattleFacts::NP, &[]),
        )
        .unwrap();
    assert!(observation.idle());
    assert!(observation.np_percent[0].is_unknown());
    assert_eq!(observation.np_percent[0].value(), None);
}

#[test]
#[ignore = "requires FGO_MAA_LIBRARY; verifies turn fades preserve live Saber identities"]
fn saber_turn_portrait_fade_with_native_recognition() {
    let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
    vision.set_maa_backend(backend(None)).unwrap();
    saber_frames::verify_roster_fade(&mut vision);
}
