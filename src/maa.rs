//! MaaFramework recognition only. No controller is bound and no action is posted.
use crate::{
    control::Control,
    vision::{Match, Rect},
};
use anyhow::{Context, Result, anyhow, ensure};
use image::{RgbaImage, imageops};
use maa_framework::{
    buffer::MaaImageBuffer, common::MaaStatus, resource::Resource, tasker::Tasker,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const JOB_TIMEOUT: Duration = Duration::from_secs(15);
static LIBRARY: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Serialize)]
pub struct NativeRecognition {
    pub hit: bool,
    pub detail: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roi: Option<Rect>,
}

struct Session {
    tasker: Tasker,
    resource: Resource,
    stopped: bool,
    templates: HashMap<String, Vec<MaaTemplate>>,
    template_dir: tempfile::TempDir,
}

pub struct MaaRecognizer {
    session: Mutex<Session>,
    ocr_available: bool,
}

impl MaaRecognizer {
    pub fn new(library: &Path, bundles: &[PathBuf], ocr_model: Option<&Path>) -> Result<Self> {
        if let Some(model) = ocr_model {
            for file in ["det.onnx", "rec.onnx", "keys.txt"] {
                ensure!(
                    model.join(file).is_file(),
                    "OCR model is missing {file}: {}",
                    model.display()
                );
            }
        }
        let path = library
            .canonicalize()
            .context("locate MaaFramework library")?;
        let mut loaded = LIBRARY
            .lock()
            .map_err(|_| anyhow!("MaaFramework library lock poisoned"))?;
        if let Some(previous) = loaded.as_ref() {
            ensure!(
                *previous == path,
                "MaaFramework library is already loaded from {}",
                previous.display()
            );
        } else {
            maa_framework::load_library(&path).map_err(|error| anyhow!(error))?;
            *loaded = Some(path);
            Tasker::set_stdout_level(0).context("disable MaaFramework stdout logging")?;
        }
        drop(loaded);
        ensure!(
            maa_framework::util::version() == "v5.14.2",
            "MaaFramework v5.14.2 is required, found {}",
            maa_framework::util::version()
        );
        let resource = Resource::new().context("create MaaFramework resource")?;
        resource
            .use_cpu()
            .context("select MaaFramework CPU inference")?;
        for bundle in bundles {
            let job =
                resource.post_bundle(bundle.to_str().context("resource path must be UTF-8")?)?;
            ensure!(
                job.wait().succeeded(),
                "load MaaFramework resource {}",
                bundle.display()
            );
        }
        if let Some(model) = ocr_model {
            let job =
                resource.post_ocr_model(model.to_str().context("OCR model path must be UTF-8")?)?;
            ensure!(
                job.wait().succeeded(),
                "load MaaFramework OCR model {}",
                model.display()
            );
        }
        let tasker = Tasker::new().context("create MaaFramework tasker")?;
        tasker
            .bind_resource(&resource)
            .context("bind MaaFramework resource")?;
        ensure!(
            tasker.inited(),
            "MaaFramework needs a valid resource bundle"
        );
        Ok(Self {
            ocr_available: ocr_model.is_some()
                || bundles
                    .iter()
                    .any(|bundle| bundle.join("model/OCR/rec.onnx").is_file()),
            session: Mutex::new(Session {
                tasker,
                resource,
                stopped: false,
                templates: HashMap::new(),
                template_dir: tempfile::Builder::new().prefix("fgo-rs-maa-").tempdir()?,
            }),
        })
    }

    pub fn has_ocr(&self) -> bool {
        self.ocr_available
    }

    pub(crate) fn install_templates<'a>(
        &self,
        templates: impl Iterator<Item = (&'a str, Vec<RgbaImage>)>,
    ) -> Result<()> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("MaaFramework session lock poisoned"))?;
        ensure!(
            session.templates.is_empty(),
            "MaaFramework templates are already installed"
        );
        for (index, (name, images)) in templates.enumerate() {
            let mut variants = Vec::with_capacity(images.len());
            for (variant, image) in images.into_iter().enumerate() {
                let file = format!("__fgo_rs_{index:04}_{variant:02}.png");
                let path = session.template_dir.path().join(&file);
                variants.push(MaaTemplate::write(&image, &path, file)?);
            }
            session.templates.insert(name.to_owned(), variants);
        }
        let job = session.resource.post_image(
            session
                .template_dir
                .path()
                .to_str()
                .context("template path must be UTF-8")?,
        )?;
        ensure!(
            job.wait().succeeded(),
            "load converted MaaFramework templates"
        );
        Ok(())
    }

    pub fn recognize(
        &self,
        image: &RgbaImage,
        algorithm: &str,
        parameters: &Value,
        control: &Control,
    ) -> Result<NativeRecognition> {
        ensure!(
            algorithm != "OCR" || self.has_ocr(),
            "MaaFramework OCR model is not configured"
        );
        ensure!(
            matches!(
                algorithm,
                "TemplateMatch" | "OCR" | "ColorMatch" | "FeatureMatch" | "And" | "Or"
            ),
            "unsupported MaaFramework recognition algorithm: {algorithm}"
        );
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("MaaFramework session lock poisoned"))?;
        Self::recognize_in(&mut session, image, algorithm, parameters, control)
    }

    pub fn recognize_node(
        &self,
        image: &RgbaImage,
        node: &str,
        control: &Control,
    ) -> Result<NativeRecognition> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("MaaFramework session lock poisoned"))?;
        let data = session
            .resource
            .get_node_data(node)?
            .with_context(|| format!("unknown MaaFramework node: {node}"))?;
        let data: Value = serde_json::from_str(&data)?;
        let recognition = &data["recognition"];
        let algorithm = recognition["type"]
            .as_str()
            .context("node has no recognition type")?;
        ensure!(
            algorithm != "OCR" || self.has_ocr(),
            "MaaFramework OCR model is not configured"
        );
        let mut result = Self::recognize_in(
            &mut session,
            image,
            algorithm,
            &recognition["param"],
            control,
        )?;
        if let Some(roi) = recognition["param"]["roi"]
            .as_array()
            .filter(|roi| roi.len() == 4)
        {
            let values: Option<Vec<u32>> = roi
                .iter()
                .map(|value| value.as_u64().and_then(|value| u32::try_from(value).ok()))
                .collect();
            if let Some(values) = values {
                result.roi = Some(Rect {
                    x: values[0],
                    y: values[1],
                    width: values[2],
                    height: values[3],
                });
            }
        }
        Ok(result)
    }

    fn recognize_in(
        session: &mut Session,
        image: &RgbaImage,
        algorithm: &str,
        parameters: &Value,
        control: &Control,
    ) -> Result<NativeRecognition> {
        ensure!(
            matches!(
                algorithm,
                "TemplateMatch" | "OCR" | "ColorMatch" | "FeatureMatch" | "And" | "Or"
            ),
            "unsupported MaaFramework recognition algorithm: {algorithm}"
        );
        control.checkpoint()?;
        ensure!(
            !session.stopped,
            "MaaFramework session was stopped after a failed recognition"
        );
        ensure!(
            image.width() > 0
                && image.height() > 0
                && image.width() <= 1280
                && image.height() <= 720,
            "invalid MaaFramework input dimensions"
        );
        let bgr: Vec<u8> = image
            .pixels()
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
            .collect();
        let mut buffer = MaaImageBuffer::new()?;
        buffer.set(&bgr, image.width() as i32, image.height() as i32)?;
        let job = session
            .tasker
            .post_recognition(algorithm, &parameters.to_string(), &buffer)?;
        ensure!(job.id > 0, "MaaFramework rejected recognition parameters");
        let start = Instant::now();
        loop {
            if let Err(error) = control.check_cancel().and_then(|()| {
                ensure!(
                    start.elapsed() < JOB_TIMEOUT,
                    "MaaFramework recognition timed out"
                );
                Ok(())
            }) {
                session.stopped = true;
                let _ = session.tasker.post_stop();
                return Err(error);
            }
            match job.status() {
                MaaStatus::SUCCEEDED => break,
                MaaStatus::PENDING | MaaStatus::RUNNING => thread::sleep(Duration::from_millis(1)),
                _ => return Err(anyhow!("MaaFramework recognition failed")),
            }
        }
        control.checkpoint()?;
        // PostRecognition returns a task ID. Resolve its node's recognition ID;
        // the binding's Job::get currently treats the task ID as a reco ID.
        let task = session
            .tasker
            .get_task_detail(job.id)?
            .context("MaaFramework returned no task detail")?;
        ensure!(
            task.nodes.len() == 1,
            "unexpected MaaFramework recognition task graph"
        );
        let node = task
            .nodes
            .into_iter()
            .next()
            .flatten()
            .context("MaaFramework returned no node detail")?;
        ensure!(
            node.action.is_none(),
            "recognition task unexpectedly contains an action"
        );
        let result = node
            .recognition
            .context("MaaFramework returned no recognition detail")?;
        session
            .tasker
            .clear_cache()
            .context("clear MaaFramework recognition cache")?;
        Ok(NativeRecognition {
            hit: result.hit,
            detail: result.detail,
            roi: None,
        })
    }

    pub(crate) fn find(
        &self,
        frame: &RgbaImage,
        region: Rect,
        (name, variant): (&str, usize),
        threshold: f64,
        control: &Control,
    ) -> Result<Option<Match>> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("MaaFramework session lock poisoned"))?;
        let template = session
            .templates
            .get(name)
            .and_then(|templates| templates.get(variant))
            .with_context(|| format!("unknown native template: {name}"))?;
        let (width, height, file) = (template.width, template.height, template.file.clone());
        region.validate(frame.width(), frame.height())?;
        ensure!(
            width <= region.width && height <= region.height,
            "template exceeds search region"
        );
        // Crop before transfer; Maa's low-level recognition does not resize it.
        let crop =
            imageops::crop_imm(frame, region.x, region.y, region.width, region.height).to_image();
        let result = Self::recognize_in(
            &mut session,
            &crop,
            "TemplateMatch",
            &json!({
                "template": file,
                "green_mask": true,
                "method": 1,
                "threshold": threshold,
                "order_by": "Score",
            }),
            control,
        )?;
        if !result.hit {
            return Ok(None);
        }
        let best = &result.detail["best"];
        let score = best["score"]
            .as_f64()
            .context("MaaFramework template score missing")?;
        ensure!(
            score.is_finite() && score >= 0.0,
            "invalid MaaFramework template score"
        );
        if score >= threshold {
            return Ok(None);
        }
        let bbox = best["box"]
            .as_array()
            .context("MaaFramework template box missing")?;
        ensure!(bbox.len() == 4, "invalid MaaFramework template box");
        let x = u32::try_from(bbox[0].as_u64().context("invalid template x")?)?;
        let y = u32::try_from(bbox[1].as_u64().context("invalid template y")?)?;
        ensure!(
            x <= region.width - width && y <= region.height - height,
            "MaaFramework match outside ROI"
        );
        Ok(Some(Match {
            x: region.x + x + width / 2,
            y: region.y + y + height / 2,
            score,
            threshold,
            region,
        }))
    }
}

pub(crate) struct MaaTemplate {
    width: u32,
    height: u32,
    file: String,
}

impl MaaTemplate {
    fn write(image: &RgbaImage, path: &Path, file: String) -> Result<Self> {
        let mut encoded = image::RgbImage::new(image.width(), image.height());
        for (x, y, pixel) in image.enumerate_pixels() {
            ensure!(
                pixel[3] == 0 || pixel.0[..3] != [0, 255, 0],
                "opaque green pixel conflicts with MaaFramework's mask; use the Rust matcher for this template"
            );
            encoded.put_pixel(
                x,
                y,
                image::Rgb(if pixel[3] == 0 {
                    [0, 255, 0]
                } else {
                    [pixel[0], pixel[1], pixel[2]]
                }),
            );
        }
        encoded.save(path)?;
        Ok(Self {
            width: image.width(),
            height: image.height(),
            file,
        })
    }
}
