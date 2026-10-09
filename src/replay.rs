use crate::runtime::{EventSink, JsonlEventSink, RuntimeEvent};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{
    io::Write,
    path::Path,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayComparisonMode {
    Observations,
    Events,
}

impl FromStr for ReplayComparisonMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "observations" => Ok(Self::Observations),
            "events" => Ok(Self::Events),
            _ => Err("comparison mode must be observations or events".into()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReplayTrace {
    events: Vec<Value>,
    capture_timestamps: Vec<Duration>,
}

impl ReplayTrace {
    /// Load the last run from an append-only JSONL trace.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read replay trace {}", path.display()))?;
        let mut parsed = Vec::new();
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            parsed.push(
                serde_json::from_str::<Value>(line)
                    .with_context(|| format!("parse replay trace line {}", index + 1))?,
            );
        }
        let run_id = parsed
            .last()
            .and_then(|event| event.get("run_id"))
            .and_then(Value::as_str)
            .context("replay trace has no run events")?
            .to_owned();
        let events = parsed
            .into_iter()
            .filter(|event| event.get("run_id").and_then(Value::as_str) == Some(run_id.as_str()))
            .collect::<Vec<_>>();
        let capture_timestamps = events
            .iter()
            .filter(|event| event.get("action").and_then(Value::as_str) == Some("capture"))
            .map(|event| {
                event
                    .get("elapsed_ms")
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis)
                    .context("capture event has no elapsed_ms")
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !capture_timestamps.is_empty(),
            "replay trace contains no capture events"
        );
        ensure!(
            capture_timestamps.windows(2).all(|pair| pair[0] <= pair[1]),
            "replay capture timestamps are not monotonic"
        );
        Ok(Self {
            events,
            capture_timestamps,
        })
    }

    pub fn capture_timestamps(&self) -> Vec<Duration> {
        self.capture_timestamps.clone()
    }

    pub fn compare(&self, actual: &[Value], mode: ReplayComparisonMode) -> ReplayDiffReport {
        compare_event_streams(&self.events, actual, mode)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayDifference {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReplayDiffReport {
    pub schema: u32,
    pub mode: ReplayComparisonMode,
    pub matched: bool,
    pub expected_events: usize,
    pub actual_events: usize,
    pub difference_count: usize,
    pub differences: Vec<ReplayDifference>,
}

const MAX_REPORTED_DIFFERENCES: usize = 20;

pub fn compare_event_streams(
    expected: &[Value],
    actual: &[Value],
    mode: ReplayComparisonMode,
) -> ReplayDiffReport {
    let expected = comparable_events(expected, mode);
    let actual = comparable_events(actual, mode);
    let difference_count = expected
        .iter()
        .zip(&actual)
        .filter(|(left, right)| left != right)
        .count()
        + expected.len().abs_diff(actual.len());
    let differences = (0..expected.len().max(actual.len()))
        .filter_map(|index| {
            let left = expected.get(index);
            let right = actual.get(index);
            (left != right).then(|| ReplayDifference {
                index,
                expected: left.cloned(),
                actual: right.cloned(),
            })
        })
        .take(MAX_REPORTED_DIFFERENCES)
        .collect();
    ReplayDiffReport {
        schema: 1,
        mode,
        matched: difference_count == 0,
        expected_events: expected.len(),
        actual_events: actual.len(),
        difference_count,
        differences,
    }
}

fn comparable_events(events: &[Value], mode: ReplayComparisonMode) -> Vec<Value> {
    events
        .iter()
        .filter(|event| {
            mode == ReplayComparisonMode::Events
                || event.get("action").and_then(Value::as_str) == Some("observation")
        })
        .cloned()
        .map(canonical_event)
        .collect()
}

fn canonical_event(mut event: Value) -> Value {
    let Some(object) = event.as_object_mut() else {
        return event;
    };
    object.remove("run_id");
    object.remove("elapsed_ms");
    object.remove("sequence");
    match object.get("action").and_then(Value::as_str) {
        Some("capture") => {
            object.retain(|key, _| matches!(key.as_str(), "frame" | "action"));
        }
        Some("tap") => {
            object.remove("device_point");
            object.remove("executed");
        }
        Some("swipe") => {
            object.remove("device_from");
            object.remove("device_to");
            object.remove("executed");
        }
        Some("verification") => {
            object.remove("started_at_ms");
            object.remove("deadline_ms");
            if let Some(observation) = object
                .get_mut("current_observation")
                .and_then(Value::as_object_mut)
            {
                observation.remove("elapsed_ms");
            }
        }
        _ => {}
    }
    event
}

#[derive(Clone, Default)]
pub struct CapturedReplayEvents(Arc<Mutex<Vec<Value>>>);

impl CapturedReplayEvents {
    pub fn snapshot(&self) -> Result<Vec<Value>> {
        self.0
            .lock()
            .map(|events| events.clone())
            .map_err(|_| anyhow::anyhow!("replay event buffer lock poisoned"))
    }
}

pub struct ReplayEventSink {
    output: JsonlEventSink,
    captured: CapturedReplayEvents,
}

impl ReplayEventSink {
    pub fn new(trace: Option<Box<dyn Write + Send>>) -> (Self, CapturedReplayEvents) {
        let captured = CapturedReplayEvents::default();
        (
            Self {
                output: JsonlEventSink::new(trace),
                captured: captured.clone(),
            },
            captured,
        )
    }
}

impl EventSink for ReplayEventSink {
    fn emit(&mut self, event: RuntimeEvent) -> Result<()> {
        let value = serde_json::to_value(&event)?;
        self.output.emit(event)?;
        self.captured
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("replay event buffer lock poisoned"))?
            .push(value);
        Ok(())
    }
}

pub fn require_matching_replay(report: &ReplayDiffReport) -> Result<()> {
    if report.matched {
        Ok(())
    } else {
        bail!(
            "replay differs in {} event(s); inspect the JSON diff report",
            report.difference_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[test]
    fn trace_scores_round_trip_without_hiding_real_differences() {
        let file = NamedTempFile::new().unwrap();
        let scores = [
            0.059279108877783364_f64,
            0.10585346398713447,
            0.09455218943020247,
            0.09112489422869635,
            0.056312876566586324,
            0.046801036200119564,
        ];
        let events = vec![
            json!({"run_id": "live", "action": "capture", "elapsed_ms": 1, "frame": 1}),
            json!({"run_id": "live", "action": "observation", "frame": 1,
                "evidence": scores.map(|score| json!({"score": score}))}),
        ];
        std::fs::write(
            file.path(),
            events
                .iter()
                .map(|event| format!("{event}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let trace = ReplayTrace::load(file.path()).unwrap();
        assert!(trace.compare(&events, ReplayComparisonMode::Events).matched);
        let mut changed = events;
        changed[1]["evidence"][0]["score"] = json!(f64::from_bits(scores[0].to_bits() + 1));
        assert_eq!(
            trace
                .compare(&changed, ReplayComparisonMode::Events)
                .difference_count,
            1
        );
    }

    #[test]
    fn trace_loader_uses_the_last_run_and_capture_timeline() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            concat!(
                "{\"run_id\":\"old\",\"action\":\"capture\",\"elapsed_ms\":5}\n",
                "{\"run_id\":\"new\",\"action\":\"capture\",\"elapsed_ms\":10}\n",
                "{\"run_id\":\"new\",\"action\":\"observation\",\"elapsed_ms\":11}\n",
                "{\"run_id\":\"new\",\"action\":\"capture\",\"elapsed_ms\":25}\n",
            ),
        )
        .unwrap();
        let trace = ReplayTrace::load(file.path()).unwrap();
        assert_eq!(
            trace.capture_timestamps(),
            [Duration::from_millis(10), Duration::from_millis(25)]
        );
        assert_eq!(trace.events.len(), 3);
    }

    #[test]
    fn semantic_comparison_ignores_runtime_and_device_specific_fields() {
        let expected = vec![json!({
            "run_id": "a", "sequence": 1, "frame": 2, "elapsed_ms": 100,
            "action": "tap", "point": {"x": 1, "y": 2},
            "device_point": {"x": 2, "y": 4}, "executed": true
        })];
        let actual = vec![json!({
            "run_id": "b", "sequence": 1, "frame": 2, "elapsed_ms": 900,
            "action": "tap", "point": {"x": 1, "y": 2},
            "device_point": {"x": 1, "y": 2}, "executed": false
        })];
        assert!(compare_event_streams(&expected, &actual, ReplayComparisonMode::Events).matched);
    }

    #[test]
    fn observation_comparison_reports_semantic_differences() {
        let expected = vec![json!({
            "run_id": "a", "sequence": 1, "frame": 1, "elapsed_ms": 1,
            "action": "observation", "schema": 1, "scene": "battle_turn"
        })];
        let actual = vec![json!({
            "run_id": "b", "sequence": 1, "frame": 1, "elapsed_ms": 2,
            "action": "observation", "schema": 1, "scene": "unknown"
        })];
        let report = compare_event_streams(&expected, &actual, ReplayComparisonMode::Observations);
        assert!(!report.matched);
        assert_eq!(report.difference_count, 1);
        assert_eq!(report.differences[0].index, 0);
    }
}
