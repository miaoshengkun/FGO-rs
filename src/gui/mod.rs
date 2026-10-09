//! Slint presentation layer. A single worker owns each device/runner session.
mod state;

use crate::{
    adb::{Adb, DeviceInfo},
    catalog::Region,
    control::Control,
    device::{AdbDevice, FrameSource, ReplayDevice},
    farming::{Apple, FixedFarmConfig},
    runner::{self, RecognitionConfig, RunConfig, RunHooks},
    vision::{self, Point},
};
use anyhow::{Context, Result, ensure};
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use state::{LOG_LIMIT, Mailbox, UiFrames, UiObserver};
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

slint::include_modules!();

pub struct GuiOptions {
    pub resources: Option<PathBuf>,
    pub adb: PathBuf,
    pub output: Option<PathBuf>,
    pub script: Option<PathBuf>,
    pub profile: Option<PathBuf>,
    pub replay: Option<PathBuf>,
}

enum Request {
    Discover {
        adb: Adb,
        address: Option<String>,
    },
    Preview {
        adb: Adb,
        serial: Option<String>,
        replay: Option<PathBuf>,
    },
    Run(Box<RunConfig>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Discover,
    Preview,
    Run,
}

enum JobResult {
    Devices(Vec<DeviceInfo>),
    Preview,
    Run,
}

struct Job {
    kind: JobKind,
    control: Control,
    done: Receiver<std::result::Result<JobResult, String>>,
    worker: thread::JoinHandle<()>,
}

struct Controller {
    root: PathBuf,
    output: PathBuf,
    replay: Option<PathBuf>,
    profiles: Vec<PathBuf>,
    scripts: Vec<PathBuf>,
    devices: Vec<DeviceInfo>,
    job: Option<Job>,
    pending: Option<Request>,
    mailbox: Mailbox,
    logs: VecDeque<String>,
    closing: bool,
}

pub fn run(options: GuiOptions) -> Result<()> {
    let root = resource_root(options.resources.as_deref())?;
    let (profiles, profile_index) = choices(
        &root.join("profiles"),
        "json",
        options.profile.as_deref(),
        "rider-v1.json",
    )?;
    let (scripts, script_index) = choices(
        &root.join("scripts"),
        "rhai",
        options.script.as_deref(),
        "inspect_rider.rhai",
    )?;
    let home = user_data_dir()?;
    let output = absolute(options.output.unwrap_or_else(|| home.join("logs")))?;
    let replay = options.replay.map(absolute).transpose()?;
    let ui = AppWindow::new()?;
    ui.set_task_index(i32::from(
        options.script.is_some() || options.profile.is_some(),
    ));
    ui.set_profiles(path_labels(&profiles));
    ui.set_scripts(path_labels(&scripts));
    ui.set_profile_index(profile_index as i32);
    ui.set_script_index(script_index as i32);
    ui.set_adb_path(options.adb.to_string_lossy().as_ref().into());
    ui.set_offline(replay.is_some());
    ui.set_devices(strings(if replay.is_some() {
        vec!["离线回放 · 禁用输入".into()]
    } else {
        vec!["正在发现设备…".into()]
    }));
    ui.set_device_index(0);
    ui.set_backends(strings(if cfg!(feature = "maa") {
        vec!["Rust 识别".into(), "MaaFramework 识别".into()]
    } else {
        vec!["Rust 识别".into()]
    }));
    let data = root.join("data/atlas");
    ui.set_data_dir(
        if data.is_dir() {
            data
        } else {
            home.join("data/atlas")
        }
        .to_string_lossy()
        .as_ref()
        .into(),
    );
    let sdk = root.join(if cfg!(target_os = "windows") {
        ".tools/maa/sdk/bin/MaaFramework.dll"
    } else {
        ".tools/maa/sdk/bin/libMaaFramework.dylib"
    });
    if sdk.is_file() {
        ui.set_maa_library(sdk.to_string_lossy().as_ref().into());
    }
    let ocr = root.join("../MaaFgo/assets/resource/base/model/OCR");
    if ocr.is_dir() {
        ui.set_maa_ocr(ocr.to_string_lossy().as_ref().into());
    }
    let controller = Rc::new(RefCell::new(Controller {
        root,
        output,
        replay,
        profiles,
        scripts,
        devices: Vec::new(),
        job: None,
        pending: None,
        mailbox: Mailbox::default(),
        logs: VecDeque::new(),
        closing: false,
    }));
    {
        let weak = ui.as_weak();
        ui.on_select_quest(move |x, y| {
            if let Some(ui) = weak.upgrade()
                && !ui.get_running()
                && !ui.get_busy()
                && let Some(point) = preview_point(x, y)
            {
                ui.set_quest_x(point.x as i32);
                ui.set_quest_y(point.y as i32);
                ui.set_picking_quest(false);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_refresh_devices(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().discover(&ui, None);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_connect_device(move || {
            if let Some(ui) = weak.upgrade() {
                let address = ui.get_connection_address().trim().to_owned();
                if address.is_empty()
                    || address.starts_with('-')
                    || !address.contains(':')
                    || address.chars().any(char::is_whitespace)
                {
                    ui.set_error_message("请输入有效的 ADB 地址，例如 127.0.0.1:16384".into());
                } else {
                    state.borrow_mut().discover(&ui, Some(address));
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_device_changed(move || {
            if let Some(ui) = weak.upgrade() {
                state.borrow_mut().preview(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_start_run(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = state.borrow_mut();
                if state.closing
                    || state.pending.is_some()
                    || state
                        .job
                        .as_ref()
                        .is_some_and(|job| job.kind != JobKind::Preview)
                {
                    return;
                }
                match state.config(&ui) {
                    Ok(config) => {
                        ui.set_trace_path(config.trace.to_string_lossy().as_ref().into());
                        state.request(&ui, Request::Run(Box::new(config)));
                    }
                    Err(error) => {
                        state.error(&ui, format!("{error:#}"));
                    }
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_pause_resume(move || {
            if let Some(ui) = weak.upgrade() {
                let state = state.borrow();
                if let Some(job) = state.job.as_ref().filter(|job| job.kind == JobKind::Run) {
                    if job.control.is_pause_requested() {
                        job.control.resume();
                    } else {
                        job.control.pause();
                    }
                    ui.set_paused(job.control.is_pause_requested());
                }
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_stop_run(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state = state.borrow_mut();
                state.pending = None;
                if let Some(job) = &state.job {
                    job.control.stop();
                }
                ui.set_stopping(true);
                ui.set_status("正在停止…".into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        ui.on_clear_log(move || {
            state.borrow_mut().logs.clear();
            if let Some(ui) = weak.upgrade() {
                ui.set_logs(strings(Vec::new()));
            }
        });
    }
    {
        let state = controller.clone();
        ui.window().on_close_requested(move || {
            let mut state = state.borrow_mut();
            state.closing = true;
            state.pending = None;
            if let Some(job) = &state.job {
                job.control.stop();
                slint::CloseRequestResponse::KeepWindowShown
            } else {
                slint::CloseRequestResponse::HideWindow
            }
        });
    }
    let timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let state = controller.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                if let Some(ui) = weak.upgrade() {
                    state.borrow_mut().poll(&ui);
                }
            },
        );
    }
    if controller.borrow().replay.is_some() {
        controller.borrow_mut().preview(&ui);
    } else {
        controller.borrow_mut().discover(&ui, None);
    }
    let result = ui.run();
    timer.stop();
    // Event-loop failure/OS shutdown also cancels work. Joining is outside UI callbacks.
    if let Some(job) = controller.borrow_mut().job.take() {
        job.control.stop();
        let _ = job.worker.join();
    }
    result.context("desktop event loop")
}

impl Controller {
    fn adb(ui: &AppWindow) -> Adb {
        Adb {
            executable: PathBuf::from(ui.get_adb_path().as_str()),
            timeout: Duration::from_secs(15),
        }
    }

    fn discover(&mut self, ui: &AppWindow, address: Option<String>) {
        if self.replay.is_none() {
            self.request(
                ui,
                Request::Discover {
                    adb: Self::adb(ui),
                    address,
                },
            );
        }
    }

    fn selected_serial(&self, ui: &AppWindow) -> Result<Option<String>> {
        if self.replay.is_some() {
            return Ok(None);
        }
        let device = self
            .devices
            .get(usize::try_from(ui.get_device_index())?)
            .context("请先选择已连接的设备")?;
        ensure!(
            device.state == "device",
            "设备 {} 当前为 {}，请检查模拟器或 ADB 授权",
            device.serial,
            device.state
        );
        Ok(Some(device.serial.clone()))
    }

    fn preview(&mut self, ui: &AppWindow) {
        ui.set_picking_quest(false);
        ui.set_has_preview(false);
        match self.selected_serial(ui) {
            Ok(serial) => {
                ui.set_preview_info("正在获取画面…".into());
                self.request(
                    ui,
                    Request::Preview {
                        adb: Self::adb(ui),
                        serial,
                        replay: self.replay.clone(),
                    },
                );
            }
            Err(error) => self.error(ui, format!("{error:#}")),
        }
    }

    fn config(&self, ui: &AppWindow) -> Result<RunConfig> {
        let serial = self.selected_serial(ui)?;
        let fixed_farm = if ui.get_task_index() == 0 {
            Some(FixedFarmConfig {
                quest: Point {
                    x: u32::try_from(ui.get_quest_x())?,
                    y: u32::try_from(ui.get_quest_y())?,
                },
                team: u8::try_from(ui.get_team_index())?,
            })
        } else {
            None
        };
        // Fixed farming does not load these files; selection remains available
        // when switching back to the existing Profile/script mode.
        let script = self
            .scripts
            .get(usize::try_from(ui.get_script_index())?)
            .context("请选择脚本")?
            .clone();
        let profile = self
            .profiles
            .get(usize::try_from(ui.get_profile_index())?)
            .context("请选择 Profile")?
            .clone();
        let recognition = if ui.get_backend_index() == 1 {
            ensure!(cfg!(feature = "maa"), "当前构建没有 MaaFramework 支持");
            ensure!(
                !ui.get_maa_library().trim().is_empty(),
                "请选择 MaaFramework 动态库"
            );
            RecognitionConfig {
                library: Some(PathBuf::from(ui.get_maa_library().as_str())),
                resources: Vec::new(),
                ocr_model: (!ui.get_maa_ocr().trim().is_empty())
                    .then(|| PathBuf::from(ui.get_maa_ocr().as_str())),
            }
        } else {
            RecognitionConfig::default()
        };
        let region = match ui.get_region_index() {
            0 => Region::Cn,
            1 => Region::Jp,
            2 => Region::Na,
            3 => Region::Kr,
            4 => Region::Tw,
            _ => anyhow::bail!("无效区域"),
        };
        let apple = match ui.get_apple_index() {
            0 => Apple::Gold,
            1 => Apple::Silver,
            2 => Apple::Bronze,
            _ => anyhow::bail!("无效苹果类型"),
        };
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let folder = self
            .output
            .join(format!("run-{stamp}-{}", std::process::id()));
        let config = RunConfig {
            fixed_farm,
            adb: Self::adb(ui),
            script,
            serial,
            replay: self.replay.clone(),
            replay_trace: None,
            replay_compare: None,
            replay_report: None,
            execute: self.replay.is_none() && ui.get_mode_index() == 1,
            templates: self.root.join("assets/templates.json"),
            recognition,
            profile,
            region,
            data_dir: PathBuf::from(ui.get_data_dir().as_str()),
            viewport: None,
            max_seconds: u64::try_from(ui.get_max_seconds())?,
            max_operations: 1_000_000,
            trace: folder.join("trace.jsonl"),
            frames: Some(folder.join("frames")),
            battles: u32::try_from(ui.get_battles())?,
            apples: u32::try_from(ui.get_apples())?,
            apple,
        };
        config.validate()?;
        Ok(config)
    }

    fn request(&mut self, ui: &AppWindow, request: Request) {
        if self.closing || self.pending.is_some() {
            return;
        }
        ui.set_picking_quest(false);
        if let Some(job) = &self.job {
            if job.kind != JobKind::Preview {
                return;
            }
            job.control.stop();
            self.pending = Some(request);
            ui.set_busy(true);
            ui.set_status("切换任务中…".into());
        } else {
            self.spawn(ui, request);
        }
    }

    fn spawn(&mut self, ui: &AppWindow, request: Request) {
        let kind = match &request {
            Request::Discover { .. } => JobKind::Discover,
            Request::Preview { .. } => JobKind::Preview,
            Request::Run(_) => JobKind::Run,
        };
        let control = match &request {
            Request::Run(config) => config.control(),
            Request::Preview { .. } => Control::new(Duration::MAX),
            Request::Discover { .. } => Control::new(Duration::from_secs(40)),
        };
        self.mailbox = Mailbox::default();
        let mailbox = self.mailbox.clone();
        let worker_control = control.clone();
        let (sender, done) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                work(request, worker_control, mailbox)
            }));
            let result = match result {
                Ok(result) => result.map_err(|error| format!("{error:#}")),
                Err(_) => Err("后台线程异常退出；请查看运行日志".into()),
            };
            let _ = sender.send(result);
        });
        self.job = Some(Job {
            kind,
            control,
            done,
            worker,
        });
        ui.set_running(kind == JobKind::Run);
        ui.set_busy(kind == JobKind::Discover);
        ui.set_paused(false);
        ui.set_stopping(false);
        ui.set_error_message("".into());
        ui.set_status(
            match kind {
                JobKind::Discover => "发现设备…",
                JobKind::Preview => "预览中",
                JobKind::Run => "准备 / 运行中",
            }
            .into(),
        );
        if kind == JobKind::Run {
            ui.set_scene("未知".into());
            ui.set_wave("—".into());
            ui.set_turn("—".into());
            ui.set_numbers("HP / NP · 未请求识别".into());
            self.log(ui, "开始任务；完整 JSONL 与截图将写入运行目录".into());
        }
    }

    fn poll(&mut self, ui: &AppWindow) {
        let updates = self.mailbox.drain();
        if let Some(preview) = updates.frame {
            let frame = preview.frame;
            let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                frame.as_raw(),
                frame.width(),
                frame.height(),
            );
            ui.set_preview(slint::Image::from_rgba8(pixels));
            ui.set_has_preview(true);
            ui.set_preview_info(preview.description.into());
        }
        if let Some(view) = updates.view {
            if !view.scene.is_empty() {
                ui.set_scene(view.scene.into());
            }
            if !view.wave.is_empty() {
                ui.set_wave(view.wave.into());
            }
            if !view.turn.is_empty() {
                ui.set_turn(view.turn.into());
            }
            if !view.numbers.is_empty() {
                ui.set_numbers(view.numbers.into());
            }
        }
        if updates.omitted > 0 {
            self.logs.push_back(format!(
                "合并了 {} 条旧展示日志，完整记录见 JSONL",
                updates.omitted
            ));
        }
        if !updates.logs.is_empty() || updates.omitted > 0 {
            self.logs.extend(updates.logs);
            self.refresh_logs(ui);
        }
        if let Some(job) = &self.job
            && job.kind == JobKind::Run
        {
            ui.set_paused(job.control.is_pause_requested());
            ui.set_stopping(job.control.is_stop_requested());
            ui.set_status(
                if job.control.is_stop_requested() {
                    "正在停止…"
                } else if job.control.is_paused() && job.control.is_pause_requested() {
                    "已暂停"
                } else if job.control.is_pause_requested() {
                    "正在暂停…"
                } else if job.control.is_paused() {
                    "正在恢复…"
                } else {
                    "运行中"
                }
                .into(),
            );
        }
        let completed = self.job.as_ref().and_then(|job| match job.done.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("后台通道已关闭".into())),
            Err(mpsc::TryRecvError::Empty) => None,
        });
        if let Some(result) = completed {
            let job = self.job.take().expect("completed job");
            let stopped = job.control.is_stop_requested();
            ui.set_busy(false);
            ui.set_running(false);
            ui.set_paused(false);
            ui.set_stopping(false);
            if self.closing {
                let _ = slint::quit_event_loop();
                return;
            }
            if let Some(next) = self.pending.take() {
                self.spawn(ui, next);
                return;
            }
            match result {
                Ok(JobResult::Devices(devices)) => {
                    let previous = self.selected_serial(ui).ok().flatten();
                    self.devices = devices;
                    let index = self
                        .devices
                        .iter()
                        .position(|d| previous.as_ref() == Some(&d.serial))
                        .or_else(|| self.devices.iter().position(|d| d.state == "device"));
                    ui.set_devices(strings(if self.devices.is_empty() {
                        vec!["未发现设备".into()]
                    } else {
                        self.devices
                            .iter()
                            .map(|d| format!("{} · {}", d.serial, d.state))
                            .collect()
                    }));
                    ui.set_device_index(index.map(|i| i as i32).unwrap_or(-1));
                    if index.is_some() {
                        self.preview(ui);
                    } else {
                        ui.set_has_preview(false);
                        ui.set_preview_info("连接设备后获取截图".into());
                        ui.set_status("无可用设备".into());
                    }
                }
                Ok(JobResult::Preview) => {
                    ui.set_status("回放就绪".into());
                }
                Ok(JobResult::Run) => {
                    ui.set_status("已完成".into());
                    self.log(ui, "任务已完成".into());
                }
                Err(_) if stopped => {
                    ui.set_status("已停止".into());
                    self.log(ui, "任务已停止".into());
                }
                Err(error) => self.error(ui, error),
            }
        }
    }

    fn error(&mut self, ui: &AppWindow, message: String) {
        ui.set_status("发生错误".into());
        ui.set_error_message(message.chars().take(2000).collect::<String>().into());
        self.log(ui, message);
    }
    fn log(&mut self, ui: &AppWindow, message: String) {
        self.logs.push_back(message.chars().take(2000).collect());
        self.refresh_logs(ui);
    }
    fn refresh_logs(&mut self, ui: &AppWindow) {
        while self.logs.len() > LOG_LIMIT {
            self.logs.pop_front();
        }
        ui.set_logs(strings(self.logs.iter().rev().cloned().collect()));
    }
}

fn work(request: Request, control: Control, mailbox: Mailbox) -> Result<JobResult> {
    match request {
        Request::Discover { adb, address } => {
            if let Some(address) = address {
                let output = adb.command(None, &["connect", &address], &control)?;
                let output = String::from_utf8_lossy(&output);
                ensure!(
                    output.contains("connected to"),
                    "ADB 连接失败：{}",
                    output.trim()
                );
                mailbox.log(output.trim().into());
            }
            Ok(JobResult::Devices(adb.devices(&control)?))
        }
        Request::Preview {
            adb,
            serial,
            replay,
        } => {
            if let Some(replay) = replay {
                let raw = ReplayDevice::new(&replay)?.capture(&control)?;
                let (frame, _) = vision::normalize(&raw, None)?;
                mailbox.frame(
                    &frame,
                    format!(
                        "离线截图 · 1280 × 720（原始 {} × {}）",
                        raw.width(),
                        raw.height()
                    ),
                );
                return Ok(JobResult::Preview);
            }
            let mut device = AdbDevice {
                adb,
                serial: serial.context("missing preview device")?,
            };
            loop {
                let raw = device.capture(&control)?;
                let (frame, _) = vision::normalize(&raw, None)?;
                mailbox.frame(
                    &frame,
                    format!(
                        "设备截图 · 1280 × 720（原始 {} × {}）",
                        raw.width(),
                        raw.height()
                    ),
                );
                control.sleep(Duration::from_millis(500))?;
            }
        }
        Request::Run(config) => {
            let hooks = RunHooks {
                events: Some(Box::new(UiObserver::new(mailbox.clone()))),
                frames: Some(Box::new(UiFrames::new(mailbox))),
            };
            runner::prepare(*config, control, hooks)?.run()?;
            Ok(JobResult::Run)
        }
    }
}

fn preview_point(x: f32, y: f32) -> Option<Point> {
    (x.is_finite() && y.is_finite() && (0.0..1.0).contains(&x) && (0.0..1.0).contains(&y))
        .then_some(Point {
            x: (x * 1280.0) as u32,
            y: (y * 720.0) as u32,
        })
}

fn strings(values: Vec<String>) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        values
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
    .into()
}
fn path_labels(paths: &[PathBuf]) -> ModelRc<SharedString> {
    strings(
        paths
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
    )
}
fn absolute(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn choices(
    folder: &Path,
    extension: &str,
    extra: Option<&Path>,
    preferred: &str,
) -> Result<(Vec<PathBuf>, usize)> {
    let mut paths = std::fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.retain(|path| path.is_file() && path.extension().is_some_and(|ext| ext == extension));
    paths.sort();
    let selected = if let Some(extra) = extra {
        let extra = extra
            .canonicalize()
            .with_context(|| format!("read {}", extra.display()))?;
        if let Some(index) = paths.iter().position(|path| path == &extra) {
            index
        } else {
            paths.push(extra);
            paths.len() - 1
        }
    } else {
        paths
            .iter()
            .position(|path| path.file_name().is_some_and(|name| name == preferred))
            .unwrap_or(0)
    };
    ensure!(
        !paths.is_empty(),
        "no {extension} files in {}",
        folder.display()
    );
    Ok((paths, selected))
}

fn resource_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(root) = explicit {
        let root = root.canonicalize()?;
        ensure!(
            valid_root(&root),
            "资源目录必须包含 assets/templates.json、profiles/ 和 scripts/"
        );
        return Ok(root);
    }
    let exe = std::env::current_exe()?;
    let parent = exe.parent().context("executable parent")?;
    let mut candidates = vec![
        parent.join("resources"),
        parent.join("../Resources"),
        std::env::current_dir()?,
    ];
    if cfg!(debug_assertions) {
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    }
    candidates
        .into_iter()
        .find(|path| valid_root(path))
        .context("找不到资源；请使用 --resources 指定包含 assets/、profiles/、scripts/ 的目录")?
        .canonicalize()
        .context("resolve resources")
}
fn valid_root(root: &Path) -> bool {
    root.join("assets/templates.json").is_file()
        && root.join("profiles").is_dir()
        && root.join("scripts").is_dir()
}

fn user_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        Ok(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA missing")?)
                .join("FGO-rs"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("HOME missing")?)
                .join("Library/Application Support/FGO-rs"),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let base = match std::env::var_os("XDG_DATA_HOME") {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(std::env::var_os("HOME").context("HOME missing")?)
                .join(".local/share"),
        };
        Ok(base.join("FGO-rs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_selection_maps_to_normalized_pixels_and_rejects_outside_image() {
        assert_eq!(preview_point(0.5, 0.5), Some(Point { x: 640, y: 360 }));
        assert_eq!(
            preview_point(845.5 / 1280.0, 203.5 / 720.0),
            Some(Point { x: 845, y: 203 })
        );
        assert_eq!(preview_point(0.0, 0.0), Some(Point { x: 0, y: 0 }));
        for (x, y) in [(1.0, 0.5), (-0.1, 0.5), (0.5, 1.0), (0.5, f32::NAN)] {
            assert_eq!(preview_point(x, y), None);
        }
    }
    #[test]
    fn external_script_with_spaces_is_preserved() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("default.rhai"), "").unwrap();
        let external = tempfile::tempdir().unwrap();
        let path = external.path().join("中文 strategy.rhai");
        std::fs::write(&path, "").unwrap();
        let (paths, selected) =
            choices(folder.path(), "rhai", Some(&path), "default.rhai").unwrap();
        assert_eq!(paths[selected], path.canonicalize().unwrap());
    }
}
