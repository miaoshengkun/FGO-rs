use fgo_rs::{
    adb::Adb,
    catalog::Region,
    farming::{Apple, FixedFarmConfig},
    profile::RIDER_PROFILE_V1,
    replay::{ReplayComparisonMode, compare_event_streams},
    runner::{self, EventObserver, RecognitionConfig, RunConfig, RunHooks},
    runtime::{FrameSink, RuntimeEvent},
    vision::Point,
};
use image::RgbaImage;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

fn config(folder: &Path) -> RunConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut profile = RIDER_PROFILE_V1.clone();
    profile.servants.clear();
    profile.support_candidates.clear();
    let profile_path = folder.join("中文 profile.json");
    std::fs::write(&profile_path, serde_json::to_string(&profile).unwrap()).unwrap();
    let script = folder.join("中文 strategy.rhai");
    std::fs::write(&script, "capture(); log(\"共用 Runner\");").unwrap();
    RunConfig {
        fixed_farm: None,
        adb: Adb {
            executable: "must-not-call-adb".into(),
            timeout: Duration::from_secs(1),
        },
        script,
        serial: None,
        replay: Some(root.join("tests/fixtures/live-saber-last-vii.png")),
        replay_trace: None,
        replay_compare: None,
        replay_report: None,
        execute: false,
        templates: root.join("assets/templates.json"),
        recognition: RecognitionConfig::default(),
        profile: profile_path,
        region: Region::Cn,
        data_dir: folder.join("no-atlas-required"),
        viewport: None,
        max_seconds: 60,
        max_operations: 100_000,
        trace: folder.join("shared.jsonl"),
        frames: Some(folder.join("shared frames")),
        battles: 1,
        apples: 0,
        apple: Apple::Gold,
    }
}

struct Events(Arc<Mutex<Vec<Value>>>);
impl EventObserver for Events {
    fn observe(&mut self, event: &RuntimeEvent) {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(event).unwrap());
    }
}
struct Frames(Arc<Mutex<Vec<RgbaImage>>>);
impl FrameSink for Frames {
    fn publish(&mut self, _: u64, _: u64, frame: &RgbaImage) {
        self.0.lock().unwrap().push(frame.clone());
    }
}
fn read_trace(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn fixed_farming_completes_two_recorded_battles_without_scripts_profiles_or_atlas() {
    let folder = tempfile::tempdir().unwrap();
    let mut config = config(folder.path());
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let replay = folder.path().join("fixed replay");
    std::fs::create_dir(&replay).unwrap();
    // Keep the recorded scene/portraits, but make only skill 1 of the first
    // frontline servant ready. Place actual cooldown overlays in the other
    // slots so this replay verifies a cast and cooldown, rather than bypassing
    // automatic skills with an unrelated scripted profile.
    let original = image::open(fixtures.join("step4.png")).unwrap().to_rgba8();
    let (mut ready, _) = fgo_rs::vision::normalize(&original, None).unwrap();
    let overlay = image::open(config.templates.parent().unwrap().join("cn/still.png"))
        .unwrap()
        .to_rgba8();
    let templates: Value =
        serde_json::from_slice(&std::fs::read(&config.templates).unwrap()).unwrap();
    for slot in 1..=3 {
        for skill in 1..=3 {
            if (slot, skill) == (1, 1) {
                continue;
            }
            let roi = &templates[format!("skill_cd_{slot}_{skill}")]["region"];
            image::imageops::overlay(
                &mut ready,
                &overlay,
                roi["x"].as_i64().unwrap(),
                roi["y"].as_i64().unwrap(),
            );
        }
    }
    let mut cooling = ready.clone();
    let roi = &templates["skill_cd_1_1"]["region"];
    image::imageops::overlay(
        &mut cooling,
        &overlay,
        roi["x"].as_i64().unwrap(),
        roi["y"].as_i64().unwrap(),
    );
    let mut sequence = vec![
        "live-saber-last-vii.png",
        "live-saber-last-vii.png",
        "live-quest-confirm.png",
        "step2.png",
        "step2.png",
        "live-formation.png",
        "live-formation.png",
        "skill-ready",
        "skill-ready",
        "skill-ready",
        "live-skill-confirm.png",
        "live-target.png",
        "skill-cooling",
    ];
    sequence.extend(vec!["skill-cooling"; 9]);
    sequence.extend([
        "live-speed-double.png",
        "live-speed-double.png",
        "blank",
        "blank",
        "live-bond-result.png",
        "live-bond-result.png",
        "live-repeat.png",
        // Repeat skips formation; the second battle must reset tracking anyway.
        "live-repeat.png",
        "step2.png",
        "step2.png",
        "skill-ready",
        "skill-ready",
        "skill-ready",
        "skill-ready",
        "skill-ready",
        "live-skill-confirm.png",
        "live-target.png",
        "skill-cooling",
    ]);
    sequence.extend(vec!["skill-cooling"; 9]);
    sequence.extend([
        "live-speed-double.png",
        "live-speed-double.png",
        "blank",
        "blank",
        "live-bond-result.png",
        "live-bond-result.png",
        "live-repeat.png",
    ]);
    for (index, source) in sequence.iter().enumerate() {
        let destination = replay.join(format!("{index:06}.png"));
        match *source {
            "blank" => RgbaImage::new(1280, 720).save(destination).unwrap(),
            "skill-ready" => ready.save(destination).unwrap(),
            "skill-cooling" => cooling.save(destination).unwrap(),
            _ => {
                std::fs::copy(fixtures.join(source), destination).unwrap();
            }
        }
    }
    config.replay = Some(replay);
    config.fixed_farm = Some(FixedFarmConfig {
        quest: Point { x: 910, y: 205 },
        team: 0,
    });
    config.battles = 2;
    config.script = folder.path().join("no script.rhai");
    config.profile = folder.path().join("no profile.json");
    runner::prepare(config.clone(), config.control(), RunHooks::default())
        .unwrap()
        .run()
        .unwrap();
    let events = read_trace(&config.trace);
    let commands = events
        .iter()
        .filter_map(|event| event.get("command"))
        .collect::<Vec<_>>();
    for kind in [
        "select_quest",
        "confirm_quest",
        "select_first_support",
        "start_battle",
        "cast_servant_skill",
        "confirm_skill",
        "attack",
        "select_cards",
        "advance_settlement",
    ] {
        assert!(
            commands.iter().any(|command| command["kind"] == kind),
            "missing {kind}"
        );
    }
    assert_eq!(
        commands
            .iter()
            .filter(|command| command["kind"] == "select_first_support")
            .count(),
        2
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command["kind"] == "attack")
            .count(),
        2
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command["kind"] == "start_battle")
            .count(),
        1
    );
    assert!(!commands.iter().any(|command| matches!(
        command["kind"].as_str(),
        Some("cast_master_skill" | "order_change" | "select_support" | "select_team")
    )));
    assert_eq!(
        commands
            .iter()
            .filter(|command| command["kind"] == "cast_servant_skill")
            .count(),
        2
    );
    assert_eq!(
        commands
            .iter()
            .filter(|command| command["kind"] == "confirm_skill")
            .count(),
        2
    );
    let skills_and_attacks = commands
        .iter()
        .filter_map(|command| command["kind"].as_str())
        .filter(|kind| matches!(*kind, "cast_servant_skill" | "attack"))
        .collect::<Vec<_>>();
    assert_eq!(
        skills_and_attacks,
        [
            "cast_servant_skill",
            "attack",
            "cast_servant_skill",
            "attack"
        ]
    );
    assert!(
        events
            .iter()
            .any(|event| event["message"] == "Farming stopped: 2 battles, 0 apples remaining")
    );
    assert!(
        events
            .iter()
            .any(|event| event["point"]["x"] == 910 && event["point"]["y"] == 205)
    );
    assert!(
        events
            .iter()
            .all(|event| event.get("executed").is_none_or(|value| *value == false))
    );
    let turns = events
        .iter()
        .filter(|event| event.get("progress").is_some())
        .filter_map(|event| event["progress"]["turns_seen"].as_u64())
        .collect::<Vec<_>>();
    assert_eq!(turns.iter().copied().max(), Some(1));
}

#[test]
fn fixed_farming_validates_coordinates_team_and_region_before_creating_outputs() {
    let folder = tempfile::tempdir().unwrap();
    for (point, team, region) in [
        (Point { x: 1280, y: 0 }, 0, Region::Cn),
        (Point { x: 0, y: 720 }, 0, Region::Cn),
        (Point { x: 0, y: 0 }, 16, Region::Cn),
        (Point { x: 0, y: 0 }, 0, Region::Jp),
    ] {
        let mut config = config(folder.path());
        config.fixed_farm = Some(FixedFarmConfig { quest: point, team });
        config.region = region;
        assert!(runner::prepare(config.clone(), config.control(), RunHooks::default()).is_err());
        assert!(!config.trace.exists());
    }
}

#[test]
fn cli_and_observed_runner_emit_identical_semantics_and_normalized_frames() {
    let folder = tempfile::tempdir().unwrap();
    let config = config(folder.path());
    let events = Arc::new(Mutex::new(Vec::new()));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let hooks = RunHooks {
        events: Some(Box::new(Events(events.clone()))),
        frames: Some(Box::new(Frames(frames.clone()))),
    };
    runner::prepare(config.clone(), config.control(), hooks)
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(*events.lock().unwrap(), read_trace(&config.trace));
    let recorded = image::open(config.frames.as_ref().unwrap().join("000001.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(recorded.dimensions(), (1280, 720));
    assert_eq!(frames.lock().unwrap().as_slice(), &[recorded]);
    let cli_trace = folder.path().join("cli.jsonl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fgo-rs"))
        .arg("run")
        .arg(&config.script)
        .arg("--replay")
        .arg(config.replay.as_ref().unwrap())
        .arg("--profile")
        .arg(&config.profile)
        .arg("--templates")
        .arg(&config.templates)
        .arg("--trace")
        .arg(&cli_trace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = compare_event_streams(
        &read_trace(&config.trace),
        &read_trace(&cli_trace),
        ReplayComparisonMode::Events,
    );
    assert!(report.matched, "{report:?}");
}

#[test]
fn shared_runner_preserves_timed_replay_comparison() {
    let folder = tempfile::tempdir().unwrap();
    let first = config(folder.path());
    runner::prepare(first.clone(), first.control(), RunHooks::default())
        .unwrap()
        .run()
        .unwrap();
    let mut second = first.clone();
    second.replay_trace = Some(first.trace);
    second.trace = folder.path().join("second.jsonl");
    second.frames = Some(folder.path().join("second frames"));
    second.replay_compare = Some(ReplayComparisonMode::Events);
    second.replay_report = Some(folder.path().join("report.json"));
    runner::prepare(second.clone(), second.control(), RunHooks::default())
        .unwrap()
        .run()
        .unwrap();
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(second.replay_report.unwrap()).unwrap())
            .unwrap();
    assert_eq!(report["matched"], true);
}

#[test]
fn invalid_input_mode_and_stopped_start_never_create_a_trace() {
    let folder = tempfile::tempdir().unwrap();
    let mut config = config(folder.path());
    config.execute = true;
    assert!(runner::prepare(config.clone(), config.control(), RunHooks::default()).is_err());
    config.execute = false;
    config.serial = Some("unexpected-second-source".into());
    assert!(config.validate().is_err());
    config.serial = None;
    let control = config.control();
    control.stop();
    assert!(runner::prepare(config.clone(), control, RunHooks::default()).is_err());
    assert!(!config.trace.exists());
}

#[test]
fn failure_returns_the_saved_diagnostic_frame_path() {
    let folder = tempfile::tempdir().unwrap();
    let config = config(folder.path());
    std::fs::write(&config.script, "capture(); fail(\"deliberate failure\");").unwrap();
    let error = runner::prepare(config.clone(), config.control(), RunHooks::default())
        .unwrap()
        .run()
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("deliberate failure"));
    let diagnostic = std::fs::read_dir(folder.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("failure-")
        })
        .unwrap();
    assert!(message.contains(diagnostic.to_str().unwrap()));
    assert_eq!(image::open(diagnostic).unwrap().width(), 1280);
}
