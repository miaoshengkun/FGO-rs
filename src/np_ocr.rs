//! CN NP text completeness and the fixed-font fallback for runs without OCR.
//! Gauge fill and NP-card glow are not numeric evidence.
use crate::{
    observation::{
        DetectorKind, Recognition, RecognitionEvidence, RecognitionMethod, ScoreDirection,
        UnknownReason,
    },
    vision::Rect,
};
use image::RgbaImage;

const HEIGHT: usize = 12;
const THRESHOLD: f64 = 0.12;
const MARGIN: f64 = 0.04;

pub(crate) fn read_np_percent(frame: &RgbaImage, roi: Rect) -> Recognition<u16> {
    let mut evidence = RecognitionEvidence {
        detector: DetectorKind::NpPercent,
        roi,
        method: RecognitionMethod::BinaryGlyphDifference,
        score: 1.0,
        threshold: THRESHOLD,
        direction: ScoreDirection::LowerIsBetter,
        feature_floor: None,
    };
    let unknown = |evidence| Recognition::unknown(UnknownReason::NoCandidate, Some(evidence));
    let Some(NpLayout { pixels, runs }) = np_layout(frame, roi) else {
        return unknown(evidence);
    };
    let width = roi.width as usize;
    let mut value = 0u16;
    let mut worst: f64 = 0.0;
    for (index, &(left, right)) in runs.iter().enumerate() {
        let mut scores = [f64::INFINITY; 11];
        let source_width = right - left;
        for &(digit, rows) in GLYPHS {
            let target_width = rows[0].len();
            let mut mismatch = 0;
            for (y, row) in rows.iter().enumerate() {
                for x in 0..target_width {
                    mismatch += usize::from(
                        pixels[y * width + left + x * source_width / target_width]
                            != (row.as_bytes()[x] == b'#'),
                    );
                }
            }
            let score = mismatch as f64 / (HEIGHT * target_width) as f64
                + source_width.abs_diff(target_width) as f64 * 0.02;
            scores[digit as usize] = scores[digit as usize].min(score);
        }
        let mut ranked: Vec<_> = scores.into_iter().enumerate().collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        let (digit, score) = ranked[0];
        worst = worst.max(score);
        evidence.score = worst;
        if score > THRESHOLD {
            return unknown(evidence);
        }
        if ranked[1].1 - score < MARGIN {
            return Recognition::unknown(UnknownReason::Ambiguous, Some(evidence));
        }
        if index == runs.len() - 1 {
            if digit != 10 {
                return unknown(evidence);
            }
        } else {
            if digit == 10 || (index == 0 && digit == 0 && runs.len() > 2) {
                return unknown(evidence);
            }
            // The narrow final 1 can end one pixel earlier (71% on the
            // live Saber turn-3 frame). Other digits still need the old edge.
            if index == runs.len() - 2 && right == 47 && digit != 1 {
                return unknown(evidence);
            }
            value = value * 10 + digit as u16;
        }
    }
    if value > 300 {
        return unknown(evidence);
    }
    Recognition::known(value, evidence)
}

/// Check the visible text's geometry without classifying any digit. OCR can
/// confidently turn a damaged 300% into 30%; the missing column span rejects it.
#[cfg(feature = "maa")]
pub(crate) fn complete_np_text(frame: &RgbaImage, roi: Rect, value: u16) -> bool {
    let Some(layout) = np_layout(frame, roi) else {
        return false;
    };
    let digit_count = if value < 10 {
        1
    } else if value < 100 {
        2
    } else {
        3
    };
    value <= 300
        && layout.runs.len() == digit_count + 1
        && (layout.runs[digit_count - 1].1 != 47 || value % 10 == 1)
}

struct NpLayout {
    pixels: Vec<bool>,
    runs: Vec<(usize, usize)>,
}

fn np_layout(frame: &RgbaImage, roi: Rect) -> Option<NpLayout> {
    if roi.height != HEIGHT as u32
        || roi.width != 76
        || roi
            .x
            .checked_add(roi.width)
            .is_none_or(|x| x > frame.width())
        || roi
            .y
            .checked_add(roi.height)
            .is_none_or(|y| y > frame.height())
    {
        return None;
    }
    let width = roi.width as usize;
    let mut pixels = vec![false; width * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..width {
            let pixel = frame.get_pixel(roi.x + x as u32, roi.y + y as u32);
            let low = pixel[0].min(pixel[1]).min(pixel[2]);
            let high = pixel[0].max(pixel[1]).max(pixel[2]);
            pixels[y * width + x] = low >= 150 && high - low <= 25;
        }
    }
    // The gauge border can overlap a digit's horizontal span without touching
    // the glyph. Remove its thin connected components before segmentation.
    let mut seen = vec![false; pixels.len()];
    for start in 0..pixels.len() {
        if !pixels[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let mut component = Vec::new();
        let (mut top, mut bottom) = (HEIGHT, 0);
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            top = top.min(y);
            bottom = bottom.max(y);
            component.push(index);
            for ny in y.saturating_sub(1)..=(y + 1).min(HEIGHT - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                    let next = ny * width + nx;
                    if pixels[next] && !seen[next] {
                        seen[next] = true;
                        stack.push(next);
                    }
                }
            }
        }
        if bottom - top < 2 {
            for index in component {
                pixels[index] = false;
            }
        }
    }
    let mut runs = Vec::new();
    let mut start = None;
    for x in 0..=width {
        let on = x < width && (0..HEIGHT).any(|y| pixels[y * width + x]);
        match (start, on) {
            (None, true) => start = Some(x),
            (Some(left), false) => {
                let rows = (0..HEIGHT)
                    .filter(|y| (left..x).any(|column| pixels[y * width + column]))
                    .count();
                // Ignore only thin horizontal gauge edges. Taller unrecognized
                // components must reject the whole number, never disappear.
                if rows > 2 {
                    if rows < 10 {
                        return None;
                    }
                    runs.push((left, x));
                }
                start = None;
            }
            _ => {}
        }
    }
    if !(2..=4).contains(&runs.len()) {
        return None;
    }
    let &(suffix_start, suffix_end) = runs.last().unwrap();
    if !(50..=52).contains(&suffix_start)
        || !(68..=70).contains(&suffix_end)
        || !(47..=50).contains(&runs[runs.len() - 2].1)
        || runs
            .windows(2)
            .any(|pair| !(1..=5).contains(&(pair[1].0 - pair[0].1)))
    {
        return None;
    }
    Some(NpLayout { pixels, runs })
}
// CN NP text, top 12 rows above the gauge border. Independent of enemy HP fonts.
// Samples: step4/5/9/10 and live-enemy-standard-one/two/three fixtures.
const GLYPHS: &[(u8, [&str; HEIGHT])] = &[
    (
        2,
        [
            // live-saber-retest-turn3, field 3 (220%), middle digit 2
            "....###....",
            "..#######..",
            ".###....##.",
            "###.....##.",
            "........###",
            "........##.",
            "........##.",
            "......###..",
            ".....###...",
            "....###....",
            "...###.....",
            "..##.......",
        ],
    ),
    (
        3,
        [
            // live-saber-turn1-charged, field 1 (34%)
            "....###....",
            "..#######..",
            ".##....###.",
            "###.....##.",
            "........##.",
            "........##.",
            "......###..",
            "....####...",
            ".....####..",
            "........##.",
            "........###",
            "........###",
        ],
    ),
    (
        0,
        [
            // step4, field 1
            "....##.....",
            "..#######..",
            ".###...###.",
            ".##.....##.",
            "##......##.",
            "##......###",
            "##......###",
            "##......###",
            "##......###",
            "##......###",
            "##......###",
            "##......##.",
        ],
    ),
    (
        1,
        [
            // step10, field 1
            "......", "....##", "...###", ".#####", "###.##", "#...##", "....##", "....##",
            "....##", "....##", "....##", "....##",
        ],
    ),
    (
        2,
        [
            // step5, field 1
            "....##....",
            ".########.",
            "###....##.",
            "##.....###",
            "........##",
            ".......###",
            ".......##.",
            "......###.",
            "....####..",
            "...###....",
            "..###.....",
            ".###......",
        ],
    ),
    (
        3,
        [
            // live-enemy-standard-one, field 1
            "...###....",
            ".#######..",
            "###....##.",
            "##.....###",
            ".......###",
            ".......##.",
            "......##..",
            "...#####..",
            ".....####.",
            ".......###",
            "........##",
            "........##",
        ],
    ),
    (
        4,
        [
            // step10, field 3
            "...........",
            ".......##..",
            "......###..",
            ".....####..",
            "....#####..",
            "....##.##..",
            "...##..##..",
            "..##...##..",
            ".##....##..",
            "###....##..",
            "###...#####",
            "###########",
        ],
    ),
    (
        5,
        [
            // step5, field 1
            "..........",
            "##########",
            "###.......",
            "###.......",
            "###.......",
            "###.......",
            "#######...",
            ".....####.",
            ".......###",
            "........##",
            "........##",
            "........##",
        ],
    ),
    (
        6,
        [
            // live-enemy-standard-three, field 1
            ".....##....",
            "..#######..",
            ".###....##.",
            ".##......#.",
            "##.........",
            "##.........",
            "###.####...",
            "#####.####.",
            "###.....##.",
            "##......###",
            "##.......##",
            "##.......##",
        ],
    ),
    (
        7,
        [
            // step4, field 2
            "...........",
            ".##########",
            "........###",
            "........##.",
            ".......##..",
            ".......##..",
            "......##...",
            "......##...",
            ".....##....",
            ".....##....",
            "....###....",
            "#...##...##",
        ],
    ),
    (
        8,
        [
            // step4, field 3
            "....###....",
            ".########..",
            "###....###.",
            "##......##.",
            "##......##.",
            "###.....##.",
            ".####.###..",
            "..#######..",
            ".########..",
            "###.....##.",
            "##......###",
            "##.......##",
        ],
    ),
    (
        9,
        [
            // live-enemy-standard-two, field 1
            "....###....",
            "..#######..",
            ".##....###.",
            "###.....##.",
            "##......###",
            "##......###",
            "###.....###",
            ".##....####",
            ".##########",
            "...####.###",
            "........###",
            "........##.",
        ],
    ),
    (
        10,
        [
            // step4, field 1
            "..###.......##....",
            ".##.###.....#.....",
            ".#...##....##.....",
            "##....#...##......",
            "##...##...##......",
            ".##..##..##.......",
            ".#####...##.......",
            "........##........",
            "........##...###..",
            ".......##...##.##.",
            ".......##..##...##",
            "#.#...##...##...##",
        ],
    ),
    (
        0,
        [
            // step4, field 2
            ".....##....",
            "...######..",
            "..##....##.",
            ".##.....##.",
            ".##......##",
            "###......##",
            "###......##",
            "###......##",
            "###......##",
            "###......##",
            "###......##",
            ".##......##",
        ],
    ),
    (
        8,
        [
            // step9, field 1
            "....###....",
            "..########.",
            ".###....##.",
            ".##......##",
            ".##......##",
            ".##.....###",
            "..###.####.",
            "...######..",
            "..########.",
            ".##.....###",
            "###......##",
            "###......##",
        ],
    ),
    (
        4,
        [
            // live-enemy-standard-two, field 3
            "............",
            ".......###..",
            "......####..",
            "......####..",
            ".....#####..",
            "....##..##..",
            "...##...##..",
            "..##....##..",
            "..##....##..",
            ".##.....##..",
            "####...####.",
            ".###########",
        ],
    ),
    (
        6,
        [
            // live-enemy-standard-two, field 2
            ".....###...",
            "...#######.",
            "..##....##.",
            ".##......#.",
            ".##........",
            ".##........",
            ".##.#####..",
            "#####..###.",
            "####....###",
            ".##......##",
            ".##......##",
            ".##......##",
        ],
    ),
    (
        10,
        [
            // step4, field 2
            "..###.......#....",
            "###.##.....##....",
            "##...#....##.....",
            "#....##...##.....",
            "##...##..##......",
            "##..##...##......",
            ".#####..##.......",
            "........##.......",
            ".......##...####.",
            ".......##..######",
            "......##..##...##",
            "#.....#...##...##",
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{layout::UI_LAYOUT_V1, vision::normalize};
    use std::path::Path;

    fn frame(name: &str) -> RgbaImage {
        let raw = image::open(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.png")),
        )
        .unwrap()
        .to_rgba8();
        normalize(&raw, None).unwrap().0
    }

    #[test]
    fn real_np_corpus_covers_every_digit_zero_and_three_hundred() {
        for (name, expected) in [
            ("live-saber-retest-turn2", [Some(34), Some(53), Some(92)]),
            ("live-saber-retest-turn3", [Some(76), Some(77), Some(220)]),
            ("live-saber-turn1", [Some(14), Some(15), Some(80)]),
            ("live-saber-turn1-charged", [Some(34), Some(35), Some(160)]),
            ("live-saber-turn3", [Some(72), Some(71), Some(123)]),
            ("step4", [Some(0), Some(70), Some(78)]),
            ("step5", [Some(25), Some(70), Some(78)]),
            ("step9", [Some(82), Some(70), Some(78)]),
            ("step10", [Some(100), Some(100), Some(148)]),
            ("live-enemy-standard-one", [Some(300), None, None]),
            ("live-enemy-standard-two", [Some(90), Some(64), Some(74)]),
            ("live-enemy-standard-three", [Some(60), Some(24), Some(0)]),
            ("live-enemy-reflow-one", [Some(8), None, Some(32)]),
            ("owner-mac-turn", [Some(100), Some(100), Some(148)]),
            ("owner-continuous-turn", [Some(100), Some(100), Some(148)]),
        ] {
            let frame = frame(name);
            let values = UI_LAYOUT_V1
                .battle
                .np_percent
                .map(|roi| read_np_percent(&frame, roi));
            assert_eq!(
                values.map(Recognition::value),
                expected,
                "{name}: {values:?}"
            );
        }
    }

    #[test]
    fn rejects_out_of_range_and_leading_zero_in_otherwise_clear_text() {
        let source = frame("live-enemy-standard-one");
        let roi = UI_LAYOUT_V1.battle.np_percent[0];
        // Replace the final 0 of an actual 300% with the 4 from another real
        // frame. All glyphs remain readable, but 304 is outside the NP domain.
        let four = frame("live-enemy-standard-three");
        let mut invalid = source.clone();
        for y in 660..672 {
            for dx in 0..11 {
                invalid.put_pixel(258 + dx, y, *four.get_pixel(575 + dx, y));
            }
        }
        assert!(read_np_percent(&invalid, roi).is_unknown());
        // Removing just the leading 3 leaves 00%, which must not become zero.
        let mut leading_zero = source;
        for y in 660..672 {
            for x in 231..243 {
                leading_zero.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        assert!(read_np_percent(&leading_zero, roi).is_unknown());
    }

    #[test]
    fn rejects_missing_suffix_dim_text_corrupt_digits_and_invalid_bounds() {
        let frame = frame("step10");
        let roi = UI_LAYOUT_V1.battle.np_percent[0];
        for (x, width) in [(271, 24), (245, 5)] {
            let mut damaged = frame.clone();
            for y in 660..672 {
                for x in x..x + width {
                    damaged.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
            assert!(read_np_percent(&damaged, roi).is_unknown());
        }
        let mut dim = frame;
        for pixel in dim.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                *channel /= 3;
            }
        }
        assert!(read_np_percent(&dim, roi).is_unknown());
        assert!(read_np_percent(&RgbaImage::new(10, 10), roi).is_unknown());
        assert!(read_np_percent(&dim, Rect { x: u32::MAX, ..roi }).is_unknown());
    }
}
