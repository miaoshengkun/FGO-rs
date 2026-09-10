use crate::{
    battle::{BattleDriver, BattlePort, BattleState},
    control::Control,
    device::Device,
    farming::{Apple, FarmDriver, FarmPort, FarmSession, FarmState},
    vision::{self, Point, Rect, Vision},
};
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use rhai::{Dynamic, Engine, EvalAltResult, INT, Map};
use serde::Serialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Capture {
        width: u32,
        height: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        file: Option<PathBuf>,
    },
    Tap {
        point: Point,
        device_point: Point,
        executed: bool,
    },
    Swipe {
        from: Point,
        to: Point,
        device_from: Point,
        device_to: Point,
        duration_ms: u64,
        executed: bool,
    },
    Log {
        message: String,
    },
}

pub struct Host {
    device: Box<dyn Device>,
    vision: Vision,
    frame: Option<RgbaImage>,
    viewport: Option<Rect>,
    mapped_viewport: Option<Rect>,
    execute: bool,
    trace: Option<Box<dyn Write + Send>>,
    recording: Option<PathBuf>,
    frame_count: u64,
    farm: FarmSession,
    pub control: Control,
}

impl Host {
    pub fn new(
        device: Box<dyn Device>,
        vision: Vision,
        viewport: Option<Rect>,
        execute: bool,
        trace: Option<Box<dyn Write + Send>>,
        control: Control,
    ) -> Self {
        Self {
            device,
            vision,
            frame: None,
            viewport,
            mapped_viewport: None,
            execute,
            trace,
            recording: None,
            frame_count: 0,
            farm: FarmSession::default(),
            control,
        }
    }

    fn record(&mut self, action: Action) -> Result<()> {
        eprintln!("{}", serde_json::to_string(&action)?);
        if let Some(trace) = self.trace.as_mut() {
            serde_json::to_writer(&mut *trace, &action)?;
            writeln!(trace)?;
            trace.flush()?;
        }
        Ok(())
    }

    pub fn capture(&mut self) -> Result<()> {
        self.control.checkpoint()?;
        let raw = self.device.capture(&self.control)?;
        let (frame, viewport) = vision::normalize(&raw, self.viewport)?;
        self.frame_count += 1;
        let file = self
            .recording
            .as_ref()
            .map(|dir| dir.join(format!("{:06}.png", self.frame_count)));
        if let Some(file) = &file {
            frame.save(file)?;
        }
        self.frame = Some(frame);
        self.mapped_viewport = Some(viewport);
        self.record(Action::Capture {
            width: raw.width(),
            height: raw.height(),
            file,
        })
    }

    pub fn record_frames(&mut self, path: PathBuf) -> Result<()> {
        ensure!(
            !path.exists(),
            "frame directory already exists: {}",
            path.display()
        );
        std::fs::create_dir_all(&path)?;
        self.recording = Some(path);
        Ok(())
    }

    pub fn configure_farm(&mut self, battles: u32, apples: u32, apple: Apple) {
        self.farm.max_battles = battles;
        self.farm.apples_remaining = apples;
        self.farm.apple = apple;
    }

    fn farming<T>(
        &mut self,
        operation: impl FnOnce(&mut FarmDriver<'_, Self>) -> Result<T>,
    ) -> Result<T> {
        let mut session = std::mem::take(&mut self.farm);
        let result = operation(&mut FarmDriver {
            port: self,
            session: &mut session,
        });
        self.farm = session;
        result
    }

    pub fn find(&self, name: &str) -> Result<Option<vision::Match>> {
        self.control.checkpoint()?;
        let frame = self
            .frame
            .as_ref()
            .context("call capture() before recognition")?;
        if name == "support120" {
            self.vision.support(frame, &self.control)
        } else {
            self.vision.find(frame, name, &self.control)
        }
    }

    pub fn tap(&mut self, point: Point) -> Result<()> {
        self.control.checkpoint()?;
        let device_point = self
            .mapped_viewport
            .context("call capture() before input")?
            .device_point(point)?;
        if self.execute {
            self.device.tap(device_point, &self.control)?;
        }
        self.record(Action::Tap {
            point,
            device_point,
            executed: self.execute,
        })
    }

    pub fn swipe(&mut self, from: Point, to: Point, duration: Duration) -> Result<()> {
        self.control.checkpoint()?;
        let viewport = self
            .mapped_viewport
            .context("call capture() before input")?;
        let device_from = viewport.device_point(from)?;
        let device_to = viewport.device_point(to)?;
        if self.execute {
            self.device
                .swipe(device_from, device_to, duration, &self.control)?;
        }
        self.record(Action::Swipe {
            from,
            to,
            device_from,
            device_to,
            duration_ms: duration.as_millis() as u64,
            executed: self.execute,
        })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.frame
            .as_ref()
            .context("no captured frame")?
            .save(path)?;
        Ok(())
    }
}

impl BattlePort for Host {
    fn observe(&mut self) -> Result<BattleState> {
        self.capture()?;
        self.vision.battle_state(
            self.frame.as_ref().context("missing captured frame")?,
            &self.control,
        )
    }
    fn tap(&mut self, point: Point) -> Result<()> {
        Host::tap(self, point)
    }
    fn sleep(&mut self, ms: u64) -> Result<()> {
        self.control.sleep(Duration::from_millis(ms))
    }
    fn elapsed_ms(&self) -> u64 {
        self.control.elapsed().as_millis() as u64
    }
    fn log(&mut self, message: &str) -> Result<()> {
        self.record(Action::Log {
            message: message.into(),
        })
    }
}

impl FarmPort for Host {
    fn observe_farm(&mut self) -> Result<FarmState> {
        self.capture()?;
        self.vision.farm_state(
            self.frame.as_ref().context("missing captured frame")?,
            &self.control,
        )
    }
    fn swipe(&mut self, from: Point, to: Point, ms: u64) -> Result<()> {
        Host::swipe(self, from, to, Duration::from_millis(ms))
    }
}

type ScriptResult<T> = std::result::Result<T, Box<EvalAltResult>>;

fn script_error(error: anyhow::Error) -> Box<EvalAltResult> {
    format!("{error:#}").into()
}
fn point(x: INT, y: INT) -> Result<Point> {
    ensure!(
        (0..1280).contains(&x) && (0..720).contains(&y),
        "coordinates must be inside 1280x720"
    );
    Ok(Point {
        x: x as u32,
        y: y as u32,
    })
}
fn milliseconds(ms: INT) -> Result<Duration> {
    ensure!(
        (0..=3_600_000).contains(&ms),
        "duration must be between 0 and 3600000 ms"
    );
    Ok(Duration::from_millis(ms as u64))
}
fn with_host<T>(
    host: &Arc<Mutex<Host>>,
    f: impl FnOnce(&mut Host) -> Result<T>,
) -> ScriptResult<T> {
    let mut host = host
        .lock()
        .map_err(|_| Box::<EvalAltResult>::from("host lock poisoned"))?;
    f(&mut host).map_err(script_error)
}

pub struct Runtime {
    engine: Engine,
    host: Arc<Mutex<Host>>,
}

impl Runtime {
    pub fn new(host: Host, max_operations: u64) -> Self {
        let control = host.control.clone();
        let host = Arc::new(Mutex::new(host));
        let mut engine = Engine::new();
        engine.set_max_operations(max_operations);
        engine.set_max_call_levels(32);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_string_size(64 * 1024);
        engine.set_max_array_size(4096);
        engine.set_max_map_size(256);
        // Scripts use only registered capabilities; no filesystem module resolver.
        engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
        let progress = control.clone();
        engine.on_progress(move |_| {
            progress
                .checkpoint()
                .err()
                .map(|e| Dynamic::from(e.to_string()))
        });
        let h = host.clone();
        engine.register_fn("capture", move || with_host(&h, |h| h.capture()));
        let h = host.clone();
        engine.register_fn("find", move |name: &str| -> ScriptResult<Map> {
            with_host(&h, |h| {
                let mut map = Map::new();
                let found = h.find(name)?;
                map.insert("found".into(), found.is_some().into());
                if let Some(m) = found {
                    map.insert("x".into(), (m.x as INT).into());
                    map.insert("y".into(), (m.y as INT).into());
                    map.insert("score".into(), m.score.into());
                }
                Ok(map)
            })
        });
        let h = host.clone();
        engine.register_fn("matches", move |name: &str| {
            with_host(&h, |h| Ok(h.find(name)?.is_some()))
        });
        let h = host.clone();
        engine.register_fn("tap", move |x: INT, y: INT| {
            with_host(&h, |h| h.tap(point(x, y)?))
        });
        let h = host.clone();
        engine.register_fn(
            "swipe",
            move |x1: INT, y1: INT, x2: INT, y2: INT, ms: INT| {
                with_host(&h, |h| {
                    h.swipe(point(x1, y1)?, point(x2, y2)?, milliseconds(ms)?)
                })
            },
        );
        let sleep_control = control.clone();
        engine.register_fn("sleep", move |ms: INT| -> ScriptResult<()> {
            let duration = milliseconds(ms).map_err(script_error)?;
            sleep_control.sleep(duration).map_err(script_error)
        });
        engine.register_fn("elapsed_ms", move || control.elapsed().as_millis() as INT);
        let h = host.clone();
        engine.register_fn("log", move |message: &str| {
            with_host(&h, |h| {
                h.record(Action::Log {
                    message: message.into(),
                })
            })
        });
        engine.register_fn("fail", |message: &str| -> ScriptResult<()> {
            Err(message.into())
        });
        let h = host.clone();
        engine.register_fn("verify_opening", move || {
            with_host(&h, |h| BattleDriver { port: h }.verify_opening())
        });
        let h = host.clone();
        engine.register_fn("verify_support", move || {
            with_host(&h, |h| BattleDriver { port: h }.verify_support())
        });
        let h = host.clone();
        engine.register_fn("cast_skill", move |slot: INT, skill: INT, strict: bool| {
            with_host(&h, |h| {
                ensure!(
                    (1..=3).contains(&slot) && (1..=3).contains(&skill),
                    "servant and skill must be 1..3"
                );
                BattleDriver { port: h }.cast_skill(slot as usize, skill as usize, strict)
            })
        });
        let h = host.clone();
        engine.register_fn("cast_master_skill", move |skill: INT, strict: bool| {
            with_host(&h, |h| {
                ensure!((1..=2).contains(&skill), "master skill must be 1 or 2");
                BattleDriver { port: h }.cast_master_skill(skill as usize, strict)
            })
        });
        let h = host.clone();
        engine.register_fn("order_change", move || {
            with_host(&h, |h| BattleDriver { port: h }.order_change())
        });
        let h = host.clone();
        engine.register_fn("attack", move |first: bool| {
            with_host(&h, |h| BattleDriver { port: h }.attack(first))
        });
        let h = host.clone();
        engine.register_fn("wait_next_turn", move || {
            with_host(&h, |h| BattleDriver { port: h }.wait_next())
        });
        let h = host.clone();
        engine.register_fn("enter_battle", move || {
            with_host(&h, |h| h.farming(|d| d.enter()))
        });
        let h = host.clone();
        engine.register_fn("select_support", move || {
            with_host(&h, |h| h.farming(|d| d.select_support()))
        });
        let h = host.clone();
        engine.register_fn("finish_battle", move || {
            with_host(&h, |h| h.farming(|d| d.finish()))
        });
        let h = host.clone();
        engine.register_fn("farm_progress", move || {
            with_host(&h, |h| {
                let mut map = Map::new();
                map.insert("completed".into(), (h.farm.completed as INT).into());
                map.insert(
                    "apples_remaining".into(),
                    (h.farm.apples_remaining as INT).into(),
                );
                Ok(map)
            })
        });
        Self { engine, host }
    }

    pub fn run(&self, source: &str) -> Result<()> {
        self.engine
            .run(source)
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }

    pub fn check(source: &str) -> Result<()> {
        // compile only; unavailable host function names are diagnosed at runtime.
        Engine::new()
            .compile(source)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
    }

    pub fn save_last_frame(&self, path: &Path) -> Result<()> {
        self.host
            .lock()
            .map_err(|_| anyhow::anyhow!("host lock poisoned"))?
            .save(path)
    }
}
