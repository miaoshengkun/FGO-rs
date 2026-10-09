use crate::{adb::Adb, control::Control, vision::Point};
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Produces screenshots. Kept separate from input so deployments can combine
/// different capture and control transports.
pub trait FrameSource: Send {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage>;
}

/// Sends normalized device input.
pub trait InputSink: Send {
    fn tap(&mut self, point: Point, control: &Control) -> Result<()>;
    fn swipe(
        &mut self,
        from: Point,
        to: Point,
        duration: Duration,
        control: &Control,
    ) -> Result<()>;
}

/// Combined device contract used by the existing host constructor.
pub trait Device: FrameSource + InputSink {}
impl<T: FrameSource + InputSink + ?Sized> Device for T {}

/// Combines independently selected screenshot and input implementations.
pub struct SplitDevice {
    frames: Box<dyn FrameSource>,
    input: Box<dyn InputSink>,
}

impl SplitDevice {
    pub fn new(frames: Box<dyn FrameSource>, input: Box<dyn InputSink>) -> Self {
        Self { frames, input }
    }
}

impl FrameSource for SplitDevice {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage> {
        self.frames.capture(control)
    }
}

impl InputSink for SplitDevice {
    fn tap(&mut self, point: Point, control: &Control) -> Result<()> {
        self.input.tap(point, control)
    }

    fn swipe(
        &mut self,
        from: Point,
        to: Point,
        duration: Duration,
        control: &Control,
    ) -> Result<()> {
        self.input.swipe(from, to, duration, control)
    }
}

pub struct AdbDevice {
    pub adb: Adb,
    pub serial: String,
}

impl FrameSource for AdbDevice {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage> {
        let bytes = self.adb.command(
            Some(&self.serial),
            &["exec-out", "screencap", "-p"],
            control,
        )?;
        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .context("decode ADB PNG screenshot")
            .map(|img| img.to_rgba8())
    }
}

impl InputSink for AdbDevice {
    fn tap(&mut self, point: Point, control: &Control) -> Result<()> {
        self.adb.command(
            Some(&self.serial),
            &[
                "shell",
                "input",
                "tap",
                &point.x.to_string(),
                &point.y.to_string(),
            ],
            control,
        )?;
        Ok(())
    }

    fn swipe(
        &mut self,
        from: Point,
        to: Point,
        duration: Duration,
        control: &Control,
    ) -> Result<()> {
        self.adb.command(
            Some(&self.serial),
            &[
                "shell",
                "input",
                "swipe",
                &from.x.to_string(),
                &from.y.to_string(),
                &to.x.to_string(),
                &to.y.to_string(),
                &duration.as_millis().to_string(),
            ],
            control,
        )?;
        Ok(())
    }
}

pub struct ReplayDevice {
    paths: Vec<PathBuf>,
    timestamps: Option<Vec<Duration>>,
    index: usize,
}

impl ReplayDevice {
    pub fn new(path: &Path) -> Result<Self> {
        let mut paths = if path.is_dir() {
            let entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries
                .into_iter()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("png")))
                .collect()
        } else {
            vec![path.to_path_buf()]
        };
        // step2.png precedes step10.png; a numeric prefix (0001.png) is preferred.
        paths.sort_by_key(|p| {
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            let prefix = stem
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .to_owned();
            let number = stem[prefix.len()..].parse::<u64>().unwrap_or(0);
            (prefix, number, p.clone())
        });
        ensure!(!paths.is_empty(), "no PNG frames in {}", path.display());
        Ok(Self {
            paths,
            timestamps: None,
            index: 0,
        })
    }

    pub fn with_timestamps(path: &Path, timestamps: Vec<Duration>) -> Result<Self> {
        let mut replay = Self::new(path)?;
        ensure!(
            replay.paths.len() == timestamps.len(),
            "replay has {} PNG frame(s), but trace has {} capture event(s)",
            replay.paths.len(),
            timestamps.len()
        );
        ensure!(
            timestamps.windows(2).all(|pair| pair[0] <= pair[1]),
            "replay capture timestamps are not monotonic"
        );
        replay.timestamps = Some(timestamps);
        Ok(replay)
    }
}

impl FrameSource for ReplayDevice {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage> {
        control.checkpoint()?;
        if let Some(timestamp) = self
            .timestamps
            .as_ref()
            .and_then(|times| times.get(self.index))
        {
            control.advance_to(*timestamp)?;
        }
        let path = self
            .paths
            .get(self.index)
            .context("replay exhausted (one PNG per capture call)")?;
        let frame = image::open(path)
            .with_context(|| format!("load replay frame {}", path.display()))?
            .to_rgba8();
        self.index += 1;
        Ok(frame)
    }
}

impl InputSink for ReplayDevice {
    fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
        bail!("replay cannot send device input")
    }
    fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
        bail!("replay cannot send device input")
    }
}
