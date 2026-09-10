use anyhow::Result;
use fgo_rs::{
    adb::parse_devices,
    control::Control,
    device::{Device, ReplayDevice},
    runtime::{Host, Runtime},
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
impl Device for FakeDevice {
    fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
        Ok(RgbaImage::new(1920, 1080))
    }
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
    let (rt, _, _) = runtime(false, control(), 100);
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
}

#[test]
fn stop_interrupts_host_sleep() {
    let c = control();
    let (rt, _, _) = runtime(false, c.clone(), 100000);
    let worker = std::thread::spawn(move || rt.run("sleep(30000);"));
    std::thread::sleep(Duration::from_millis(60));
    let begin = Instant::now();
    c.stop();
    assert!(worker.join().unwrap().is_err());
    assert!(begin.elapsed() < Duration::from_secs(1));
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
fn support_script_selects_the_expected_friend_in_record_mode() {
    struct SupportFrames(usize);
    impl Device for SupportFrames {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            self.0 += 1;
            if self.0 <= 2 {
                Ok(frame(2))
            } else {
                Ok(image::open(root().join("tests/fixtures/live-formation.png"))?.to_rgba8())
            }
        }
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
    let taps: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|a| a["action"] == "tap")
        .collect();
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
    impl Device for SkillFrames {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            let result = frame(if self.used { 5 } else { 4 });
            self.used = true;
            Ok(result)
        }
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
    let taps: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|a| a["action"] == "tap")
        .collect();
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0]["point"]["x"], 73);
    assert_eq!(taps[0]["executed"], false);
}

#[test]
fn settlement_script_uses_real_frames_and_reports_completed_battles() {
    struct Results(std::collections::VecDeque<&'static str>);
    impl Device for Results {
        fn capture(&mut self, _: &Control) -> Result<RgbaImage> {
            let name = self.0.pop_front().expect("unexpected extra capture");
            Ok(image::open(root().join(format!("tests/fixtures/live-{name}.png")))?.to_rgba8())
        }
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
    let taps: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|a| a["action"] == "tap")
        .collect();
    assert_eq!(taps.len(), 3);
    assert!(
        taps.iter()
            .all(|a| a["point"]["x"] == 1231 && a["point"]["y"] == 687)
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
