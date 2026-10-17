use anyhow::Result;
use fgo_rs::{
    adb::parse_devices,
    battle::BattleState,
    control::{Control, ControlError},
    device::{FrameSource, InputSink, ReplayDevice},
    domain::Scene,
    farming::{Apple, FarmState},
    observation::Observation,
    profile::PartyProfile,
    replay::{ReplayComparisonMode, ReplayEventSink, compare_event_streams},
    runtime::{EventSink, Host, Runtime, RuntimeEvent},
    vision::{self, Point, Rect, Vision},
};
use image::RgbaImage;
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn control() -> Control {
    Control::new(Duration::from_secs(30))
}
fn vision() -> Vision {
    Vision::load(&root().join("assets/templates.json")).unwrap()
}
fn frame(step: u32) -> RgbaImage {
    vision::normalize(
        &image::open(root().join(format!("tests/fixtures/step{step}.png")))
            .unwrap()
            .to_rgba8(),
        None,
    )
    .unwrap()
    .0
}

#[test]
fn viewport_offsets_and_scaling_agree_with_input_coordinates() {
    let raw = RgbaImage::new(2400, 1080);
    let (image, viewport) = vision::normalize(&raw, None).unwrap();
    assert_eq!(image.dimensions(), (1280, 720));
    assert_eq!(viewport.x, 240);
    assert_eq!(
        viewport.device_point(Point { x: 640, y: 360 }).unwrap(),
        Point { x: 1200, y: 540 }
    );
    assert!(viewport.device_point(Point { x: 1280, y: 720 }).is_err());
    assert!(vision::normalize(&RgbaImage::new(1080, 1920), None).is_err());
    assert!(
        Rect {
            x: u32::MAX,
            y: 0,
            width: 10,
            height: 10
        }
        .validate(1280, 720)
        .is_err()
    );
}

#[test]
fn rider_screenshots_preserve_existing_detection_decisions() {
    let v = vision();
    let c = control();
    let quest = frame(1);
    assert!(v.find(&quest, "event", &c).unwrap().is_some());
    let found = v.find(&quest, "quest", &c).unwrap().unwrap();
    assert!(
        found.x.abs_diff(876) <= 3 && found.y.abs_diff(188) <= 3,
        "{found:?}"
    );
    let support = v.support(&frame(2), &c).unwrap().unwrap();
    assert_eq!(support.x, 800);
    assert!(support.y.abs_diff(273) <= 3, "{support:?}");
    assert!(v.find(&frame(4), "first_turn", &c).unwrap().is_some());
    assert!(v.find(&frame(7), "selected_1", &c).unwrap().is_none());
    let selected = frame(8);
    assert!(v.find(&selected, "selected_1", &c).unwrap().is_some());
    assert!(v.find(&selected, "selected_5", &c).unwrap().is_some());
    assert!(v.find(&selected, "selected_4", &c).unwrap().is_none());
    assert!(v.find(&frame(6), "master_ready", &c).unwrap().is_some());
    assert!(v.find(&frame(11), "master_used", &c).unwrap().is_some());
    let closed = frame(10);
    assert!(v.find(&closed, "master_ready", &c).unwrap().is_none());
    assert!(v.find(&closed, "master_used", &c).unwrap().is_none());
    let cards = frame(12);
    let owners: Vec<_> = (1..=5)
        .map(|i| {
            v.find(&cards, &format!("support_card_{i}"), &c)
                .unwrap()
                .is_some()
        })
        .collect();
    assert_eq!(owners, vec![true, false, true, true, false]);
}

#[test]
fn support_requires_friend_mark_and_a_complete_row() {
    let v = vision();
    let c = control();
    let mut screenshot = frame(2);
    for y in 175..720 {
        for x in 1100..1190 {
            screenshot.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
        }
    }
    assert!(v.support(&screenshot, &c).unwrap().is_none());
    assert!(v.support(&RgbaImage::new(1280, 720), &c).unwrap().is_none());
}

#[test]
fn unavailable_devices_are_not_silently_marked_connected() {
    let devices = parse_devices(
        "* daemon started successfully *\nList of devices attached\nemulator-5554 device product:test\r\nabc unauthorized\noffline-1 offline\n",
    );
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[1].state, "unauthorized");
    assert_eq!(devices[2].state, "offline");
}

#[test]
fn observations_have_a_single_primary_scene() {
    assert_eq!(BattleState::default().scene(), Scene::Unknown);
    assert_eq!(
        BattleState {
            turn: true,
            colors: [Some(fgo_rs::battle::Color::Arts); 5],
            ..Default::default()
        }
        .scene(),
        Scene::CommandCards
    );
    assert_eq!(
        FarmState {
            event: true,
            menu: true,
            ..Default::default()
        }
        .scene(),
        Scene::QuestList
    );
    assert_eq!(
        FarmState {
            ap_empty: true,
            repeat: true,
            ..Default::default()
        }
        .scene(),
        Scene::ApRecovery
    );

    let layered = Observation::classify(FarmState {
        ap_empty: true,
        apple_confirm: Some(Apple::Gold),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(layered.scene, Scene::AppleConfirmation);

    let conflict = Observation::classify(FarmState {
        support: true,
        formation: true,
        ..Default::default()
    })
    .unwrap_err();
    assert_eq!(
        conflict.candidates,
        vec![Scene::Formation, Scene::SupportList]
    );

    let conflict = Observation::classify(BattleState {
        victory: true,
        defeated: true,
        ..Default::default()
    })
    .unwrap_err();
    assert_eq!(conflict.candidates, vec![Scene::Defeated, Scene::Victory]);
}

#[derive(Clone, Default)]
struct Trace(Arc<Mutex<Vec<u8>>>);
impl Write for Trace {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct FakeDevice {
    taps: Arc<AtomicUsize>,
}
impl FrameSource for FakeDevice {
    fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
        Ok(RgbaImage::new(1920, 1080))
    }
}
impl InputSink for FakeDevice {
    fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
        self.taps.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
        self.taps.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn runtime(execute: bool, c: Control, max_ops: u64) -> (Runtime, Arc<AtomicUsize>, Trace) {
    let taps = Arc::new(AtomicUsize::new(0));
    let trace = Trace::default();
    let host = Host::new(
        Box::new(FakeDevice { taps: taps.clone() }),
        vision(),
        None,
        execute,
        Some(Box::new(trace.clone())),
        c,
    );
    (Runtime::new(host, max_ops), taps, trace)
}

#[test]
fn party_profile_templates_are_checked_before_runtime_start() {
    let taps = Arc::new(AtomicUsize::new(0));
    let mut host = Host::new(
        Box::new(FakeDevice { taps }),
        vision(),
        None,
        false,
        None,
        control(),
    );
    let mut opening_markers = PartyProfile::default().opening_markers;
    opening_markers.push("missing_fixture_marker".into());
    let profile = PartyProfile {
        opening_markers,
        ..PartyProfile::default()
    };
    let error = host.configure_profile(profile).unwrap_err().to_string();
    assert!(error.contains("missing templates"));
    assert!(error.contains("missing_fixture_marker"));
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<RuntimeEvent>>>);

impl EventSink for Events {
    fn emit(&mut self, event: RuntimeEvent) -> Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[test]
fn structured_event_sink_can_replace_jsonl_output() {
    let taps = Arc::new(AtomicUsize::new(0));
    let events = Events::default();
    let host = Host::with_event_sink(
        Box::new(FakeDevice { taps }),
        vision(),
        None,
        false,
        Box::new(events.clone()),
        control(),
    );
    Runtime::new(host, 10_000).run("capture();").unwrap();
    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[0].frame, 1);
}

#[test]
fn dry_run_records_coordinates_without_calling_device_input() {
    let (rt, taps, trace) = runtime(false, control(), 10000);
    rt.run("capture(); tap(640,360); swipe(100,200,400,500,300);")
        .unwrap();
    assert_eq!(taps.load(Ordering::SeqCst), 0);
    let text = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[1]["executed"], false);
    assert_eq!(lines[1]["device_point"]["x"], 960);
    assert_eq!(lines[1]["device_point"]["y"], 540);
    assert!(lines[0]["run_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(lines[0]["sequence"], 1);
    assert_eq!(lines[1]["sequence"], 2);
    assert_eq!(lines[2]["sequence"], 3);
    assert_eq!(lines[0]["frame"], 1);
    assert!(lines[0]["elapsed_ms"].is_u64());
    assert_eq!(lines[0]["viewport"]["width"], 1920);
    assert_eq!(lines[0]["viewport"]["height"], 1080);
    assert_eq!(lines[0]["canvas_width"], 1280);
    assert_eq!(lines[0]["canvas_height"], 720);
    assert_eq!(lines[0]["normalizer"], "center_crop_16_9_catmull_rom_v1");
}

#[test]
fn separate_frame_and_input_adapters_can_be_composed() {
    struct Frames;
    impl FrameSource for Frames {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            Ok(RgbaImage::new(1920, 1080))
        }
    }
    struct Input(Arc<AtomicUsize>);
    impl InputSink for Input {
        fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    let inputs = Arc::new(AtomicUsize::new(0));
    let host = Host::with_io(
        Box::new(Frames),
        Box::new(Input(inputs.clone())),
        vision(),
        None,
        true,
        None,
        control(),
    );
    Runtime::new(host, 10000)
        .run("capture(); tap(640, 360); swipe(1, 2, 3, 4, 5);")
        .unwrap();
    assert_eq!(inputs.load(Ordering::SeqCst), 2);
}

#[test]
fn real_input_needs_a_frame_and_valid_coordinates() {
    let (rt, taps, _) = runtime(true, control(), 10000);
    assert!(rt.run("tap(0,0);").is_err());
    assert!(rt.run("capture(); tap(-1,0);").is_err());
    assert_eq!(taps.load(Ordering::SeqCst), 0);
    rt.run("capture(); tap(640,360);").unwrap();
    assert_eq!(taps.load(Ordering::SeqCst), 1);
}

#[test]
fn script_loops_are_limited_and_errors_include_line_numbers() {
    let (rt, _, trace) = runtime(false, control(), 100);
    assert!(
        rt.run("loop {}")
            .unwrap_err()
            .to_string()
            .contains("operations")
    );
    let error = rt
        .run("\n\nfail(\"expected error\");")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("expected error") && error.contains("line 3"),
        "{error}"
    );
    let records: Vec<serde_json::Value> = String::from_utf8(trace.0.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record["action"] == "error"));
    assert!(records.iter().all(|record| record["phase"] == "script"));
}

#[test]
fn stop_interrupts_host_sleep() {
    let c = control();
    let (rt, _, _) = runtime(false, c.clone(), 100000);
    let worker = std::thread::spawn(move || rt.run("sleep(30000);"));
    std::thread::sleep(Duration::from_millis(60));
    let begin = Instant::now();
    c.stop();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(
        error.downcast_ref::<ControlError>(),
        Some(&ControlError::Stopped)
    );
    assert!(begin.elapsed() < Duration::from_secs(1));
}

#[test]
fn script_time_limit_remains_typed_and_cannot_be_caught_to_continue_inputs() {
    for source in [
        "sleep(100); capture(); tap(640,360);",
        "fn wait() { sleep(100); } wait(); capture(); tap(640,360);",
        "try { sleep(100); } catch (error) { fail(\"must not reach catch\"); }",
    ] {
        let c = Control::simulated(Duration::from_millis(100));
        let (rt, taps, trace) = runtime(true, c, 100000);
        let error = rt.run(source).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ControlError>(),
            Some(&ControlError::TimeLimitReached),
            "{source}: {error:#}"
        );
        assert_eq!(taps.load(Ordering::SeqCst), 0);
        let records: Vec<serde_json::Value> = String::from_utf8(trace.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["action"], "log");
        assert_eq!(records[0]["message"], "execution time limit reached");
    }
}

#[test]
fn script_loop_time_limit_remains_typed_without_a_host_call() {
    let (rt, taps, _) = runtime(true, Control::new(Duration::ZERO), 100000);
    let error = rt.run("loop {}").unwrap_err();
    assert_eq!(
        error.downcast_ref::<ControlError>(),
        Some(&ControlError::TimeLimitReached)
    );
    assert_eq!(taps.load(Ordering::SeqCst), 0);
}

#[test]
fn pause_prevents_actions_and_stop_also_interrupts_pause() {
    let c = control();
    c.pause();
    let (rt, taps, _) = runtime(true, c.clone(), 100000);
    let worker = std::thread::spawn(move || rt.run("capture(); tap(640,360);"));
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(taps.load(Ordering::SeqCst), 0);
    c.resume();
    worker.join().unwrap().unwrap();
    assert_eq!(taps.load(Ordering::SeqCst), 1);
    c.pause();
    c.stop();
    assert!(c.checkpoint().is_err());
}

#[test]
fn wall_clock_limit_interrupts_blocking_waits() {
    let c = Control::new(Duration::from_millis(80));
    assert!(
        c.sleep(Duration::from_secs(30))
            .unwrap_err()
            .to_string()
            .contains("time limit")
    );
}

#[test]
fn replay_exhaustion_is_an_error_instead_of_reusing_stale_frames() {
    let mut replay = ReplayDevice::new(&root().join("tests/fixtures/step2.png")).unwrap();
    assert!(replay.capture(&control()).is_ok());
    assert!(
        replay
            .capture(&control())
            .unwrap_err()
            .to_string()
            .contains("exhausted")
    );
}

#[test]
fn timed_replay_advances_the_simulated_clock_at_each_capture() {
    let directory = tempfile::tempdir().unwrap();
    RgbaImage::new(1280, 720)
        .save(directory.path().join("000001.png"))
        .unwrap();
    RgbaImage::new(1280, 720)
        .save(directory.path().join("000002.png"))
        .unwrap();
    let mut replay = ReplayDevice::with_timestamps(
        directory.path(),
        vec![Duration::from_millis(125), Duration::from_millis(900)],
    )
    .unwrap();
    let control = Control::simulated(Duration::from_secs(2));

    replay.capture(&control).unwrap();
    assert_eq!(control.elapsed(), Duration::from_millis(125));
    replay.capture(&control).unwrap();
    assert_eq!(control.elapsed(), Duration::from_millis(900));
}

#[test]
fn read_only_rhai_script_runs_on_real_reference_frame() {
    let host = Host::new(
        Box::new(ReplayDevice::new(&root().join("tests/fixtures/step2.png")).unwrap()),
        vision(),
        None,
        false,
        None,
        control(),
    );
    Runtime::new(host, 100000)
        .run(include_str!("../scripts/inspect_rider.rhai"))
        .unwrap();
    Runtime::check(include_str!("../scripts/select_rider_support.rhai")).unwrap();
}

#[test]
fn recognized_wave_reaches_rhai_battle_progress() {
    let host = Host::new(
        Box::new(ReplayDevice::new(&root().join("tests/fixtures/step4.png")).unwrap()),
        vision(),
        None,
        false,
        None,
        control(),
    );
    Runtime::new(host, 100000)
        .run(
            r#"
                verify_opening();
                let progress = battle_progress();
                if progress.turns_seen != 1 || progress.wave != 1 ||
                   progress.wave_total != 1 || progress.wave_turn != 1 {
                    fail("recognized wave did not reach battle progress");
                }
            "#,
        )
        .unwrap();
}

#[test]
fn support_script_selects_the_expected_friend_in_record_mode() {
    struct SupportFrames(usize);
    impl FrameSource for SupportFrames {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            self.0 += 1;
            if self.0 <= 2 {
                Ok(frame(2))
            } else {
                Ok(image::open(root().join("tests/fixtures/live-formation.png"))?.to_rgba8())
            }
        }
    }
    impl InputSink for SupportFrames {
        fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
        fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
    }
    let trace = Trace::default();
    let host = Host::new(
        Box::new(SupportFrames(0)),
        vision(),
        None,
        false,
        Some(Box::new(trace.clone())),
        control(),
    );
    Runtime::new(host, 100000)
        .run(include_str!("../scripts/select_rider_support.rhai"))
        .unwrap();
    let text = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect();
    let taps: Vec<_> = records.iter().filter(|a| a["action"] == "tap").collect();
    assert_eq!(taps.len(), 2);
    assert_eq!(taps[0]["point"]["x"], 363);
    assert_eq!(taps[1]["point"]["x"], 800);
    assert!(taps[1]["point"]["y"].as_u64().unwrap().abs_diff(273) <= 3);
}

#[test]
fn verified_skill_api_runs_through_the_real_rhai_host_and_vision() {
    struct SkillFrames {
        used: bool,
    }
    impl FrameSource for SkillFrames {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            let result = frame(if self.used { 5 } else { 4 });
            self.used = true;
            Ok(result)
        }
    }
    impl InputSink for SkillFrames {
        fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
        fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
    }
    let trace = Trace::default();
    let host = Host::new(
        Box::new(SkillFrames { used: false }),
        vision(),
        None,
        false,
        Some(Box::new(trace.clone())),
        control(),
    );
    Runtime::new(host, 100000)
        .run("cast_skill(1, 1, true);")
        .unwrap();
    let text = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect();
    let taps: Vec<_> = records
        .iter()
        .filter(|record| record["action"] == "tap")
        .collect();
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0]["point"]["x"], 73);
    assert_eq!(taps[0]["executed"], false);
    let command = records
        .iter()
        .find(|record| record["action"] == "command")
        .unwrap();
    assert_eq!(command["command"]["kind"], "cast_servant_skill");
    assert_eq!(command["command"]["servant"], "one");
    assert_eq!(command["command"]["skill"], "one");
    assert_eq!(command["command"]["target"]["kind"], "ally");
    assert_eq!(command["command"]["target"]["slot"], "three");
    assert!(
        records
            .iter()
            .any(|record| { record["action"] == "scene" && record["current"] == "battle_turn" })
    );
    let observations: Vec<_> = records
        .iter()
        .filter(|record| record["action"] == "observation")
        .collect();
    assert!(!observations.is_empty());
    assert_eq!(observations[0]["schema"], 2);
    assert_eq!(observations[0]["scene"], "battle_turn");
    assert_eq!(observations[0]["facts"]["turn"], true);
    assert_eq!(observations[0]["facts"]["first_turn"], true);
    assert_eq!(observations[0]["evidence"][0]["field"], "skills.1.1");
    assert_eq!(observations[0]["evidence"][0]["value"], "available");
    assert_eq!(
        observations[0]["evidence"][0]["method"],
        "color_chroma_pixel_count"
    );
    assert_eq!(
        observations[0]["evidence"][0]["detector"],
        "skill_icon_chroma"
    );
    assert_eq!(observations[0]["evidence"][0]["roi"]["width"], 41);
    assert!(records.iter().any(|record| {
        record["action"] == "verification"
            && record["description"] == "waiting for skill screen"
            && record["status"] == "started"
            && record["expected_scenes"][0] == "battle_turn"
    }));
    assert!(records.iter().any(|record| {
        record["action"] == "verification"
            && record["description"] == "waiting for skill screen"
            && record["status"] == "succeeded"
    }));
    let skill_verification = records
        .iter()
        .find(|record| {
            record["action"] == "verification"
                && record["description"] == "servant 1 skill 1 entering cooldown"
                && record["status"] == "succeeded"
        })
        .unwrap();
    assert_eq!(
        skill_verification["last_action"]["kind"],
        "cast_servant_skill"
    );
    assert_eq!(skill_verification["action_frame"], 1);
    assert_eq!(skill_verification["transition_from"], "battle_turn");
    assert_eq!(skill_verification["retry_count"], 1);
    assert_eq!(skill_verification["current_observation"]["frame"], 2);
    assert_eq!(
        skill_verification["current_observation"]["scene"],
        "battle_turn"
    );
}

#[test]
fn simulated_replay_reproduces_the_complete_skill_event_stream() {
    fn run_replay(
        path: &std::path::Path,
        timestamps: Option<Vec<Duration>>,
    ) -> Vec<serde_json::Value> {
        let (sink, events) = ReplayEventSink::new(None);
        let replay = timestamps.map_or_else(
            || ReplayDevice::new(path),
            |timestamps| ReplayDevice::with_timestamps(path, timestamps),
        );
        let host = Host::with_event_sink(
            Box::new(replay.unwrap()),
            vision(),
            None,
            false,
            Box::new(sink),
            Control::simulated(Duration::from_secs(60)),
        );
        Runtime::new(host, 100_000)
            .run("cast_skill(1, 1, true);")
            .unwrap();
        events.snapshot().unwrap()
    }

    let directory = tempfile::tempdir().unwrap();
    std::fs::copy(
        root().join("tests/fixtures/step4.png"),
        directory.path().join("000001.png"),
    )
    .unwrap();
    std::fs::copy(
        root().join("tests/fixtures/step5.png"),
        directory.path().join("000002.png"),
    )
    .unwrap();

    let expected = run_replay(directory.path(), None);
    let timestamps = expected
        .iter()
        .filter(|event| event["action"] == "capture")
        .map(|event| Duration::from_millis(event["elapsed_ms"].as_u64().unwrap()))
        .collect();
    let actual = run_replay(directory.path(), Some(timestamps));
    let report = compare_event_streams(&expected, &actual, ReplayComparisonMode::Events);

    assert!(report.matched, "{:?}", report.differences);
    assert!(expected.iter().any(|event| {
        event["action"] == "verification"
            && event["description"] == "servant 1 skill 1 entering cooldown"
            && event["status"] == "succeeded"
    }));
}

#[test]
fn settlement_script_uses_real_frames_and_reports_completed_battles() {
    struct Results(std::collections::VecDeque<&'static str>);
    impl FrameSource for Results {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            let name = self.0.pop_front().expect("unexpected extra capture");
            Ok(image::open(root().join(format!("tests/fixtures/live-{name}.png")))?.to_rgba8())
        }
    }
    impl InputSink for Results {
        fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
        fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
            panic!("record mode sent input")
        }
    }
    let trace = Trace::default();
    let host = Host::new(
        Box::new(Results(
            [
                "bond-result",
                "bond-result",
                "master-result",
                "loot",
                "repeat",
            ]
            .into(),
        )),
        vision(),
        None,
        false,
        Some(Box::new(trace.clone())),
        control(),
    );
    Runtime::new(host, 100000).run(
        "finish_battle(); let p=farm_progress(); if p.completed!=1 || p.apples_remaining!=0 {fail(\"wrong progress\");} if enter_battle() {fail(\"battle limit ignored\");}"
    ).unwrap();
    let text = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
    let records: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect();
    let taps: Vec<_> = records
        .iter()
        .filter(|record| record["action"] == "tap")
        .collect();
    assert_eq!(taps.len(), 3);
    assert!(
        taps.iter()
            .all(|a| a["point"]["x"] == 1231 && a["point"]["y"] == 687)
    );
    let commands: Vec<_> = records
        .iter()
        .filter(|record| record["action"] == "command")
        .collect();
    assert_eq!(commands.len(), 3);
    assert!(
        commands
            .iter()
            .all(|record| record["command"]["kind"] == "advance_settlement")
    );
}

#[test]
fn bond_level_and_ce_reward_settlement_replays_without_extra_battle() {
    let directory = tempfile::tempdir().unwrap();
    for (index, name) in [
        "bond-level-up",
        "bond-level-up",
        "bond-ce-reward",
        "master-result",
        "loot",
        "repeat",
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::copy(
            root().join(format!("tests/fixtures/live-{name}.png")),
            directory.path().join(format!("{index:06}.png")),
        )
        .unwrap();
    }
    let run = || {
        let trace = Trace::default();
        let host = Host::new(
            Box::new(ReplayDevice::new(directory.path()).unwrap()),
            vision(),
            None,
            false,
            Some(Box::new(trace.clone())),
            Control::simulated(Duration::from_secs(30)),
        );
        Runtime::new(host, 100000).run("finish_battle(); let p=farm_progress(); if p.completed!=1 {fail(\"not completed\");} if enter_battle() {fail(\"unexpected second battle\");}").unwrap();
        let text = String::from_utf8(trace.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>()
    };
    let first = run();
    let second = run();
    assert!(
        fgo_rs::replay::compare_event_streams(
            &first,
            &second,
            fgo_rs::replay::ReplayComparisonMode::Events
        )
        .matched
    );
    let commands: Vec<_> = first
        .iter()
        .filter(|event| event["action"] == "command")
        .map(|event| event["command"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "advance_settlement",
            "close_special_drop",
            "advance_settlement",
            "advance_settlement"
        ]
    );
}

#[test]
fn stop_interrupts_a_native_battle_wait_while_the_host_is_locked() {
    let c = control();
    let (rt, taps, _) = runtime(true, c.clone(), 100000);
    let worker = std::thread::spawn(move || rt.run("verify_opening();"));
    std::thread::sleep(Duration::from_millis(60));
    let begin = Instant::now();
    c.stop();
    assert!(worker.join().unwrap().is_err());
    assert!(begin.elapsed() < Duration::from_secs(1));
    assert_eq!(taps.load(Ordering::SeqCst), 0);
}

#[test]
fn profile_skill_api_replays_real_frames_through_rhai_and_verified_executor() {
    let directory = tempfile::tempdir().unwrap();
    for (index, step) in [4, 4, 5].into_iter().enumerate() {
        std::fs::copy(
            root().join(format!("tests/fixtures/step{step}.png")),
            directory.path().join(format!("{:06}.png", index + 1)),
        )
        .unwrap();
    }
    let (sink, events) = ReplayEventSink::new(None);
    let mut host = Host::with_event_sink(
        Box::new(ReplayDevice::new(directory.path()).unwrap()),
        vision(),
        None,
        false,
        Box::new(sink),
        Control::simulated(Duration::from_secs(60)),
    );
    let mut profile = fgo_rs::profile::PartyProfile::default();
    profile.skill_rules = vec![fgo_rs::profile::ServantSkillRule {
        atlas_skill_id: None,
        servant: fgo_rs::domain::PartySlot::One,
        skill: fgo_rs::domain::SkillSlot::One,
        target: fgo_rs::domain::PartySlot::Three.into(),
        hp_below: None,
        np_below: None,
        enemy_charge_full: false,
        any_of: Vec::new(),
        earliest: Some(fgo_rs::profile::SkillTiming { wave: 1, turn: 1 }),
    }];
    host.configure_profile(profile).unwrap();
    Runtime::new(host, 100_000)
        .run("if cast_profile_skills() != 1 { fail(\"expected one verified cast\"); }")
        .unwrap();
    let records = events.snapshot().unwrap();
    let taps: Vec<_> = records.iter().filter(|r| r["action"] == "tap").collect();
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0]["point"]["x"], 73);
    assert_eq!(taps[0]["executed"], false);
    assert!(records.iter().any(|r| r["action"] == "verification"
        && r["description"] == "servant 1 skill 1 entering cooldown"
        && r["status"] == "succeeded"));
}

#[test]
fn dynamic_lowest_np_target_replays_real_skill_target_frames() {
    let directory = tempfile::tempdir().unwrap();
    for (index, step) in [4, 4, 5].into_iter().enumerate() {
        std::fs::copy(
            root().join(format!("tests/fixtures/step{step}.png")),
            directory.path().join(format!("{:06}.png", index + 1)),
        )
        .unwrap();
    }
    let (sink, events) = ReplayEventSink::new(None);
    let mut host = Host::with_event_sink(
        Box::new(ReplayDevice::new(directory.path()).unwrap()),
        vision(),
        None,
        false,
        Box::new(sink),
        Control::simulated(Duration::from_secs(60)),
    );
    let mut profile = fgo_rs::profile::PartyProfile::default();
    profile.skill_rules = vec![fgo_rs::profile::ServantSkillRule {
        atlas_skill_id: None,
        servant: fgo_rs::domain::PartySlot::One,
        skill: fgo_rs::domain::SkillSlot::One,
        target: fgo_rs::profile::SkillRuleTarget::LowestNp {
            lowest_np: fgo_rs::profile::LowestNpTarget { below: 100 },
        },
        hp_below: None,
        np_below: None,
        enemy_charge_full: false,
        any_of: Vec::new(),
        earliest: None,
    }];
    host.configure_profile(profile).unwrap();

    Runtime::new(host, 100_000)
        .run("if cast_profile_skills() != 1 { fail(\"expected one dynamic cast\"); }")
        .unwrap();
    let records = events.snapshot().unwrap();
    let taps: Vec<_> = records.iter().filter(|r| r["action"] == "tap").collect();
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0]["point"]["x"], 73);
    assert_eq!(taps[0]["point"]["y"], 573);
    assert!(records.iter().any(|record| {
        record["action"] == "command"
            && record["command"]["kind"] == "cast_servant_skill"
            && record["command"]["target"]["slot"] == "one"
    }));
}

#[test]
fn enemy_charge_condition_skips_on_confirmed_real_non_full_gauges() {
    let directory = tempfile::tempdir().unwrap();
    for index in 1..=2 {
        std::fs::copy(
            root().join("tests/fixtures/step4.png"),
            directory.path().join(format!("{index:06}.png")),
        )
        .unwrap();
    }
    let (sink, events) = ReplayEventSink::new(None);
    let mut host = Host::with_event_sink(
        Box::new(ReplayDevice::new(directory.path()).unwrap()),
        vision(),
        None,
        false,
        Box::new(sink),
        Control::simulated(Duration::from_secs(60)),
    );
    let mut profile = fgo_rs::profile::PartyProfile::default();
    profile.skill_rules = vec![fgo_rs::profile::ServantSkillRule {
        atlas_skill_id: None,
        servant: fgo_rs::domain::PartySlot::One,
        skill: fgo_rs::domain::SkillSlot::One,
        target: fgo_rs::domain::PartySlot::Three.into(),
        hp_below: None,
        np_below: None,
        enemy_charge_full: true,
        any_of: Vec::new(),
        earliest: None,
    }];
    host.configure_profile(profile).unwrap();

    Runtime::new(host, 100_000)
        .run("if cast_profile_skills() != 0 { fail(\"full charge required\"); }")
        .unwrap();
    let records = events.snapshot().unwrap();
    assert!(!records.iter().any(|record| record["action"] == "command"));
    assert!(!records.iter().any(|record| record["action"] == "tap"));
    assert!(records.iter().any(|record| {
        record["action"] == "observation"
            && record["facts"]["enemies"][0]["charge"]["current"] == 0
            && record["facts"]["enemies"][0]["charge"]["total"] == 5
    }));
}

#[test]
fn profile_master_skill_api_replays_real_frames_through_rhai_and_verified_executor() {
    for (skill, direct) in [
        (fgo_rs::domain::SkillSlot::One, false),
        (fgo_rs::domain::SkillSlot::Three, false),
        (fgo_rs::domain::SkillSlot::Three, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let frames = if direct {
            vec![6, 11, 11, 11, 9]
        } else {
            vec![6, 6, 11, 11, 11, 9]
        };
        for (index, step) in frames.into_iter().enumerate() {
            std::fs::copy(
                root().join(format!("tests/fixtures/step{step}.png")),
                directory.path().join(format!("{:06}.png", index + 1)),
            )
            .unwrap();
        }
        let (sink, events) = ReplayEventSink::new(None);
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(directory.path()).unwrap()),
            vision(),
            None,
            false,
            Box::new(sink),
            Control::simulated(Duration::from_secs(60)),
        );
        let mut profile = fgo_rs::profile::PartyProfile::default();
        if skill == fgo_rs::domain::SkillSlot::Three {
            profile.order_change = None;
        }
        profile.master_skill_rules = vec![fgo_rs::profile::MasterSkillRule {
            skill,
            target: fgo_rs::domain::PartySlot::Two.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: Some(fgo_rs::profile::SkillTiming { wave: 1, turn: 1 }),
        }];
        host.configure_profile(profile).unwrap();
        Runtime::new(host, 100_000)
            .run(if direct {
                "if !cast_master_skill(3, true) { fail(\"expected verified cast\"); }"
            } else {
                "if cast_profile_master_skills() != 1 { fail(\"expected one verified cast\"); }"
            })
            .unwrap();
        let records = events.snapshot().unwrap();
        let taps: Vec<_> = records.iter().filter(|r| r["action"] == "tap").collect();
        assert_eq!(taps.len(), 2);
        assert_eq!(
            taps[0]["point"]["x"],
            fgo_rs::layout::UI_LAYOUT_V1.battle.master_skills[skill.index()].x
        );
        assert_eq!(taps[0]["executed"], false);
        let command = records.iter().find(|r| r["action"] == "command").unwrap();
        assert_eq!(command["command"]["kind"], "cast_master_skill");
        assert_eq!(
            command["command"]["target"]["slot"],
            if direct { "three" } else { "two" }
        );
        assert_eq!(
            command["command"]["skill"],
            if skill == fgo_rs::domain::SkillSlot::Three {
                "three"
            } else {
                "one"
            }
        );
        assert!(records.iter().any(|r| r["action"] == "verification"
            && r["description"]
                == format!("master skill {} target and animation", skill.index() + 1)
            && r["status"] == "succeeded"));
    }
}

#[test]
fn enemy_master_skill_replays_target_checks_through_real_host_and_vision() {
    let selected = vision()
        .battle_state(&frame(6), &control())
        .unwrap()
        .selected_enemy
        .unwrap();
    for direct in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut steps = vec![6, 6, 4, 4, 6, 11, 11, 11, 9];
        if !direct {
            steps.insert(0, 6);
        }
        for (index, step) in steps.into_iter().enumerate() {
            std::fs::copy(
                root().join(format!("tests/fixtures/step{step}.png")),
                directory.path().join(format!("{:06}.png", index + 1)),
            )
            .unwrap();
        }
        let (sink, events) = ReplayEventSink::new(None);
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(directory.path()).unwrap()),
            vision(),
            None,
            false,
            Box::new(sink),
            Control::simulated(Duration::from_secs(60)),
        );
        let mut profile = PartyProfile::default();
        profile.master_skill_rules = vec![fgo_rs::profile::MasterSkillRule {
            skill: fgo_rs::domain::SkillSlot::One,
            target: fgo_rs::profile::MasterSkillTarget::Enemy { enemy: selected },
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
        }];
        host.configure_profile(profile).unwrap();
        let script = if direct {
            format!(
                "if !cast_master_skill_on_enemy(1, {}, true) {{ fail(\"expected cast\"); }}",
                selected.index() + 1
            )
        } else {
            "if cast_profile_master_skills() != 1 { fail(\"expected cast\"); }".into()
        };
        Runtime::new(host, 100_000).run(&script).unwrap();
        let records = events.snapshot().unwrap();
        let commands: Vec<_> = records
            .iter()
            .filter(|r| r["action"] == "command")
            .collect();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["command"]["target"]["kind"], "enemy");
        assert!(records.iter().any(|r| r["action"] == "verification"
            && r["description"] == "master skill 1 target and animation"
            && r["status"] == "succeeded"));
    }
}

#[test]
fn servant_enemy_api_replays_real_target_and_cooldown_frames() {
    let selected = vision()
        .battle_state(&frame(4), &control())
        .unwrap()
        .selected_enemy
        .unwrap();
    for direct in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut steps = vec![4, 4, 5];
        if !direct {
            steps.insert(0, 4);
        }
        for (index, step) in steps.into_iter().enumerate() {
            std::fs::copy(
                root().join(format!("tests/fixtures/step{step}.png")),
                directory.path().join(format!("{:06}.png", index + 1)),
            )
            .unwrap();
        }
        let (sink, events) = ReplayEventSink::new(None);
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(directory.path()).unwrap()),
            vision(),
            None,
            false,
            Box::new(sink),
            Control::simulated(Duration::from_secs(60)),
        );
        let mut profile = PartyProfile::default();
        profile.skill_rules = vec![fgo_rs::profile::ServantSkillRule {
            servant: fgo_rs::domain::PartySlot::One,
            skill: fgo_rs::domain::SkillSlot::One,
            target: fgo_rs::profile::SkillRuleTarget::Enemy { enemy: selected },
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
            atlas_skill_id: None,
        }];
        host.configure_profile(profile).unwrap();
        let script = if direct {
            format!(
                "if !cast_skill_on_enemy(1, 1, {}, true) {{ fail(\"expected cast\"); }}",
                selected.index() + 1
            )
        } else {
            "if cast_profile_skills() != 1 { fail(\"expected cast\"); }".into()
        };
        Runtime::new(host, 100_000).run(&script).unwrap();
        let records = events.snapshot().unwrap();
        let commands: Vec<_> = records
            .iter()
            .filter(|r| r["action"] == "command")
            .collect();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["command"]["target"]["kind"], "enemy");
        assert!(records.iter().any(|r| r["action"] == "verification"
            && r["description"] == "servant 1 skill 1 entering cooldown"
            && r["status"] == "succeeded"));
    }
}

#[test]
fn skill_confirmation_real_popup_runs_through_rhai_and_cooldown_verification() {
    // These are deliberately arranged screenshots, not a continuous execution.
    for enabled in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        for (index, name) in [
            "step6.png",
            "live-skill-confirm.png",
            "step11.png",
            "step11.png",
            "step11.png",
            "step9.png",
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::copy(
                root().join("tests/fixtures").join(name),
                directory.path().join(format!("{index:06}.png")),
            )
            .unwrap();
        }
        let (sink, events) = ReplayEventSink::new(None);
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(directory.path()).unwrap()),
            vision(),
            None,
            false,
            Box::new(sink),
            Control::simulated(Duration::from_secs(60)),
        );
        let profile = PartyProfile {
            confirm_skill_use: enabled,
            ..PartyProfile::default()
        };
        host.configure_profile(profile).unwrap();
        let result = Runtime::new(host, 100_000).run("cast_master_skill(2,true);");
        assert_eq!(result.is_ok(), enabled, "{result:?}");
        let records = events.snapshot().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|e| e["action"] == "command" && e["command"]["kind"] == "confirm_skill")
                .count(),
            usize::from(enabled)
        );
        assert!(
            records
                .iter()
                .filter(|e| e["action"] == "tap")
                .all(|e| e["executed"] == false)
        );
    }
}

#[test]
fn default_speed_switch_runs_through_rhai_real_glyphs_and_attack_verification() {
    // Arranged real scenes: this verifies the pipeline, not a live device transition.
    let directory = tempfile::tempdir().unwrap();
    for (index, name) in [
        "step9.png",
        "owner-continuous-cards.png",
        "owner-continuous-cards.png",
        "step12.png",
        "step12.png",
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::copy(
            root().join("tests/fixtures").join(name),
            directory.path().join(format!("{index:06}.png")),
        )
        .unwrap();
    }
    for index in 5..7 {
        RgbaImage::new(1280, 720)
            .save(directory.path().join(format!("{index:06}.png")))
            .unwrap();
    }
    let (sink, events) = ReplayEventSink::new(None);
    let mut host = Host::with_event_sink(
        Box::new(ReplayDevice::new(directory.path()).unwrap()),
        vision(),
        None,
        false,
        Box::new(sink),
        Control::simulated(Duration::from_secs(60)),
    );
    host.configure_profile(PartyProfile {
        enemy_target: fgo_rs::profile::EnemyTargetStrategy::Current,
        ..PartyProfile::default()
    })
    .unwrap();
    Runtime::new(host, 100_000).run("attack(false);").unwrap();
    let records = events.snapshot().unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|e| e["action"] == "command" && e["command"]["kind"] == "set_double_speed")
            .count(),
        1
    );
    for description in ["confirming double battle speed", "starting attack"] {
        assert!(
            records.iter().any(|e| e["action"] == "verification"
                && e["description"] == description
                && e["status"] == "succeeded"),
            "{description}"
        );
    }
}
