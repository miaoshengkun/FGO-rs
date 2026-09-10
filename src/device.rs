use crate::{adb::Adb, control::Control, vision::Point};
use anyhow::{Context, Result, bail, ensure};
use image::RgbaImage;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub trait Device: Send {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage>;
    fn tap(&mut self, point: Point, control: &Control) -> Result<()>;
    fn swipe(
        &mut self,
        from: Point,
        to: Point,
        duration: Duration,
        control: &Control,
    ) -> Result<()>;
}

pub struct AdbDevice {
    pub adb: Adb,
    pub serial: String,
}

impl Device for AdbDevice {
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
        Ok(Self { paths, index: 0 })
    }
}

impl Device for ReplayDevice {
    fn capture(&mut self, control: &Control) -> Result<RgbaImage> {
        control.checkpoint()?;
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
    fn tap(&mut self, _: Point, _: &Control) -> Result<()> {
        bail!("replay cannot send device input")
    }
    fn swipe(&mut self, _: Point, _: Point, _: Duration, _: &Control) -> Result<()> {
        bail!("replay cannot send device input")
    }
}
