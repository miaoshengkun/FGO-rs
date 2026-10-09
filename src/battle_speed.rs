//! Fixed CN command-card speed glyphs from recorded normal/double-speed frames.
use crate::{
    battle::BattleSpeed,
    layout::UI_LAYOUT_V1,
    observation::{
        DetectorKind, Recognition, RecognitionEvidence, RecognitionMethod, ScoreDirection,
        UnknownReason,
    },
};
use image::RgbaImage;
use std::sync::LazyLock;

const THRESHOLD: f64 = 0.035;
const MARGIN: f64 = 0.04;
static GLYPHS: LazyLock<[Vec<bool>; 2]> = LazyLock::new(|| {
    [
        include_bytes!("../assets/live/battle-speed-normal.png").as_slice(),
        include_bytes!("../assets/live/battle-speed-double.png").as_slice(),
    ]
    .map(|bytes| {
        image::load_from_memory(bytes)
            .expect("embedded speed glyph")
            .to_rgba8()
            .pixels()
            .map(white)
            .collect()
    })
});

fn white(pixel: &image::Rgba<u8>) -> bool {
    let low = pixel[0].min(pixel[1]).min(pixel[2]);
    let high = pixel[0].max(pixel[1]).max(pixel[2]);
    low >= 150 && high - low <= 25
}

pub(crate) fn read_battle_speed(frame: &RgbaImage) -> Recognition<BattleSpeed> {
    let roi = UI_LAYOUT_V1.battle.speed_arrows;
    if roi.validate(frame.width(), frame.height()).is_err() {
        return Recognition::unknown(UnknownReason::NoCandidate, None);
    }
    // Compare the complete binary ROI, including empty space. A missing arrow
    // must never turn a damaged double-speed icon into a normal-speed match.
    let pixels: Vec<_> = (roi.y..roi.y + roi.height)
        .flat_map(|y| (roi.x..roi.x + roi.width).map(move |x| white(frame.get_pixel(x, y))))
        .collect();
    let scores = GLYPHS.each_ref().map(|glyph| {
        pixels.iter().zip(glyph).filter(|(a, b)| a != b).count() as f64 / pixels.len() as f64
    });
    let best = usize::from(scores[1] < scores[0]);
    let evidence = RecognitionEvidence {
        detector: DetectorKind::BattleSpeed,
        roi,
        method: RecognitionMethod::BinaryGlyphDifference,
        score: scores[best],
        threshold: THRESHOLD,
        direction: ScoreDirection::LowerIsBetter,
        feature_floor: None,
    };
    if scores[best] > THRESHOLD {
        Recognition::unknown(UnknownReason::NoCandidate, Some(evidence))
    } else if scores[1 - best] - scores[best] < MARGIN {
        Recognition::unknown(UnknownReason::Ambiguous, Some(evidence))
    } else {
        Recognition::known([BattleSpeed::Normal, BattleSpeed::Double][best], evidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(name: &str) -> RgbaImage {
        let raw = image::open(format!("tests/fixtures/{name}.png"))
            .unwrap()
            .to_rgba8();
        crate::vision::normalize(&raw, None).unwrap().0
    }
    #[test]
    fn recorded_speed_modes_are_distinguished() {
        for name in [
            "owner-continuous-cards",
            "owner-mac-cards",
            "live-owner-buffs",
            "live-speed-normal",
        ] {
            let image = frame(name);
            let started = std::time::Instant::now();
            let reading = read_battle_speed(&image);
            eprintln!("{name}: {:?}, {:?}", reading.evidence(), started.elapsed());
            assert_eq!(reading.value(), Some(BattleSpeed::Normal), "{name}");
        }
        assert_eq!(
            read_battle_speed(&frame("live-speed-double")).value(),
            Some(BattleSpeed::Double)
        );
        let image = frame("step12");
        let started = std::time::Instant::now();
        let reading = read_battle_speed(&image);
        eprintln!("step12: {:?}, {:?}", reading.evidence(), started.elapsed());
        assert_eq!(reading.value(), Some(BattleSpeed::Double));
    }
    #[test]
    fn missing_arrow_blank_and_wrong_scene_are_unknown() {
        let mut broken = frame("step12");
        for y in 44..76 {
            for x in 1139..1167 {
                broken.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(read_battle_speed(&broken).is_unknown());
        assert!(read_battle_speed(&RgbaImage::new(1280, 720)).is_unknown());
        assert!(read_battle_speed(&RgbaImage::new(10, 10)).is_unknown());
        assert!(read_battle_speed(&frame("step6")).is_unknown());
    }
}
