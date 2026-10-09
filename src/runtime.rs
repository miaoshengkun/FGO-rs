use crate::{
    battle::{
        BattleDriver, BattleObservationRequest, BattleOutcome, BattlePort, BattleProgress,
        BattleState, BattleTracker, Color, RequiredBattleFacts,
    },
    control::Control,
    device::{Device, FrameSource, InputSink, SplitDevice},
    domain::{Command, Scene, VerificationStatus},
    executor::Executor,
    farming::{Apple, FarmDriver, FarmPort, FarmSession, FarmState, RequiredFarmFacts},
    observation::{FieldEvidence, Observation, SceneFacts},
    profile::{PartyProfile, RIDER_PROFILE_V1},
    verification::{ActionVerificationContext, VerificationOrigin},
    vision::{self, Point, Rect, Vision},
};
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use rhai::{
    AST, ASTNode, Dynamic, Engine, EvalAltResult, Expr, INT, Map, OptimizationLevel, Scope,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FunctionSignature {
    name: &'static str,
    arity: usize,
}

const HOST_API: &[FunctionSignature] = &[
    FunctionSignature {
        name: "attack_other_nps",
        arity: 0,
    },
    FunctionSignature {
        name: "require_profile",
        arity: 1,
    },
    FunctionSignature {
        name: "attack_np",
        arity: 2,
    },
    FunctionSignature {
        name: "cast_skill_on_enemy",
        arity: 4,
    },
    FunctionSignature {
        name: "cast_master_skill_on_enemy",
        arity: 3,
    },
    FunctionSignature {
        name: "cast_profile_master_skills",
        arity: 0,
    },
    FunctionSignature {
        name: "cast_profile_skills",
        arity: 0,
    },
    FunctionSignature {
        name: "capture",
        arity: 0,
    },
    FunctionSignature {
        name: "find",
        arity: 1,
    },
    FunctionSignature {
        name: "matches",
        arity: 1,
    },
    FunctionSignature {
        name: "tap",
        arity: 2,
    },
    FunctionSignature {
        name: "swipe",
        arity: 5,
    },
    FunctionSignature {
        name: "sleep",
        arity: 1,
    },
    FunctionSignature {
        name: "elapsed_ms",
        arity: 0,
    },
    FunctionSignature {
        name: "log",
        arity: 1,
    },
    FunctionSignature {
        name: "fail",
        arity: 1,
    },
    FunctionSignature {
        name: "verify_opening",
        arity: 0,
    },
    FunctionSignature {
        name: "verify_support",
        arity: 0,
    },
    FunctionSignature {
        name: "cast_skill",
        arity: 3,
    },
    FunctionSignature {
        name: "cast_master_skill",
        arity: 2,
    },
    FunctionSignature {
        name: "order_change",
        arity: 0,
    },
    FunctionSignature {
        name: "attack",
        arity: 1,
    },
    FunctionSignature {
        name: "wait_next_turn",
        arity: 0,
    },
    FunctionSignature {
        name: "enter_battle",
        arity: 0,
    },
    FunctionSignature {
        name: "select_support",
        arity: 0,
    },
    FunctionSignature {
        name: "finish_battle",
        arity: 0,
    },
    FunctionSignature {
        name: "farm_progress",
        arity: 0,
    },
    FunctionSignature {
        name: "battle_progress",
        arity: 0,
    },
];

// Rhai evaluates this intrinsic directly instead of exposing function metadata.
// Dynamic eval/Fn/call are intentionally omitted because they bypass call validation.
const RHAI_INTRINSICS: &[FunctionSignature] = &[FunctionSignature {
    name: "type_of",
    arity: 1,
}];

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    Capture {
        width: u32,
        height: u32,
        viewport: Rect,
        canvas_width: u32,
        canvas_height: u32,
        normalizer: &'static str,
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
    Decision {
        message: String,
    },
    Verification {
        #[serde(flatten)]
        context: ActionVerificationContext,
        status: VerificationStatus,
    },
    Error {
        phase: &'static str,
        message: String,
    },
    Scene {
        #[serde(skip_serializing_if = "Option::is_none")]
        previous: Option<Scene>,
        current: Scene,
    },
    Observation {
        schema: u32,
        scene: Scene,
        facts: serde_json::Value,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        unknown_fields: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        evidence: Vec<FieldEvidence>,
    },
    Command {
        command: Command,
    },
    BattleProgress {
        progress: BattleProgress,
    },
}

#[derive(Debug, Serialize)]
pub struct RuntimeEvent {
    pub run_id: String,
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub frame: u64,
    #[serde(flatten)]
    pub action: Action,
}

pub trait EventSink: Send {
    fn emit(&mut self, event: RuntimeEvent) -> Result<()>;
}

pub struct JsonlEventSink {
    trace: Option<Box<dyn Write + Send>>,
}

impl JsonlEventSink {
    pub fn new(trace: Option<Box<dyn Write + Send>>) -> Self {
        Self { trace }
    }
}

impl EventSink for JsonlEventSink {
    fn emit(&mut self, event: RuntimeEvent) -> Result<()> {
        let json = serde_json::to_string(&event)?;
        eprintln!("{json}");
        if let Some(trace) = self.trace.as_mut() {
            writeln!(trace, "{json}")?;
            trace.flush()?;
        }
        Ok(())
    }
}

pub struct Host {
    device: Box<dyn Device>,
    vision: Vision,
    frame: Option<RgbaImage>,
    viewport: Option<Rect>,
    mapped_viewport: Option<Rect>,
    execute: bool,
    events: Box<dyn EventSink>,
    recording: Option<PathBuf>,
    frame_count: u64,
    event_count: u64,
    run_id: String,
    last_scene: Option<Scene>,
    last_command: Option<(Command, u64)>,
    profile: PartyProfile,
    profile_markers: Vec<String>,
    noble_phantasm_colors: [Option<Color>; 6],
    atlas_noble_phantasm_colors: BTreeMap<u32, Color>,
    battle: BattleTracker,
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
        Self::with_event_sink(
            device,
            vision,
            viewport,
            execute,
            Box::new(JsonlEventSink::new(trace)),
            control,
        )
    }

    pub fn with_event_sink(
        device: Box<dyn Device>,
        vision: Vision,
        viewport: Option<Rect>,
        execute: bool,
        events: Box<dyn EventSink>,
        control: Control,
    ) -> Self {
        Self {
            device,
            vision,
            frame: None,
            viewport,
            mapped_viewport: None,
            execute,
            events,
            recording: None,
            frame_count: 0,
            event_count: 0,
            run_id: format!(
                "{}-{}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
                std::process::id(),
                RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            last_scene: None,
            last_command: None,
            profile: RIDER_PROFILE_V1.clone(),
            profile_markers: RIDER_PROFILE_V1.required_markers(),
            noble_phantasm_colors: [None; 6],
            atlas_noble_phantasm_colors: BTreeMap::new(),
            battle: BattleTracker::default(),
            farm: FarmSession::default(),
            control,
        }
    }

    pub fn with_io(
        frames: Box<dyn FrameSource>,
        input: Box<dyn InputSink>,
        vision: Vision,
        viewport: Option<Rect>,
        execute: bool,
        trace: Option<Box<dyn Write + Send>>,
        control: Control,
    ) -> Self {
        Self::new(
            Box::new(SplitDevice::new(frames, input)),
            vision,
            viewport,
            execute,
            trace,
            control,
        )
    }

    fn record(&mut self, action: Action) -> Result<()> {
        self.event_count += 1;
        self.events.emit(RuntimeEvent {
            run_id: self.run_id.clone(),
            sequence: self.event_count,
            elapsed_ms: self.control.elapsed().as_millis() as u64,
            frame: self.frame_count,
            action,
        })
    }

    fn record_scene(&mut self, current: Scene) -> Result<()> {
        if current == Scene::Unknown || self.last_scene == Some(current) {
            return Ok(());
        }
        let previous = self.last_scene.replace(current);
        self.record(Action::Scene { previous, current })
    }

    fn record_battle_progress(&mut self) -> Result<()> {
        self.record(Action::BattleProgress {
            progress: self.battle.progress().clone(),
        })
    }

    fn accept_observation<T: SceneFacts + Serialize>(&mut self, facts: T) -> Result<T> {
        match Observation::classify(facts) {
            Ok(Observation { scene, facts }) => {
                self.record(Action::Observation {
                    schema: 2,
                    scene,
                    facts: serde_json::to_value(&facts)?,
                    unknown_fields: facts.unknown_fields(),
                    evidence: facts.recognition_evidence(),
                })?;
                self.record_scene(scene)?;
                Ok(facts)
            }
            Err(conflict) => {
                self.record(Action::Error {
                    phase: "recognition",
                    message: conflict.to_string(),
                })?;
                Err(conflict.into())
            }
        }
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
            viewport,
            canvas_width: vision::WIDTH,
            canvas_height: vision::HEIGHT,
            normalizer: "center_crop_16_9_catmull_rom_v1",
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

    pub fn configure_profile(&mut self, profile: PartyProfile) -> Result<()> {
        profile.validate()?;
        let markers = profile.required_markers();
        let missing: Vec<_> = profile
            .required_templates()
            .iter()
            .filter(|name| !self.vision.has_template(name))
            .cloned()
            .collect();
        ensure!(
            missing.is_empty(),
            "party profile {} references missing templates: {}",
            profile.id,
            missing.join(", ")
        );
        self.vision.farm_templates = profile.farm_templates.clone();
        self.profile = profile;
        self.profile_markers = markers;
        Ok(())
    }

    pub fn configure_noble_phantasm_colors(
        &mut self,
        colors: impl IntoIterator<Item = (crate::domain::PartySlot, Color)>,
    ) -> Result<()> {
        let mut configured = [None; 6];
        for (slot, color) in colors {
            ensure!(
                configured[slot.index()].is_none(),
                "duplicate NP color for party slot {}",
                slot.index() + 1
            );
            configured[slot.index()] = Some(color);
        }
        self.noble_phantasm_colors = configured;
        Ok(())
    }

    pub fn configure_atlas_noble_phantasm_colors(
        &mut self,
        colors: impl IntoIterator<Item = (u32, Color)>,
    ) -> Result<()> {
        let mut configured = BTreeMap::new();
        for (atlas_id, color) in colors {
            ensure!(atlas_id > 0, "invalid Atlas servant ID");
            ensure!(
                configured.insert(atlas_id, color).is_none(),
                "duplicate NP color for Atlas servant {atlas_id}"
            );
        }
        self.atlas_noble_phantasm_colors = configured;
        Ok(())
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

impl Executor for Host {
    fn tap(&mut self, point: Point) -> Result<()> {
        Host::tap(self, point)
    }
    fn swipe(&mut self, from: Point, to: Point, ms: u64) -> Result<()> {
        Host::swipe(self, from, to, Duration::from_millis(ms))
    }
    fn sleep(&mut self, ms: u64) -> Result<()> {
        self.control.sleep(Duration::from_millis(ms))
    }
    fn elapsed_ms(&self) -> u64 {
        self.control.elapsed().as_millis() as u64
    }
    fn current_frame_id(&self) -> u64 {
        self.frame_count
    }
    fn verification_origin(&self) -> VerificationOrigin {
        VerificationOrigin {
            last_action: self.last_command.map(|(command, _)| command),
            action_frame: self.last_command.map(|(_, frame)| frame),
        }
    }
    fn log(&mut self, message: &str) -> Result<()> {
        self.record(Action::Log {
            message: message.into(),
        })
    }
    fn decision(&mut self, message: &str) -> Result<()> {
        self.record(Action::Decision {
            message: message.into(),
        })
    }
    fn verification(
        &mut self,
        context: &ActionVerificationContext,
        status: VerificationStatus,
    ) -> Result<()> {
        self.record(Action::Verification {
            context: context.clone(),
            status,
        })
    }
    fn command(&mut self, command: Command) -> Result<()> {
        let starts_battle = command == Command::StartBattle;
        self.last_command = Some((command, self.frame_count));
        self.record(Action::Command { command })?;
        if starts_battle && self.battle.reset() {
            self.record_battle_progress()?;
        }
        Ok(())
    }
}

impl BattlePort for Host {
    fn observe(&mut self) -> Result<BattleState> {
        self.observe_required(RequiredBattleFacts::ALL)
    }

    fn observe_required(&mut self, facts: RequiredBattleFacts) -> Result<BattleState> {
        self.observe_for(BattleObservationRequest::new(facts, &[]))
    }

    fn observe_for(&mut self, request: BattleObservationRequest<'_>) -> Result<BattleState> {
        self.capture()?;
        let state = self.vision.battle_state_with_request(
            self.frame.as_ref().context("missing captured frame")?,
            &self.control,
            &self.profile_markers,
            BattleObservationRequest::new(
                request.facts.union(RequiredBattleFacts::TRACKING),
                request.expected_scenes,
            ),
        )?;
        let state = self.accept_observation(state)?;
        if self.battle.observe(&state)? {
            self.record_battle_progress()?;
        }
        Ok(state)
    }
    fn profile(&self) -> &PartyProfile {
        &self.profile
    }
    fn battle_progress(&self) -> Option<BattleProgress> {
        Some(self.battle.progress().clone())
    }
    fn frontline(&self) -> [Option<crate::domain::PartySlot>; 3] {
        self.battle.progress().frontline()
    }
    fn noble_phantasm_color(&self, identity: crate::domain::PartySlot) -> Option<Color> {
        self.noble_phantasm_colors[identity.index()]
    }
    fn atlas_noble_phantasm_color(&self, atlas_id: u32) -> Option<Color> {
        self.atlas_noble_phantasm_colors.get(&atlas_id).copied()
    }
    fn attack_committed(&mut self) -> Result<()> {
        self.battle.attack_committed();
        Ok(())
    }
    fn order_change_committed(
        &mut self,
        field: crate::domain::PartySlot,
        reserve: crate::domain::PartySlot,
    ) -> Result<()> {
        if self.battle.order_change_committed(field, reserve)? {
            self.record_battle_progress()?;
        }
        Ok(())
    }
}

impl FarmPort for Host {
    fn observe_farm(&mut self) -> Result<FarmState> {
        self.observe_farm_required(RequiredFarmFacts::ALL)
    }

    fn observe_farm_required(&mut self, facts: RequiredFarmFacts) -> Result<FarmState> {
        self.capture()?;
        let state = self.vision.farm_state_with_facts(
            self.frame.as_ref().context("missing captured frame")?,
            &self.control,
            facts,
        )?;
        self.accept_observation(state)
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

fn configure_engine(engine: &mut Engine, max_operations: u64) {
    // Validation walks the unoptimized AST so calls in constant/dead branches are not hidden.
    engine.set_optimization_level(OptimizationLevel::None);
    engine.set_max_operations(max_operations);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_string_size(64 * 1024);
    engine.set_max_array_size(4096);
    engine.set_max_map_size(256);
    engine.set_max_variables(256);
    engine.set_max_functions(64);
    engine.set_max_modules(0);
    engine.set_max_strings_interned(1024);
    engine.set_strict_variables(true);
    // Scripts use only registered capabilities; no filesystem module resolver.
    engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());
}

fn available_function_signatures(engine: &Engine, ast: &AST) -> BTreeSet<(String, usize)> {
    let mut signatures = engine
        .collect_fn_metadata(
            None,
            |info| {
                info.namespace
                    .is_empty()
                    .then(|| (info.metadata.name.to_string(), info.metadata.num_params))
            },
            true,
        )
        .into_iter()
        .collect::<BTreeSet<_>>();
    signatures.extend(
        ast.iter_functions()
            .map(|function| (function.name.to_owned(), function.params.len())),
    );
    signatures.extend(
        HOST_API
            .iter()
            .map(|function| (function.name.to_owned(), function.arity)),
    );
    signatures.extend(
        RHAI_INTRINSICS
            .iter()
            .map(|function| (function.name.to_owned(), function.arity)),
    );
    signatures
}

fn compile_and_validate(engine: &Engine, source: &str) -> Result<AST> {
    let ast = engine
        .compile(source)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let available = available_function_signatures(engine, &ast);
    let mut validation_error = None;

    ast.walk(&mut |path| {
        let Some(ASTNode::Expr(Expr::FnCall(call, position))) = path.last() else {
            return true;
        };
        if call.is_operator_call() {
            return true;
        }

        let name = call.name.as_str();
        let arity = call.args.len();
        if call.is_qualified() {
            validation_error = Some(anyhow::anyhow!(
                "module-qualified Rhai function `{name}` is disabled at {position}"
            ));
            return false;
        }
        if available.contains(&(name.to_owned(), arity)) {
            return true;
        }

        let host_arities = HOST_API
            .iter()
            .filter(|function| function.name == name)
            .map(|function| function.arity.to_string())
            .collect::<Vec<_>>();
        validation_error = Some(if host_arities.is_empty() {
            anyhow::anyhow!("unknown Rhai function `{name}` with {arity} argument(s) at {position}")
        } else {
            anyhow::anyhow!(
                "Rhai host function `{name}` expects {} argument(s), found {arity} at {position}",
                host_arities.join(" or ")
            )
        });
        false
    });

    validation_error.map_or(Ok(ast), Err)
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
        configure_engine(&mut engine, max_operations);
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
        engine.register_fn("require_profile", move |id: &str| {
            with_host(&h, |h| {
                ensure!(h.profile.id == id, "script requires profile {id}");
                Ok(())
            })
        });
        let h = host.clone();
        engine.register_fn("attack_np", move |slot: INT, required: bool| {
            with_host(&h, |h| {
                ensure!((1..=3).contains(&slot), "NP field slot must be 1..3");
                BattleDriver { port: h }.attack_np(slot as usize, required)
            })
        });
        let h = host.clone();
        engine.register_fn("attack_other_nps", move || {
            with_host(&h, |h| BattleDriver { port: h }.attack_other_nps())
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
        engine.register_fn(
            "cast_skill_on_enemy",
            move |slot: INT, skill: INT, enemy: INT, strict: bool| {
                with_host(&h, |h| {
                    ensure!(
                        (1..=3).contains(&slot) && (1..=3).contains(&skill),
                        "servant and skill must be 1..3"
                    );
                    ensure!((1..=6).contains(&enemy), "enemy slot must be 1..6");
                    let target = crate::domain::EnemySlot::try_from(enemy as usize)
                        .map_err(anyhow::Error::msg)?;
                    BattleDriver { port: h }.cast_skill_on_enemy(
                        slot as usize,
                        skill as usize,
                        target,
                        strict,
                    )
                })
            },
        );
        let h = host.clone();
        engine.register_fn("cast_profile_skills", move || {
            with_host(&h, |h| {
                BattleDriver { port: h }
                    .cast_profile_skills()
                    .map(|count| count as INT)
            })
        });
        let h = host.clone();
        engine.register_fn("cast_profile_master_skills", move || {
            with_host(&h, |h| {
                BattleDriver { port: h }
                    .cast_profile_master_skills()
                    .map(|count| count as INT)
            })
        });
        let h = host.clone();
        engine.register_fn("cast_master_skill", move |skill: INT, strict: bool| {
            with_host(&h, |h| {
                ensure!((1..=3).contains(&skill), "master skill must be 1..3");
                BattleDriver { port: h }.cast_master_skill(skill as usize, strict)
            })
        });
        let h = host.clone();
        engine.register_fn(
            "cast_master_skill_on_enemy",
            move |skill: INT, enemy: INT, strict: bool| {
                with_host(&h, |h| {
                    ensure!((1..=3).contains(&skill), "master skill must be 1..3");
                    ensure!((1..=6).contains(&enemy), "enemy slot must be 1..6");
                    let target = crate::domain::EnemySlot::try_from(enemy as usize)
                        .map_err(anyhow::Error::msg)?;
                    BattleDriver { port: h }.cast_master_skill_on_enemy(
                        skill as usize,
                        target,
                        strict,
                    )
                })
            },
        );
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
        let h = host.clone();
        engine.register_fn("battle_progress", move || {
            with_host(&h, |h| {
                let progress = h.battle.progress();
                let mut map = Map::new();
                map.insert("turns_seen".into(), (progress.turns_seen as INT).into());
                map.insert("wave_turn".into(), (progress.wave_turn as INT).into());
                map.insert(
                    "wave".into(),
                    progress.wave.map_or(0, |wave| wave.current as INT).into(),
                );
                map.insert(
                    "wave_total".into(),
                    progress.wave.map_or(0, |wave| wave.total as INT).into(),
                );
                map.insert(
                    "outcome".into(),
                    match progress.outcome {
                        Some(BattleOutcome::Victory) => "victory",
                        Some(BattleOutcome::Defeated) => "defeated",
                        None => "",
                    }
                    .into(),
                );
                map.insert(
                    "frontline".into(),
                    progress
                        .frontline()
                        .into_iter()
                        .map(|slot| Dynamic::from(slot.map_or(0, |slot| slot.index() + 1) as INT))
                        .collect::<rhai::Array>()
                        .into(),
                );
                Ok(map)
            })
        });
        debug_assert!(HOST_API.iter().all(|function| {
            !engine
                .collect_fn_metadata(
                    None,
                    |info| {
                        (info.namespace.is_empty()
                            && info.metadata.namespace.is_global_namespace()
                            && info.metadata.name == function.name
                            && info.metadata.num_params == function.arity)
                            .then_some(())
                    },
                    false,
                )
                .is_empty()
        }));
        Self { engine, host }
    }

    pub fn run(&self, source: &str) -> Result<()> {
        let result = compile_and_validate(&self.engine, source).and_then(|ast| {
            let ast = self
                .engine
                .optimize_ast(&Scope::new(), ast, OptimizationLevel::Simple);
            self.engine
                .run_ast(&ast)
                .map_err(|error| anyhow::anyhow!(error.to_string()))
        });
        if let Err(error) = &result
            && let Ok(mut host) = self.host.lock()
        {
            let _ = host.record(Action::Error {
                phase: "script",
                message: format!("{error:#}"),
            });
        }
        result
    }

    pub fn check(source: &str) -> Result<()> {
        let mut engine = Engine::new();
        configure_engine(&mut engine, 1_000_000);
        compile_and_validate(&engine, source)?;
        Ok(())
    }

    pub fn save_last_frame(&self, path: &Path) -> Result<()> {
        self.host
            .lock()
            .map_err(|_| anyhow::anyhow!("host lock poisoned"))?
            .save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::ReplayDevice;

    #[derive(Clone, Default)]
    struct Events(Arc<Mutex<Vec<RuntimeEvent>>>);

    impl EventSink for Events {
        fn emit(&mut self, event: RuntimeEvent) -> Result<()> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[test]
    fn static_validation_accepts_host_standard_and_script_functions() {
        Runtime::check(
            r#"
                fn report(value) { log(type_of(value)); }
                if false { tap(1, 2); cast_skill_on_enemy(1, 1, 1, true); cast_master_skill_on_enemy(2, 1, true); cast_profile_skills(); cast_profile_master_skills(); }
                report([1, 2].len());
                let progress = battle_progress();
                log(`${progress.turns_seen}`);
            "#,
        )
        .unwrap();
    }

    #[test]
    fn static_validation_rejects_unknown_calls_before_optimization() {
        let error = Runtime::check("if false { captuer(); }").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("unknown Rhai function `captuer`"));
        assert!(message.contains("0 argument(s)"));
    }

    #[test]
    fn static_validation_reports_host_arity() {
        let arity = Runtime::check("tap(1);").unwrap_err().to_string();
        assert!(arity.contains("host function `tap` expects 2 argument(s), found 1"));
    }

    #[test]
    fn committed_roster_changes_reach_events_and_rhai_progress() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let events = Events::default();
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(&root.join("tests/fixtures/step2.png")).unwrap()),
            Vision::load(&root.join("assets/templates.json")).unwrap(),
            None,
            false,
            Box::new(events.clone()),
            Control::new(Duration::from_secs(1)),
        );
        BattlePort::order_change_committed(
            &mut host,
            crate::domain::PartySlot::One,
            crate::domain::PartySlot::Five,
        )
        .unwrap();

        Runtime::new(host, 10_000)
            .run(
                r#"
                    let progress = battle_progress();
                    if progress.frontline != [5, 2, 3] || progress.turns_seen != 0 {
                        fail("unexpected battle progress");
                    }
                "#,
            )
            .unwrap();

        assert!(events.0.lock().unwrap().iter().any(|event| matches!(
            &event.action,
            Action::BattleProgress { progress }
                if progress.frontline()
                    == [
                        Some(crate::domain::PartySlot::Five),
                        Some(crate::domain::PartySlot::Two),
                        Some(crate::domain::PartySlot::Three),
                    ]
        )));
    }

    #[test]
    fn scene_conflicts_are_recorded_as_recognition_errors() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let events = Events::default();
        let mut host = Host::with_event_sink(
            Box::new(ReplayDevice::new(&root.join("tests/fixtures/step1.png")).unwrap()),
            Vision::load(&root.join("assets/templates.json")).unwrap(),
            None,
            false,
            Box::new(events.clone()),
            Control::new(Duration::from_secs(1)),
        );
        let error = host
            .accept_observation(FarmState {
                support: true,
                formation: true,
                ..Default::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("conflicting primary scenes"));
        let events = events.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        match &events[0].action {
            Action::Error { phase, message } => {
                assert_eq!(*phase, "recognition");
                assert!(message.contains("Formation"));
                assert!(message.contains("SupportList"));
            }
            action => panic!("unexpected event: {action:?}"),
        }
    }
}
