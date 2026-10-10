//! Cropped Maa OCR for numeric battle facts. A configured OCR miss stays
//! unknown; fixed-font fallback is reserved for runs without an OCR model.
use crate::{
    control::Control,
    maa::NativeRecognition,
    np_ocr::complete_np_text,
    observation::{
        DetectorKind, Recognition, RecognitionEvidence, RecognitionMethod, ScoreDirection,
        UnknownReason,
    },
    vision::{Rect, Vision},
};
use anyhow::Result;
use image::RgbaImage;
use serde_json::json;

const NUMERIC_THRESHOLD: f64 = 0.98;
const NP_PATTERN: &str = "^(0|[1-9][0-9]?|[12][0-9]{2}|300)%$";
const HP_PATTERN: &str = "^([1-9][0-9]*|[1-9][0-9]{0,2}(,[0-9]{3})+)$";

impl Vision {
    pub(crate) fn maa_np_percent(
        &self,
        frame: &RgbaImage,
        glyph_roi: Rect,
        control: &Control,
    ) -> Result<Option<Recognition<u16>>> {
        let Some(maa) = self.maa_ocr() else {
            return Ok(None);
        };
        // Padding above the fixed glyph ROI improves the recognizer while
        // keeping the gauge below it out of the crop.
        let roi = Rect {
            y: glyph_roi.y - 5,
            height: glyph_roi.height + 5,
            ..glyph_roi
        };
        let result = maa.recognize(frame, "OCR", &numeric_parameters(roi, NP_PATTERN), control)?;
        let reading = numeric_reading(&result, roi, DetectorKind::NpPercent, parse_np);
        let reading = match reading {
            Recognition::Known { value, evidence }
                if !complete_np_text(frame, glyph_roi, value) =>
            {
                Recognition::unknown(UnknownReason::IncompleteText, evidence)
            }
            _ => reading,
        };
        Ok(Some(reading))
    }

    pub(crate) fn maa_enemy_hp(
        &self,
        frame: &RgbaImage,
        roi: Rect,
        control: &Control,
    ) -> Result<Option<Recognition<u32>>> {
        let Some(maa) = self.maa_ocr() else {
            return Ok(None);
        };
        let result = maa.recognize(frame, "OCR", &numeric_parameters(roi, HP_PATTERN), control)?;
        Ok(Some(numeric_reading(
            &result,
            roi,
            DetectorKind::EnemyHp,
            parse_hp,
        )))
    }
}

fn numeric_parameters(roi: Rect, pattern: &str) -> serde_json::Value {
    json!({
        "roi": [roi.x, roi.y, roi.width, roi.height],
        "only_rec": true,
        "expected": pattern,
        "threshold": NUMERIC_THRESHOLD,
    })
}

fn numeric_reading<T: Copy>(
    result: &NativeRecognition,
    roi: Rect,
    detector: DetectorKind,
    parse: fn(&str) -> Option<T>,
) -> Recognition<T> {
    let best = &result.detail["best"];
    let score = best["score"]
        .as_f64()
        .filter(|score| score.is_finite() && (0.0..=1.0).contains(score))
        .unwrap_or(0.0);
    let evidence = RecognitionEvidence {
        detector,
        roi,
        method: RecognitionMethod::MaaOcr,
        score,
        threshold: NUMERIC_THRESHOLD,
        direction: ScoreDirection::HigherIsBetter,
        feature_floor: None,
    };
    if !result.hit || score < NUMERIC_THRESHOLD {
        return Recognition::unknown(UnknownReason::NoCandidate, Some(evidence));
    }
    if result.detail["filtered"]
        .as_array()
        .is_none_or(|results| results.len() != 1)
        || result.detail["all"]
            .as_array()
            .is_none_or(|results| results.len() != 1)
    {
        return Recognition::unknown(UnknownReason::Ambiguous, Some(evidence));
    }
    match best["text"].as_str().and_then(parse) {
        Some(value) => Recognition::known(value, evidence),
        None => Recognition::unknown(UnknownReason::NoCandidate, Some(evidence)),
    }
}

fn parse_np(text: &str) -> Option<u16> {
    let digits = text.strip_suffix('%')?;
    if !(1..=3).contains(&digits.len())
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return None;
    }
    digits.parse().ok().filter(|value| *value <= 300)
}

fn parse_hp(text: &str) -> Option<u32> {
    if !matches!(text.as_bytes().first(), Some(b'1'..=b'9')) {
        return None;
    }
    let mut groups = text.split(',');
    let first = groups.next()?;
    if text.contains(',')
        && (!(1..=3).contains(&first.len()) || groups.any(|group| group.len() != 3))
    {
        return None;
    }
    text.bytes().try_fold(0u32, |value, byte| match byte {
        b',' => Some(value),
        b'0'..=b'9' => value.checked_mul(10)?.checked_add(u32::from(byte - b'0')),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_text_requires_whole_ascii_format_and_domain() {
        for (text, expected) in [("0%", 0), ("8%", 8), ("71%", 71), ("300%", 300)] {
            assert_eq!(parse_np(text), Some(expected));
        }
        for text in [
            "",
            "100",
            "00%",
            "030%",
            "301%",
            "1000%",
            " 100%",
            "100%.",
            "１００%",
        ] {
            assert_eq!(parse_np(text), None, "{text}");
        }
        for (text, expected) in [
            ("1414", 1414),
            ("383,032", 383032),
            ("3,152,503", 3152503),
            ("4,294,967,295", u32::MAX),
        ] {
            assert_eq!(parse_hp(text), Some(expected));
        }
        for text in [
            "",
            "0",
            "01414",
            "14,14",
            "1,414,",
            "1,,414",
            "1234,567",
            "1.414",
            "1,414.",
            " 1414",
            "1414 HP",
            "１,４１４",
            "4294967296",
        ] {
            assert_eq!(parse_hp(text), None, "{text}");
        }
    }

    #[test]
    fn native_misses_low_scores_and_multiple_candidates_remain_unknown() {
        let candidate = json!({"text": "100%", "score": 0.999});
        let mut result = NativeRecognition {
            hit: true,
            detail: json!({"best": candidate, "all": [candidate], "filtered": [candidate]}),
            roi: None,
        };
        let read = |result: &NativeRecognition| {
            numeric_reading(
                result,
                Rect {
                    x: 220,
                    y: 655,
                    width: 76,
                    height: 17,
                },
                DetectorKind::NpPercent,
                parse_np,
            )
        };
        assert_eq!(read(&result).value(), Some(100));
        result.hit = false;
        assert!(read(&result).is_unknown());
        result.hit = true;
        result.detail["best"]["score"] = json!(0.979);
        assert!(read(&result).is_unknown());
        result.detail["best"]["score"] = json!(0.999);
        result.detail["best"]["text"] = json!("100%.");
        assert!(read(&result).is_unknown());
        result.detail["best"]["text"] = json!("100%");
        result.detail["all"] = json!([candidate, candidate]);
        assert!(matches!(
            read(&result),
            Recognition::Unknown {
                reason: UnknownReason::Ambiguous,
                ..
            }
        ));
        result.detail["all"] = json!([candidate]);
        result.detail["filtered"] = json!([]);
        assert!(read(&result).is_unknown());
    }
}
