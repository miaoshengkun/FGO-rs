//! Shared CLI/desktop startup and execution. No UI types or event loop live here.
use crate::{
    adb::Adb,
    battle::Color,
    catalog::{CardType, PartyServantData, Region, SupportServantData, load_profile_servant_data},
    control::{Control, ControlError},
    device::{AdbDevice, Device, ReplayDevice},
    farming::{Apple, FixedFarmConfig},
    profile::PartyProfile,
    replay::{
        CapturedReplayEvents, ReplayComparisonMode, ReplayEventSink, ReplayTrace,
        require_matching_replay,
    },
    runtime::{EventSink, FrameSink, Host, JsonlEventSink, Runtime, RuntimeEvent},
    vision::{Rect, Vision},
};
use anyhow::{Context, Result, ensure};
use std::{
    io::BufWriter,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Default)]
pub struct RecognitionConfig {
    pub library: Option<PathBuf>,
    pub resources: Vec<PathBuf>,
    pub ocr_model: Option<PathBuf>,
}

impl RecognitionConfig {
    pub fn load(&self, manifest: &Path) -> Result<Vision> {
        let vision = Vision::load(manifest)?;
        let Some(library) = self.library.as_deref() else {
            ensure!(
                self.resources.is_empty() && self.ocr_model.is_none(),
                "Maa resources require a MaaFramework library"
            );
            return Ok(vision);
        };
        #[cfg(feature = "maa")]
        {
            let mut bundles = vec![manifest.parent().unwrap_or(Path::new(".")).join("maa")];
            bundles.extend_from_slice(&self.resources);
            let backend =
                crate::maa::MaaRecognizer::new(library, &bundles, self.ocr_model.as_deref())?;
            let mut vision = vision;
            vision.set_maa_backend(backend)?;
            Ok(vision)
        }
        #[cfg(not(feature = "maa"))]
        {
            let _ = library;
            anyhow::bail!(
                "MaaFramework support is disabled; build with cargo build --features maa"
            );
        }
    }
}

pub fn configure_party_vision(
    vision: &mut Vision,
    profile: &PartyProfile,
    data_dir: &Path,
    region: Region,
) -> Result<(Vec<PartyServantData>, Vec<SupportServantData>)> {
    let (party, support) = load_profile_servant_data(data_dir, region, profile)?;
    vision.configure_card_identities(
        party
            .iter()
            .map(|servant| (servant.slot, servant.command_card_templates.clone()))
            .collect(),
        Some(profile.support_party_slot()),
    )?;
    vision.configure_support_identities(
        support
            .iter()
            .map(|servant| (servant.atlas_id, servant.command_card_templates.clone()))
            .collect(),
    )?;
    Ok((party, support))
}

#[derive(Clone)]
pub struct RunConfig {
    pub fixed_farm: Option<FixedFarmConfig>,
    pub adb: Adb,
    pub script: PathBuf,
    pub serial: Option<String>,
    pub replay: Option<PathBuf>,
    pub replay_trace: Option<PathBuf>,
    pub replay_compare: Option<ReplayComparisonMode>,
    pub replay_report: Option<PathBuf>,
    pub execute: bool,
    pub templates: PathBuf,
    pub recognition: RecognitionConfig,
    pub profile: PathBuf,
    pub region: Region,
    pub data_dir: PathBuf,
    pub viewport: Option<Rect>,
    pub max_seconds: u64,
    pub max_operations: u64,
    pub trace: PathBuf,
    pub frames: Option<PathBuf>,
    pub battles: u32,
    pub apples: u32,
    pub apple: Apple,
}

impl RunConfig {
    pub fn control(&self) -> Control {
        let limit = execution_limit(self.max_seconds);
        if self.replay.is_some() {
            Control::simulated(limit)
        } else {
            Control::new(limit)
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(config) = self.fixed_farm {
            config.validate()?;
            ensure!(
                self.region == Region::Cn,
                "fixed-position farming currently requires the CN interface"
            );
        }
        ensure!(
            self.serial.is_some() != self.replay.is_some(),
            "select exactly one device or replay source"
        );
        if let Some(serial) = &self.serial {
            ensure!(
                !serial.is_empty() && !serial.starts_with('-'),
                "invalid device serial"
            );
        }
        ensure!(
            !self.execute || self.replay.is_none(),
            "replay cannot execute device input"
        );
        ensure!(
            self.replay.is_some()
                || (self.replay_trace.is_none()
                    && self.replay_compare.is_none()
                    && self.replay_report.is_none()),
            "replay options require a replay source"
        );
        ensure!(
            self.replay_trace.is_some()
                || (self.replay_compare.is_none() && self.replay_report.is_none()),
            "comparison options require a replay trace"
        );
        ensure!(self.max_operations > 0, "invalid execution limits");
        if let Some(baseline) = &self.replay_trace {
            ensure!(
                baseline != &self.trace,
                "--trace must differ from --replay-trace so the baseline is not modified"
            );
        }
        if let Some(report) = &self.replay_report {
            ensure!(
                report != &self.trace && self.replay_trace.as_ref() != Some(report),
                "--replay-report must differ from both trace paths"
            );
        }
        Ok(())
    }
}

/// A zero run budget disables only the overall deadline. Individual action
/// timeouts and explicit cancellation still apply.
pub fn execution_limit(seconds: u64) -> Duration {
    if seconds == 0 {
        Duration::MAX
    } else {
        Duration::from_secs(seconds)
    }
}

/// A presentation-only observer. Implementations must not block or change execution.
pub trait EventObserver: Send {
    fn observe(&mut self, event: &RuntimeEvent);
}

#[derive(Default)]
pub struct RunHooks {
    pub events: Option<Box<dyn EventObserver>>,
    pub frames: Option<Box<dyn FrameSink>>,
}

struct ObservedSink {
    inner: Box<dyn EventSink>,
    observer: Box<dyn EventObserver>,
}
impl EventSink for ObservedSink {
    fn emit(&mut self, event: RuntimeEvent) -> Result<()> {
        self.observer.observe(&event);
        self.inner.emit(event)
    }
}

pub struct PreparedRun {
    runtime: Runtime,
    source: Option<String>,
    config: RunConfig,
    baseline: Option<ReplayTrace>,
    captured: Option<CapturedReplayEvents>,
}

pub fn prepare(config: RunConfig, control: Control, hooks: RunHooks) -> Result<PreparedRun> {
    config.validate()?;
    control.checkpoint()?;
    let source = if config.fixed_farm.is_some() {
        None
    } else {
        let source = std::fs::read_to_string(&config.script)
            .with_context(|| format!("read {}", config.script.display()))?;
        Runtime::check(&source)?;
        Some(source)
    };
    let profile = if config.fixed_farm.is_some() {
        None
    } else {
        Some(PartyProfile::load(&config.profile)?)
    };
    let mut vision = config.recognition.load(&config.templates)?;
    control.checkpoint()?;
    let (party_data, support_data) = if let Some(profile) = &profile {
        configure_party_vision(&mut vision, profile, &config.data_dir, config.region)?
    } else {
        (Vec::new(), Vec::new())
    };
    let baseline = config
        .replay_trace
        .as_deref()
        .map(ReplayTrace::load)
        .transpose()?;
    let device: Box<dyn Device> = if let Some(path) = &config.replay {
        Box::new(if let Some(baseline) = &baseline {
            ReplayDevice::with_timestamps(path, baseline.capture_timestamps())?
        } else {
            ReplayDevice::new(path)?
        })
    } else {
        Box::new(AdbDevice {
            adb: config.adb.clone(),
            serial: config.serial.clone().context("missing device serial")?,
        })
    };
    parent_dir(&config.trace)?;
    let output = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.trace)?;
    let (mut sink, captured): (Box<dyn EventSink>, _) = if baseline.is_some() {
        let (events, captured) = ReplayEventSink::new(Some(Box::new(BufWriter::new(output))));
        (Box::new(events), Some(captured))
    } else {
        (
            Box::new(JsonlEventSink::new(Some(Box::new(BufWriter::new(output))))),
            None,
        )
    };
    if let Some(observer) = hooks.events {
        sink = Box::new(ObservedSink {
            inner: sink,
            observer,
        });
    }
    let mut host = Host::with_event_sink(
        device,
        vision,
        config.viewport,
        config.execute,
        sink,
        control,
    );
    if let Some(profile) = profile {
        host.configure_profile(profile)?;
    }
    if let Some(fixed) = config.fixed_farm {
        host.configure_fixed_farm(fixed)?;
    }
    let color = |card| match card {
        CardType::Arts => Color::Arts,
        CardType::Buster => Color::Buster,
        CardType::Quick => Color::Quick,
    };
    host.configure_noble_phantasm_colors(
        party_data
            .iter()
            .map(|servant| (servant.slot, color(servant.noble_phantasm_card))),
    )?;
    host.configure_atlas_noble_phantasm_colors(
        support_data
            .iter()
            .map(|servant| (servant.atlas_id, color(servant.noble_phantasm_card))),
    )?;
    if let Some(path) = &config.frames {
        host.record_frames(path.clone())?;
    }
    if let Some(frames) = hooks.frames {
        host.set_frame_sink(frames);
    }
    host.configure_farm(config.battles, config.apples, config.apple);
    let runtime = Runtime::new(host, config.max_operations);
    Ok(PreparedRun {
        runtime,
        source,
        config,
        baseline,
        captured,
    })
}

impl PreparedRun {
    pub fn run(self) -> Result<()> {
        let result = match &self.source {
            Some(source) => self.runtime.run(source),
            None => self.runtime.run_default_farm(),
        };
        if let (Some(baseline), Some(captured)) = (&self.baseline, &self.captured) {
            let report = baseline.compare(
                &captured.snapshot()?,
                self.config
                    .replay_compare
                    .unwrap_or(ReplayComparisonMode::Events),
            );
            let json = serde_json::to_string_pretty(&report)?;
            if let Some(path) = &self.config.replay_report {
                parent_dir(path)?;
                std::fs::write(path, format!("{json}\n"))?;
                println!("{}", path.display());
            } else {
                println!("{json}");
            }
            require_matching_replay(&report)?;
        }
        if let Err(error) = result {
            if error.is::<ControlError>() {
                return Err(error).context("run stopped");
            }
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis();
            let diagnostic = self
                .config
                .trace
                .with_file_name(format!("failure-{stamp}.png"));
            if self.runtime.save_last_frame(&diagnostic).is_ok() {
                eprintln!("Last observed frame: {}", diagnostic.display());
                return Err(error).with_context(|| {
                    format!("run failed; last observed frame: {}", diagnostic.display())
                });
            }
            return Err(error).context("run failed");
        }
        Ok(())
    }
}

pub fn parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
