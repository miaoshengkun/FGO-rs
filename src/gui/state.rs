//! Bounded, presentation-only projection of typed runtime events.
use crate::{
    domain::Scene,
    runner::EventObserver,
    runtime::{Action, FrameSink, RuntimeEvent},
};
use image::RgbaImage;
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) const LOG_LIMIT: usize = 300;
const LINE_LIMIT: usize = 2000;

#[derive(Default)]
pub(super) struct ViewState {
    pub scene: String,
    pub wave: String,
    pub turn: String,
    pub numbers: String,
}

pub(super) struct Preview {
    pub frame: RgbaImage,
    pub description: String,
}

#[derive(Default)]
pub(super) struct Updates {
    pub frame: Option<Preview>,
    pub view: Option<ViewState>,
    pub logs: VecDeque<String>,
    pub omitted: usize,
}

#[derive(Clone, Default)]
pub(super) struct Mailbox(Arc<Mutex<Updates>>);

impl Mailbox {
    pub fn drain(&self) -> Updates {
        self.0
            .try_lock()
            .map(|mut data| std::mem::take(&mut *data))
            .unwrap_or_default()
    }

    pub fn log(&self, message: String) {
        if let Ok(mut data) = self.0.try_lock() {
            if data.logs.len() == LOG_LIMIT {
                data.logs.pop_front();
                data.omitted += 1;
            }
            data.logs
                .push_back(message.chars().take(LINE_LIMIT).collect());
        }
    }

    pub fn frame(&self, frame: &RgbaImage, description: String) {
        if let Ok(mut data) = self.0.try_lock() {
            data.frame = Some(Preview {
                frame: frame.clone(),
                description,
            });
        }
    }
}

pub(super) struct UiObserver {
    mailbox: Mailbox,
    view: ViewState,
}

impl UiObserver {
    pub fn new(mailbox: Mailbox) -> Self {
        Self {
            mailbox,
            view: ViewState::default(),
        }
    }
}

impl EventObserver for UiObserver {
    fn observe(&mut self, event: &RuntimeEvent) {
        let message = match &event.action {
            Action::Log { message } | Action::Decision { message } => Some(message.clone()),
            Action::Error { phase, message } => Some(format!("错误 [{phase}] {message}")),
            Action::Scene { current, .. } => {
                self.view.scene = scene_name(*current);
                Some(format!("场景 → {}", self.view.scene))
            }
            Action::Observation {
                scene,
                facts,
                unknown_fields,
                ..
            } => {
                self.view.scene = scene_name(*scene);
                self.view.numbers = numeric_summary(facts);
                if unknown_fields.is_empty() {
                    None
                } else {
                    Some(format!("未知字段：{}", unknown_fields.join(", ")))
                }
            }
            Action::BattleProgress { progress } => {
                self.view.wave = progress
                    .wave
                    .map(|w| format!("{}/{}", w.current, w.total))
                    .unwrap_or_else(|| "未知".into());
                self.view.turn = format!("{}（本波 {}）", progress.turns_seen, progress.wave_turn);
                None
            }
            Action::Command { command } => Some(format!("指令 {command:?}")),
            Action::Verification { status, .. } => Some(format!("动作验证 {status:?}")),
            Action::Tap { executed, .. } | Action::Swipe { executed, .. } => Some(
                if *executed {
                    "已发送设备输入"
                } else {
                    "演练：跳过设备输入"
                }
                .into(),
            ),
            Action::Capture { .. } => None,
        };
        if let Some(message) = message {
            self.mailbox.log(format!(
                "{:>6.1}s  {}",
                event.elapsed_ms as f64 / 1000.0,
                message
            ));
        }
        if let Ok(mut data) = self.mailbox.0.try_lock() {
            data.view = Some(ViewState {
                scene: self.view.scene.clone(),
                wave: self.view.wave.clone(),
                turn: self.view.turn.clone(),
                numbers: self.view.numbers.clone(),
            });
        }
    }
}

pub(super) struct UiFrames {
    mailbox: Mailbox,
    last: Option<Instant>,
}

impl UiFrames {
    pub fn new(mailbox: Mailbox) -> Self {
        Self {
            mailbox,
            last: None,
        }
    }
}

impl FrameSink for UiFrames {
    fn publish(&mut self, frame_id: u64, elapsed_ms: u64, frame: &RgbaImage) {
        if self
            .last
            .is_some_and(|last| last.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.last = Some(Instant::now());
        self.mailbox.frame(
            frame,
            format!(
                "运行截图 #{frame_id} · {:.1}s · {} × {}",
                elapsed_ms as f64 / 1000.0,
                frame.width(),
                frame.height()
            ),
        );
    }
}

fn numeric_summary(facts: &Value) -> String {
    let Some(hp) = facts.get("ally_hp").and_then(Value::as_array) else {
        return "HP / NP · 未请求识别".into();
    };
    let np = facts.get("np_percent").and_then(Value::as_array);
    (0..3)
        .map(|index| {
            let hp = known_number(hp.get(index));
            let np = known_number(np.and_then(|values| values.get(index)));
            format!("{}号位  HP {hp} / NP {np}", index + 1)
        })
        .collect::<Vec<_>>()
        .join("    ·    ")
}

fn known_number(value: Option<&Value>) -> String {
    match value {
        Some(value) if value.get("status").and_then(Value::as_str) == Some("known") => value
            .get("value")
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "未知".into()),
        Some(value) if value.get("status").and_then(Value::as_str) == Some("not_requested") => {
            "—".into()
        }
        _ => "未知".into(),
    }
}

fn scene_name(scene: Scene) -> String {
    match scene {
        Scene::Unknown => "未知".into(),
        Scene::QuestList => "关卡列表".into(),
        Scene::SupportList => "助战列表".into(),
        Scene::Formation => "队伍确认".into(),
        Scene::BattleTurn => "战斗回合".into(),
        Scene::BattleAnimation => "战斗动画".into(),
        Scene::CommandCards => "指令卡选择".into(),
        Scene::Victory => "战斗胜利".into(),
        Scene::Defeated => "战斗失败".into(),
        Scene::Settlement => "结算".into(),
        Scene::ApRecovery => "体力不足".into(),
        _ => format!("{scene:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_ui_keeps_one_frame_and_bounded_logs() {
        let mailbox = Mailbox::default();
        for index in 0..1000 {
            mailbox.log(index.to_string());
            mailbox.frame(&RgbaImage::new(2, 2), index.to_string());
        }
        let updates = mailbox.drain();
        assert_eq!(updates.logs.len(), LOG_LIMIT);
        assert_eq!(updates.omitted, 700);
        assert_eq!(updates.frame.unwrap().description, "999");
        assert!(mailbox.drain().frame.is_none());
    }

    #[test]
    fn ui_holding_mailbox_cannot_block_worker() {
        let mailbox = Mailbox::default();
        let guard = mailbox.0.lock().unwrap();
        let worker = mailbox.clone();
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            worker.log("busy".into());
            worker.frame(&RgbaImage::new(1, 1), "busy".into());
            done.send(()).unwrap();
        });
        result.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(guard);
    }

    #[test]
    fn unknown_and_unrequested_numbers_are_never_displayed_as_zero() {
        let facts = serde_json::json!({"ally_hp":[{"status":"known","value":0},{"status":"unknown"},{"status":"not_requested"}],"np_percent":[{"status":"known","value":120},{"status":"unknown"},{"status":"not_requested"}]});
        assert_eq!(
            numeric_summary(&facts),
            "1号位  HP 0 / NP 120    ·    2号位  HP 未知 / NP 未知    ·    3号位  HP — / NP —"
        );
    }
}
