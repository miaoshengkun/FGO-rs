use crate::control::Control;
use anyhow::{Context, Result, bail, ensure};
use image::{RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

/// Pixel rectangle: x, y, width, height. All script coordinates are 1280x720.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn validate(self, width: u32, height: u32) -> Result<()> {
        ensure!(self.width > 0 && self.height > 0, "empty rectangle");
        ensure!(
            self.x.checked_add(self.width).is_some_and(|x| x <= width)
                && self.y.checked_add(self.height).is_some_and(|y| y <= height),
            "rectangle outside image"
        );
        Ok(())
    }

    pub fn device_point(self, p: Point) -> Result<Point> {
        ensure!(p.x < WIDTH && p.y < HEIGHT, "point outside 1280x720 canvas");
        Ok(Point {
            x: self.x
                + ((p.x as f64 * self.width as f64 / WIDTH as f64).round() as u32)
                    .min(self.width - 1),
            y: self.y
                + ((p.y as f64 * self.height as f64 / HEIGHT as f64).round() as u32)
                    .min(self.height - 1),
        })
    }
}

pub fn normalize(raw: &RgbaImage, viewport: Option<Rect>) -> Result<(RgbaImage, Rect)> {
    let (w, h) = raw.dimensions();
    ensure!(w > 0 && h > 0, "empty screenshot");
    let rect = match viewport {
        Some(rect) => rect,
        None => {
            ensure!(
                w >= h,
                "portrait screenshot: rotate the game to landscape or set --viewport"
            );
            let (cw, ch) = if w as u64 * 9 > h as u64 * 16 {
                (h * 16 / 9, h)
            } else {
                (w, w * 9 / 16)
            };
            Rect {
                x: (w - cw) / 2,
                y: (h - ch) / 2,
                width: cw,
                height: ch,
            }
        }
    };
    rect.validate(w, h)?;
    let cropped = imageops::crop_imm(raw, rect.x, rect.y, rect.width, rect.height).to_image();
    let normalized = if cropped.dimensions() == (WIDTH, HEIGHT) {
        cropped
    } else {
        imageops::resize(&cropped, WIDTH, HEIGHT, imageops::FilterType::CatmullRom)
    };
    Ok((normalized, rect))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSpec {
    pub file: PathBuf,
    pub region: Rect,
    pub threshold: f64,
}

struct Template {
    spec: TemplateSpec,
    width: u32,
    height: u32,
    // Alpha > 0 means included, matching an OpenCV uint8 mask.
    samples: Vec<(u32, u32, [f64; 3])>,
    energy: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Match {
    pub x: u32,
    pub y: u32,
    pub score: f64,
}

pub struct Vision {
    templates: HashMap<String, Template>,
}

impl Vision {
    pub fn has_template(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }
    pub fn load(manifest: &Path) -> Result<Self> {
        let specs: HashMap<String, TemplateSpec> = serde_json::from_slice(
            &std::fs::read(manifest).with_context(|| format!("read {}", manifest.display()))?,
        )
        .context("parse template manifest")?;
        let base = manifest.parent().unwrap_or(Path::new("."));
        let mut templates = HashMap::new();
        for (name, spec) in specs {
            spec.region.validate(WIDTH, HEIGHT)?;
            ensure!(
                spec.threshold.is_finite() && spec.threshold > 0.0 && spec.threshold <= 1.0,
                "invalid threshold for {name}"
            );
            let path = base.join(&spec.file);
            let img = image::open(&path)
                .with_context(|| format!("load {}", path.display()))?
                .to_rgba8();
            let (width, height) = img.dimensions();
            ensure!(
                width <= spec.region.width && height <= spec.region.height,
                "template {name} exceeds region"
            );
            let samples: Vec<_> = img
                .enumerate_pixels()
                .filter(|(_, _, p)| p[3] > 0)
                .map(|(x, y, p)| (x, y, [p[0] as f64, p[1] as f64, p[2] as f64]))
                .collect();
            let energy = samples.iter().flat_map(|s| s.2).map(|v| v * v).sum::<f64>();
            ensure!(
                energy > 0.0,
                "template {name} has zero energy or empty mask"
            );
            templates.insert(
                name,
                Template {
                    spec,
                    width,
                    height,
                    samples,
                    energy,
                },
            );
        }
        Ok(Self { templates })
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.templates.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn find(&self, frame: &RgbaImage, name: &str, control: &Control) -> Result<Option<Match>> {
        self.find_in(frame, name, None, control)
    }

    /// A limited SQDIFF_NORMED reference matcher. Rejected candidates may short-circuit;
    /// accepted candidates always use the exact full RGB squared difference.
    pub fn find_in(
        &self,
        frame: &RgbaImage,
        name: &str,
        region: Option<Rect>,
        control: &Control,
    ) -> Result<Option<Match>> {
        let template = self
            .templates
            .get(name)
            .with_context(|| format!("unknown template: {name}"))?;
        let region = region.unwrap_or(template.spec.region);
        region.validate(frame.width(), frame.height())?;
        ensure!(
            template.width <= region.width && template.height <= region.height,
            "template exceeds search region"
        );
        let mut best = template.spec.threshold;
        let mut result = None;
        let max_denominator =
            (template.energy * template.samples.len() as f64 * 3.0 * 255.0 * 255.0).sqrt();
        for y in region.y..=region.y + region.height - template.height {
            control.checkpoint()?;
            for x in region.x..=region.x + region.width - template.width {
                let mut difference = 0.0;
                let mut energy = 0.0;
                let cutoff = best * max_denominator;
                for &(tx, ty, values) in &template.samples {
                    let p = frame.get_pixel(x + tx, y + ty);
                    for c in 0..3 {
                        let value = p[c] as f64;
                        difference += (value - values[c]).powi(2);
                        energy += value * value;
                    }
                    if difference > cutoff {
                        break;
                    }
                }
                if difference > cutoff || energy == 0.0 {
                    continue;
                }
                let score = (difference / (template.energy * energy).sqrt()).min(1.0);
                if score < best {
                    best = score;
                    result = Some(Match {
                        x: x + template.width / 2,
                        y: y + template.height / 2,
                        score,
                    });
                }
            }
        }
        Ok(result)
    }

    pub fn support(&self, frame: &RgbaImage, control: &Control) -> Result<Option<Match>> {
        // Split the search above/below each match, suppressing duplicate detections
        // within 20 pixels. This also finds weaker friend rows above a stronger
        // non-friend match. Return the uppermost complete friend row.
        let height = self
            .templates
            .get("level120")
            .context("missing level120 template")?
            .height;
        let mut ranges = vec![(175, 690)];
        let mut candidates = Vec::new();
        while let Some((top, bottom)) = ranges.pop() {
            if bottom <= top || bottom - top < height {
                continue;
            }
            let region = Rect {
                x: 65,
                y: top,
                width: 153,
                height: bottom - top,
            };
            let Some(level) = self.find_in(frame, "level120", Some(region), control)? else {
                continue;
            };
            let start = level.y - height / 2;
            let upper_end = (start + height).saturating_sub(20);
            if upper_end > top {
                ranges.push((top, upper_end));
            }
            let lower_start = start + 20;
            if lower_start < bottom {
                ranges.push((lower_start, bottom));
            }
            if level.y + 155 < HEIGHT
                && self
                    .find_in(
                        frame,
                        "friend",
                        Some(Rect {
                            x: 1100,
                            y: level.y + 80,
                            width: 90,
                            height: 75,
                        }),
                        control,
                    )?
                    .is_some()
            {
                candidates.push(Match {
                    x: 800,
                    y: level.y + 55,
                    score: level.score,
                });
            }
        }
        Ok(candidates.into_iter().min_by_key(|m| m.y))
    }

    pub fn require_canvas(frame: &RgbaImage) -> Result<()> {
        if frame.dimensions() != (WIDTH, HEIGHT) {
            bail!("expected normalized 1280x720 frame");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fixture(samples: Vec<(u32, u32, [f64; 3])>) -> Vision {
        let energy = samples.iter().flat_map(|s| s.2).map(|v| v * v).sum();
        Vision {
            templates: HashMap::from([(
                "test".into(),
                Template {
                    spec: TemplateSpec {
                        file: PathBuf::new(),
                        region: Rect {
                            x: 0,
                            y: 0,
                            width: 2,
                            height: 1,
                        },
                        threshold: 0.02,
                    },
                    width: 2,
                    height: 1,
                    samples,
                    energy,
                },
            )]),
        }
    }

    #[test]
    fn masked_out_pixels_do_not_change_the_score() {
        let vision = fixture(vec![(0, 0, [100.0; 3])]);
        let mut frame = RgbaImage::new(2, 1);
        frame.put_pixel(0, 0, image::Rgba([100, 100, 100, 255]));
        frame.put_pixel(1, 0, image::Rgba([255, 0, 255, 255]));
        let found = vision
            .find(&frame, "test", &Control::new(Duration::from_secs(1)))
            .unwrap()
            .unwrap();
        assert_eq!(found.score, 0.0);
    }

    #[test]
    fn normalized_squared_difference_uses_both_image_energies() {
        let vision = fixture(vec![(0, 0, [100.0; 3])]);
        let mut frame = RgbaImage::new(2, 1);
        frame.put_pixel(0, 0, image::Rgba([90, 90, 90, 255]));
        let found = vision
            .find(&frame, "test", &Control::new(Duration::from_secs(1)))
            .unwrap()
            .unwrap();
        assert!((found.score - 1.0 / 90.0).abs() < 1e-12);
        frame.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        assert!(
            vision
                .find(&frame, "test", &Control::new(Duration::from_secs(1)))
                .unwrap()
                .is_none()
        );
    }
}
