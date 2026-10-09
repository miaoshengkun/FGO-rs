use crate::observation::{Recognition, UnknownReason};
use crate::{control::Control, domain::PartySlot, layout::UI_LAYOUT_V1};
use anyhow::{Context, Result, bail, ensure};
use image::{RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

/// Pixel rectangle: x, y, width, height. All script coordinates are 1280x720.
#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
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
    pixels: Arc<TemplatePixels>,
}

struct TemplatePixels {
    width: u32,
    height: u32,
    // Alpha > 0 means included, matching an OpenCV uint8 mask.
    samples: Vec<(u32, u32, [f64; 3])>,
    energy: f64,
}

fn template_pixels(image: &RgbaImage) -> TemplatePixels {
    let (width, height) = image.dimensions();
    let samples: Vec<_> = image
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] > 0)
        .map(|(x, y, pixel)| (x, y, [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64]))
        .collect();
    let energy = samples
        .iter()
        .flat_map(|sample| sample.2)
        .map(|value| value * value)
        .sum::<f64>();
    TemplatePixels {
        width,
        height,
        samples,
        energy,
    }
}

fn best_template_score(
    frame: &RgbaImage,
    region: Rect,
    pixels: &TemplatePixels,
    limit: f64,
    control: &Control,
) -> Result<Option<f64>> {
    region.validate(frame.width(), frame.height())?;
    ensure!(
        pixels.width <= region.width && pixels.height <= region.height,
        "card identity template exceeds search region"
    );
    let mut best = limit;
    let max_denominator =
        (pixels.energy * pixels.samples.len() as f64 * 3.0 * 255.0 * 255.0).sqrt();
    for y in region.y..=region.y + region.height - pixels.height {
        control.checkpoint()?;
        for x in region.x..=region.x + region.width - pixels.width {
            let mut difference = 0.0;
            let mut energy = 0.0;
            let cutoff = best * max_denominator;
            for &(tx, ty, values) in &pixels.samples {
                let pixel = frame.get_pixel(x + tx, y + ty);
                for channel in 0..3 {
                    let value = pixel[channel] as f64;
                    difference += (value - values[channel]).powi(2);
                    energy += value * value;
                }
                if difference > cutoff {
                    break;
                }
            }
            if difference > cutoff || energy == 0.0 {
                continue;
            }
            let score = (difference / (pixels.energy * energy).sqrt()).min(1.0);
            if score < best {
                best = score;
            }
        }
    }
    Ok((best < limit).then_some(best))
}

const IDENTITY_MATCH_THRESHOLD: f64 = 0.11;
const IDENTITY_AMBIGUITY_MARGIN: f64 = 0.005;

fn match_identity_candidates<'a, T: Copy>(
    frame: &RgbaImage,
    region: Rect,
    identities: impl Iterator<Item = (T, &'a [Arc<TemplatePixels>])>,
    control: &Control,
) -> Result<Recognition<T>> {
    // A runner-up above the acceptance threshold can still be ambiguous.
    // Scores beyond this bound cannot invalidate an accepted best candidate.
    let limit = IDENTITY_MATCH_THRESHOLD + IDENTITY_AMBIGUITY_MARGIN;
    let mut candidates = Vec::new();
    for (identity, templates) in identities {
        let mut score = limit;
        for template in templates {
            if let Some(found) = best_template_score(frame, region, template, score, control)? {
                score = found;
            }
        }
        if score < limit {
            candidates.push((score, identity));
        }
    }
    candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
    let Some(&(best_score, identity)) = candidates.first() else {
        return Ok(Recognition::unknown(UnknownReason::NoCandidate, None));
    };
    if best_score >= IDENTITY_MATCH_THRESHOLD {
        return Ok(Recognition::unknown(UnknownReason::NoCandidate, None));
    }
    if candidates
        .get(1)
        .is_some_and(|(score, _)| score - best_score < IDENTITY_AMBIGUITY_MARGIN)
    {
        return Ok(Recognition::unknown(UnknownReason::Ambiguous, None));
    }
    Ok(identity.into())
}

struct CardIdentityTemplates {
    slot: PartySlot,
    templates: Vec<Arc<TemplatePixels>>,
}

struct SupportIdentityTemplates {
    atlas_id: u32,
    templates: Vec<Arc<TemplatePixels>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Match {
    pub x: u32,
    pub y: u32,
    pub score: f64,
    pub threshold: f64,
    pub region: Rect,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct MatchKey {
    name: String,
    region: Option<Rect>,
}

/// Reuses manifest detector results while one normalized frame is being decoded.
/// Dynamic card-identity templates remain outside this cache because their search
/// thresholds depend on the candidate currently being compared.
pub(crate) struct FrameMatcher<'a> {
    vision: &'a Vision,
    frame: &'a RgbaImage,
    control: &'a Control,
    matches: HashMap<MatchKey, Option<Match>>,
    counts: HashMap<(String, Rect), usize>,
    evaluations: usize,
}

impl<'a> FrameMatcher<'a> {
    pub(crate) fn frame(&self) -> &'a RgbaImage {
        self.frame
    }

    pub(crate) fn find(&mut self, name: &str) -> Result<Option<Match>> {
        self.find_in(name, None)
    }

    pub(crate) fn find_in(&mut self, name: &str, region: Option<Rect>) -> Result<Option<Match>> {
        let key = MatchKey {
            name: name.to_owned(),
            region,
        };
        if let Some(found) = self.matches.get(&key) {
            return Ok(found.clone());
        }
        let found = self
            .vision
            .find_in(self.frame, name, region, self.control)?;
        self.evaluations += 1;
        self.matches.insert(key, found.clone());
        Ok(found)
    }

    pub(crate) fn has(&mut self, name: &str) -> Result<bool> {
        Ok(self.find(name)?.is_some())
    }

    pub(crate) fn has_optional(&mut self, name: &str) -> Result<bool> {
        if !self.vision.has_template(name) {
            return Ok(false);
        }
        self.has(name)
    }

    pub(crate) fn count_in(&mut self, name: &str, region: Rect) -> Result<usize> {
        let key = (name.to_owned(), region);
        if let Some(count) = self.counts.get(&key) {
            return Ok(*count);
        }
        let count = self
            .vision
            .count_in(self.frame, name, region, self.control)?;
        self.evaluations += 1;
        self.counts.insert(key, count);
        Ok(count)
    }

    #[cfg(test)]
    pub(crate) fn evaluations(&self) -> usize {
        self.evaluations
    }
}

pub struct Vision {
    pub(crate) farm_templates: Option<crate::profile::FarmTemplateProfile>,
    templates: HashMap<String, Template>,
    card_identities: Vec<CardIdentityTemplates>,
    support_identities: Vec<SupportIdentityTemplates>,
    support_party_slot: Option<PartySlot>,
}

impl Vision {
    pub(crate) fn frame_matcher<'a>(
        &'a self,
        frame: &'a RgbaImage,
        control: &'a Control,
    ) -> FrameMatcher<'a> {
        FrameMatcher {
            vision: self,
            frame,
            control,
            matches: HashMap::new(),
            counts: HashMap::new(),
            evaluations: 0,
        }
    }

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
        let mut assets = HashMap::<PathBuf, Arc<TemplatePixels>>::new();
        for (name, spec) in specs {
            spec.region.validate(WIDTH, HEIGHT)?;
            ensure!(
                spec.threshold.is_finite() && spec.threshold > 0.0 && spec.threshold <= 1.0,
                "invalid threshold for {name}"
            );
            let path = base.join(&spec.file);
            let pixels = if let Some(pixels) = assets.get(&path) {
                pixels.clone()
            } else {
                let img = image::open(&path)
                    .with_context(|| format!("load {}", path.display()))?
                    .to_rgba8();
                let pixels = template_pixels(&img);
                ensure!(
                    pixels.energy > 0.0,
                    "template {name} has zero energy or empty mask"
                );
                let pixels = Arc::new(pixels);
                assets.insert(path.clone(), pixels.clone());
                pixels
            };
            ensure!(
                pixels.width <= spec.region.width && pixels.height <= spec.region.height,
                "template {name} exceeds region"
            );
            templates.insert(name, Template { spec, pixels });
        }
        Ok(Self {
            farm_templates: None,
            templates,
            card_identities: Vec::new(),
            support_identities: Vec::new(),
            support_party_slot: None,
        })
    }

    pub fn configure_card_identities(
        &mut self,
        identities: Vec<(PartySlot, Vec<RgbaImage>)>,
        support_party_slot: Option<PartySlot>,
    ) -> Result<()> {
        let mut slots = [false; 6];
        let mut configured = Vec::with_capacity(identities.len());
        for (slot, images) in identities {
            ensure!(
                !slots[slot.index()],
                "duplicate card identity for party slot {}",
                slot.index() + 1
            );
            slots[slot.index()] = true;
            ensure!(!images.is_empty(), "card identity has no templates");
            let mut templates = Vec::with_capacity(images.len());
            for image in images {
                let pixels = template_pixels(&image);
                ensure!(
                    pixels.width == 98 && pixels.height == 47 && pixels.energy > 0.0,
                    "invalid card identity template"
                );
                templates.push(Arc::new(pixels));
            }
            configured.push(CardIdentityTemplates { slot, templates });
        }
        self.card_identities = configured;
        self.support_party_slot = support_party_slot;
        Ok(())
    }

    pub fn configure_support_identities(
        &mut self,
        identities: Vec<(u32, Vec<RgbaImage>)>,
    ) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        let mut configured = Vec::with_capacity(identities.len());
        for (atlas_id, images) in identities {
            ensure!(atlas_id > 0, "invalid Atlas support identity");
            ensure!(
                ids.insert(atlas_id),
                "duplicate Atlas support identity {atlas_id}"
            );
            ensure!(!images.is_empty(), "support identity has no templates");
            let mut templates = Vec::with_capacity(images.len());
            for image in images {
                let pixels = template_pixels(&image);
                ensure!(
                    pixels.width == 98 && pixels.height == 47 && pixels.energy > 0.0,
                    "invalid support identity template"
                );
                templates.push(Arc::new(pixels));
            }
            configured.push(SupportIdentityTemplates {
                atlas_id,
                templates,
            });
        }
        self.support_identities = configured;
        Ok(())
    }

    pub(crate) const fn support_party_slot(&self) -> Option<PartySlot> {
        self.support_party_slot
    }

    pub(crate) fn card_identity(
        &self,
        frame: &RgbaImage,
        card_index: usize,
        control: &Control,
    ) -> Result<Option<PartySlot>> {
        match_identity_candidates(
            frame,
            UI_LAYOUT_V1.battle.card_identity_searches[card_index],
            self.card_identities
                .iter()
                .map(|identity| (identity.slot, identity.templates.as_slice())),
            control,
        )
        .map(|result| result.value())
    }

    #[cfg(test)]
    pub(crate) fn support_card_identity(
        &self,
        frame: &RgbaImage,
        card_index: usize,
        control: &Control,
    ) -> Result<Option<u32>> {
        self.support_card_recognition(frame, card_index, control)
            .map(|result| result.value())
    }

    pub(crate) fn support_card_recognition(
        &self,
        frame: &RgbaImage,
        card_index: usize,
        control: &Control,
    ) -> Result<Recognition<u32>> {
        match_identity_candidates(
            frame,
            UI_LAYOUT_V1.battle.card_identity_searches[card_index],
            self.support_identities
                .iter()
                .map(|identity| (identity.atlas_id, identity.templates.as_slice())),
            control,
        )
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
        let pixels = &template.pixels;
        region.validate(frame.width(), frame.height())?;
        ensure!(
            pixels.width <= region.width && pixels.height <= region.height,
            "template exceeds search region"
        );
        let mut best = template.spec.threshold;
        let mut result = None;
        let max_denominator =
            (pixels.energy * pixels.samples.len() as f64 * 3.0 * 255.0 * 255.0).sqrt();
        for y in region.y..=region.y + region.height - pixels.height {
            control.checkpoint()?;
            for x in region.x..=region.x + region.width - pixels.width {
                let mut difference = 0.0;
                let mut energy = 0.0;
                let cutoff = best * max_denominator;
                for &(tx, ty, values) in &pixels.samples {
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
                let score = (difference / (pixels.energy * energy).sqrt()).min(1.0);
                if score < best {
                    best = score;
                    result = Some(Match {
                        x: x + pixels.width / 2,
                        y: y + pixels.height / 2,
                        score,
                        threshold: template.spec.threshold,
                        region,
                    });
                }
            }
        }
        Ok(result)
    }

    /// Count disconnected regions in the thresholded template-match map.
    /// This mirrors OpenCV's matchTemplate + connectedComponents behavior for
    /// repeated fixed-size UI marks such as enemy charge pips.
    pub(crate) fn count_in(
        &self,
        frame: &RgbaImage,
        name: &str,
        region: Rect,
        control: &Control,
    ) -> Result<usize> {
        let template = self
            .templates
            .get(name)
            .with_context(|| format!("unknown template: {name}"))?;
        let pixels = &template.pixels;
        region.validate(frame.width(), frame.height())?;
        ensure!(
            pixels.width <= region.width && pixels.height <= region.height,
            "template exceeds search region"
        );
        let columns = (region.width - pixels.width + 1) as usize;
        let rows = (region.height - pixels.height + 1) as usize;
        let mut matches = vec![false; columns * rows];
        let max_denominator =
            (pixels.energy * pixels.samples.len() as f64 * 3.0 * 255.0 * 255.0).sqrt();
        let cutoff = template.spec.threshold * max_denominator;
        for row in 0..rows {
            control.checkpoint()?;
            for column in 0..columns {
                let mut difference = 0.0;
                let mut energy = 0.0;
                for &(tx, ty, values) in &pixels.samples {
                    let pixel =
                        frame.get_pixel(region.x + column as u32 + tx, region.y + row as u32 + ty);
                    for channel in 0..3 {
                        let value = pixel[channel] as f64;
                        difference += (value - values[channel]).powi(2);
                        energy += value * value;
                    }
                    if difference > cutoff {
                        break;
                    }
                }
                if difference <= cutoff
                    && energy > 0.0
                    && difference / (pixels.energy * energy).sqrt() < template.spec.threshold
                {
                    matches[row * columns + column] = true;
                }
            }
        }

        let mut count = 0;
        for start in 0..matches.len() {
            if !matches[start] {
                continue;
            }
            count += 1;
            matches[start] = false;
            let mut stack = vec![start];
            while let Some(index) = stack.pop() {
                let x = index % columns;
                let y = index / columns;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let next_x = x as isize + dx;
                        let next_y = y as isize + dy;
                        if next_x < 0
                            || next_x >= columns as isize
                            || next_y < 0
                            || next_y >= rows as isize
                        {
                            continue;
                        }
                        let next = next_y as usize * columns + next_x as usize;
                        if matches[next] {
                            matches[next] = false;
                            stack.push(next);
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    pub fn support(&self, frame: &RgbaImage, control: &Control) -> Result<Option<Match>> {
        // Split the search above/below each match, suppressing duplicate detections
        // within 20 pixels. This also finds weaker friend rows above a stronger
        // non-friend match. Return the uppermost complete friend row.
        let level_template = self
            .farm_templates
            .as_ref()
            .and_then(|farm| farm.support_level.as_deref())
            .unwrap_or("level120");
        let height = self
            .templates
            .get(level_template)
            .context("missing level120 template")?
            .pixels
            .height;
        let mut ranges = vec![(175, 690)];
        let mut candidates = Vec::new();
        while let Some((top, bottom)) = ranges.pop() {
            if bottom <= top || bottom - top < height {
                continue;
            }
            // The level badge can extend well past the old 153px ROI (the
            // current MuMu layout places the trailing "/120" near x=290).
            // Cover the complete left support card while keeping the search
            // away from servant name/details that can contain other digits.
            let region = Rect {
                x: 65,
                y: top,
                width: 280,
                height: bottom - top,
            };
            let Some(level) = self.find_in(frame, level_template, Some(region), control)? else {
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
                && match &self.farm_templates {
                    Some(farm) => self
                        .find_in(
                            frame,
                            &farm.support_name,
                            Some(Rect {
                                x: 350,
                                y: level.y.saturating_sub(20),
                                width: 650,
                                height: 140,
                            }),
                            control,
                        )?
                        .is_some(),
                    None => true,
                }
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
                    threshold: level.threshold,
                    region: level.region,
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

    #[test]
    fn saber_live_support_row() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        vision.farm_templates =
            crate::profile::PartyProfile::load(&root.join("profiles/saber-v1.json"))
                .unwrap()
                .farm_templates;
        let frame = image::open(root.join("logs/saber-live/test-vii-20260918c/frames/000007.png"))
            .unwrap()
            .to_rgba8();
        let control = Control::new(Duration::from_secs(30));
        let found = vision.support(&frame, &control).unwrap().unwrap();
        assert!(found.y > 250 && found.y < 300);
    }

    #[test]
    fn configured_support_name_must_share_the_level120_friend_row() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut vision = Vision::load(&root.join("assets/templates.json")).unwrap();
        let control = Control::new(Duration::from_secs(30));
        let original = image::open(root.join("tests/fixtures/step2.png"))
            .unwrap()
            .to_rgba8();
        let (original, _) = normalize(&original, None).unwrap();
        let candidate = vision.support(&original, &control).unwrap().unwrap();
        let name = RgbaImage::from_fn(50, 16, |x, y| {
            image::Rgba([
                ((x * 71 + y * 13) % 256) as u8,
                ((x * 31 + y * 83) % 256) as u8,
                120,
                255,
            ])
        });
        vision.templates.insert(
            "test_support_name".into(),
            Template {
                spec: TemplateSpec {
                    file: PathBuf::new(),
                    region: Rect {
                        x: 0,
                        y: 0,
                        width: WIDTH,
                        height: HEIGHT,
                    },
                    threshold: 0.001,
                },
                pixels: Arc::new(template_pixels(&name)),
            },
        );
        vision.farm_templates = Some(crate::profile::FarmTemplateProfile {
            event: "event".into(),
            quest: "quest".into(),
            formation_support: "formation_support120".into(),
            support_filter: "rider_tab".into(),
            support_tab: "rider_tab".into(),
            support_name: "test_support_name".into(),
            support_level: None,
        });
        assert!(vision.support(&original, &control).unwrap().is_none());
        let mut same_row = original.clone();
        imageops::replace(&mut same_row, &name, 400, i64::from(candidate.y - 45));
        assert_eq!(
            vision.support(&same_row, &control).unwrap().unwrap().y,
            candidate.y
        );
        let mut elsewhere = original;
        imageops::replace(&mut elsewhere, &name, 400, 20);
        assert!(vision.support(&elsewhere, &control).unwrap().is_none());
    }

    fn fixture(samples: Vec<(u32, u32, [f64; 3])>) -> Vision {
        let energy = samples.iter().flat_map(|s| s.2).map(|v| v * v).sum();
        Vision {
            farm_templates: None,
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
                    pixels: Arc::new(TemplatePixels {
                        width: 2,
                        height: 1,
                        samples,
                        energy,
                    }),
                },
            )]),
            card_identities: Vec::new(),
            support_identities: Vec::new(),
            support_party_slot: None,
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

    #[test]
    fn frame_matcher_reuses_detector_result_for_the_same_roi() {
        let vision = fixture(vec![(0, 0, [100.0; 3])]);
        let frame = RgbaImage::from_pixel(2, 1, image::Rgba([100, 100, 100, 255]));
        let control = Control::new(Duration::from_secs(1));
        let mut matcher = vision.frame_matcher(&frame, &control);

        assert!(matcher.find("test").unwrap().is_some());
        assert!(matcher.find("test").unwrap().is_some());
        assert_eq!(matcher.evaluations(), 1);
    }

    #[test]
    fn manifest_reuses_pixels_for_shifted_detectors() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/templates.json");
        let vision = Vision::load(&manifest).unwrap();
        assert!(Arc::ptr_eq(
            &vision.templates["arts_1"].pixels,
            &vision.templates["arts_5"].pixels
        ));
        assert!(Arc::ptr_eq(
            &vision.templates["critical_1_1"].pixels,
            &vision.templates["critical_1_5"].pixels
        ));
    }

    #[test]
    fn identity_runner_up_above_threshold_still_rejects_ambiguity() {
        let mut vision = fixture(vec![(0, 0, [100.0; 3])]);
        let frame = RgbaImage::from_pixel(WIDTH, HEIGHT, image::Rgba([100, 100, 100, 255]));
        let best = RgbaImage::from_pixel(98, 47, image::Rgba([72, 72, 72, 255]));
        let mut second = best.clone();
        for (index, pixel) in second.pixels_mut().enumerate() {
            if index % 4 == 0 {
                *pixel = image::Rgba([71, 71, 71, 255]);
            }
        }
        let control = Control::new(Duration::from_secs(5));
        let region = UI_LAYOUT_V1.battle.card_identity_searches[0];
        let best_score =
            best_template_score(&frame, region, &template_pixels(&best), 1.0, &control)
                .unwrap()
                .unwrap();
        let second_score =
            best_template_score(&frame, region, &template_pixels(&second), 1.0, &control)
                .unwrap()
                .unwrap();
        assert!(best_score < IDENTITY_MATCH_THRESHOLD);
        assert!(second_score > IDENTITY_MATCH_THRESHOLD);
        assert!(second_score - best_score < IDENTITY_AMBIGUITY_MARGIN);

        vision
            .configure_card_identities(
                vec![
                    (PartySlot::One, vec![best.clone()]),
                    (PartySlot::Two, vec![second.clone()]),
                ],
                None,
            )
            .unwrap();
        vision
            .configure_support_identities(vec![(1, vec![best.clone()]), (2, vec![second.clone()])])
            .unwrap();
        assert_eq!(vision.card_identity(&frame, 0, &control).unwrap(), None);
        assert_eq!(
            vision.support_card_identity(&frame, 0, &control).unwrap(),
            None
        );

        // Multiple ascensions of the same identity are not competing members.
        vision
            .configure_card_identities(
                vec![(PartySlot::One, vec![best.clone(), second.clone()])],
                None,
            )
            .unwrap();
        assert_eq!(
            vision.card_identity(&frame, 0, &control).unwrap(),
            Some(PartySlot::One)
        );
        // Being the only candidate does not make an above-threshold score valid.
        vision
            .configure_support_identities(vec![(2, vec![second])])
            .unwrap();
        assert_eq!(
            vision.support_card_identity(&frame, 0, &control).unwrap(),
            None
        );
    }

    #[test]
    fn support_identity_rejects_ambiguous_candidates() {
        let mut vision = fixture(vec![(0, 0, [100.0; 3])]);
        let template = RgbaImage::from_pixel(98, 47, image::Rgba([120, 80, 40, 255]));
        let mut frame = RgbaImage::new(WIDTH, HEIGHT);
        image::imageops::replace(&mut frame, &template, 76, 431);
        let control = Control::new(Duration::from_secs(1));

        vision
            .configure_support_identities(vec![
                (704000, vec![template.clone()]),
                (704100, vec![template.clone()]),
            ])
            .unwrap();
        assert_eq!(
            vision.support_card_identity(&frame, 0, &control).unwrap(),
            None
        );

        vision
            .configure_support_identities(vec![(704000, vec![template])])
            .unwrap();
        assert_eq!(
            vision.support_card_identity(&frame, 0, &control).unwrap(),
            Some(704000)
        );
    }
}
