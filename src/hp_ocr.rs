//! CN ally HP text; independent of enemy HP and NP fonts.
use crate::{
    digit_ocr::{Component, white_components},
    vision::Rect,
};
use image::RgbaImage;

/// CN ally HP: outlined 15-pixel digits, with a comma before each last three.
pub(crate) fn read_ally_hp(frame: &RgbaImage, region: Rect) -> Option<(u32, f64)> {
    if region.width != 98 || region.height != 18 {
        return None;
    }
    let components = white_components(frame, region, 0, 18)?;
    let mut digits: Vec<_> = components
        .iter()
        .filter(|c| c.min_y <= 2 && c.max_y >= 16)
        .collect();
    // Full-height unexpected shapes may be a merged/occluded digit.
    if digits.iter().any(|c| {
        c.max_y != 16 || !(4..=13).contains(&(c.max_x - c.min_x)) || c.min_x == 0 || c.max_x == 98
    }) {
        return None;
    }
    digits.sort_by_key(|c| c.min_x);
    if !(1..=6).contains(&digits.len()) {
        return None;
    }
    let mut result = 0u32;
    let mut score: f64 = 0.0;
    for (index, c) in digits.iter().enumerate() {
        let (digit, confidence) = classify_ally_hp(c)?;
        if index == 0 && digit == 0 && digits.len() > 1 {
            return None;
        }
        score = score.max(confidence);
        result = result * 10 + digit as u32;
    }
    // Validate grouping and the right edge to avoid accepting a partial prefix.
    if !(87..=91).contains(&digits.last()?.max_x) {
        return None;
    }
    for (i, pair) in digits.windows(2).enumerate() {
        let gap = pair[1].min_x.checked_sub(pair[0].max_x)?;
        let comma = digits.len() - i - 1 == 3;
        if comma {
            if !(5..=10).contains(&gap)
                || !components.iter().any(|c| {
                    c.min_y >= 12
                        && c.max_y >= 17
                        && c.max_x - c.min_x <= 3
                        && c.min_x >= pair[0].max_x
                        && c.max_x <= pair[1].min_x
                })
            {
                return None;
            }
        } else if !(1..=5).contains(&gap) {
            return None;
        }
    }
    Some((result, score))
}

fn classify_ally_hp(component: &Component) -> Option<(u8, f64)> {
    let width = component.max_x - component.min_x;
    let height = component.max_y - component.min_y;
    let mut source = vec![false; width * height];
    for &(x, y) in &component.points {
        source[(y - component.min_y) * width + x - component.min_x] = true;
    }
    let mut scores = [f64::INFINITY; 10];
    for &(value, rows) in ALLY_HP_GLYPHS {
        let tw = rows[0].len();
        let mut mismatch = 0;
        for (y, row) in rows.iter().enumerate() {
            for x in 0..tw {
                mismatch += usize::from(
                    source[y * height / rows.len() * width + x * width / tw]
                        != (row.as_bytes()[x] == b'#'),
                );
            }
        }
        let score = mismatch as f64 / (tw * rows.len()) as f64 + width.abs_diff(tw) as f64 * 0.02;
        scores[value as usize] = scores[value as usize].min(score);
    }
    let mut ranked: Vec<_> = scores.into_iter().enumerate().collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    (ranked[0].1 <= 0.12 && ranked[1].1 - ranked[0].1 >= 0.04)
        .then_some((ranked[0].0 as u8, ranked[0].1))
}
// Independent CN ally HP font samples; fixture name and field identify provenance.
const ALLY_HP_GLYPHS: &[(u8, &[&str])] = &[
    (
        4,
        &[
            // live-saber-retest-turn2, field 3 (17462), digit 4
            "......###..",
            "......###..",
            ".....####..",
            "....#####..",
            "...##..##..",
            "...##..##..",
            "..##...##..",
            ".##....##..",
            "###....##..",
            "###########",
            "###########",
            ".......##..",
            ".......##..",
            ".......##..",
        ],
    ),
    (
        7,
        &[
            // live-saber-turn1, field 1 (14907)
            ".#........",
            "##########",
            ".......###",
            ".......##.",
            "......##..",
            "......##..",
            ".....##...",
            ".....##...",
            "....##....",
            "....##....",
            "...###....",
            "...##.....",
            "...##.....",
            "..###.....",
            "..##......",
        ],
    ),
    (
        6,
        &[
            // live-enemy-standard-three, field 1
            "....####...",
            "..#######..",
            ".##....###.",
            "###........",
            "##.........",
            "##.........",
            "########...",
            "####..####.",
            "###.....##.",
            "##......###",
            "##......###",
            "##......###",
            ".##.....##.",
            ".###...###.",
            "..######...",
        ],
    ),
    (
        1,
        &[
            // step10, field 1
            ".....#.", "....###", "...####", "..#####", "###.###", "....###", "....###", "....###",
            "....###", "....###", "....###", "....###", "....###", "....###", ".....#.",
        ],
    ),
    (
        7,
        &[
            // step10, field 1
            "##########",
            "........##",
            ".......##.",
            ".......##.",
            "......##..",
            ".....###..",
            ".....##...",
            "....###...",
            "....##....",
            "....##....",
            "...###....",
            "...##.....",
            "...##.....",
            "...##.....",
        ],
    ),
    (
        7,
        &[
            // step10, field 1
            "##########",
            ".......###",
            ".......##.",
            "......##..",
            "......##..",
            ".....##...",
            ".....##...",
            "....##....",
            "....##....",
            "...###....",
            "...##.....",
            "...##.....",
            "..###.....",
            "..##......",
        ],
    ),
    (
        9,
        &[
            // step10, field 1
            "...####....",
            "..#######..",
            ".##....###.",
            "###.....##.",
            "##......###",
            "##......###",
            "###.....###",
            ".##....####",
            "..#########",
            "....##..###",
            "........###",
            "........##.",
            ".......###.",
            ".########..",
            "..######...",
        ],
    ),
    (
        3,
        &[
            // step10, field 1
            "...####...",
            ".########.",
            "###....###",
            "##.....###",
            ".......###",
            ".......##.",
            "......###.",
            "...#####..",
            "......###.",
            ".......###",
            "........##",
            "........##",
            "#......###",
            "##########",
            "..######..",
        ],
    ),
    (
        1,
        &[
            // step4, field 1
            "....#.", "...###", "..####", ".#####", "##.###", "....##", "....##", "....##",
            "....##", "....##", "....##", "....##", "....##", "....##", "....##",
        ],
    ),
    (
        4,
        &[
            // step4, field 1
            "........#..",
            ".......###.",
            "......####.",
            ".....#####.",
            "....######.",
            "....##..##.",
            "...##...##.",
            "..##....##.",
            ".##.....##.",
            ".##....###.",
            "###########",
            ".##########",
            ".......###.",
            "........##.",
            "........#..",
        ],
    ),
    (
        2,
        &[
            // step4, field 1
            "....####...",
            "..#######..",
            ".##.....##.",
            "###.....##.",
            "........###",
            "........##.",
            ".......###.",
            "......###..",
            ".....###...",
            "....###....",
            "...##......",
            "..##.......",
            ".##........",
            "####.......",
            "##########.",
        ],
    ),
    (
        9,
        &[
            // step4, field 1
            "...####...",
            ".########.",
            "###....##.",
            "##......##",
            "##......##",
            "##......##",
            "##.....###",
            "###....###",
            ".#########",
            "....#...##",
            "........##",
            "........##",
            ".......##.",
            "#########.",
            ".#######..",
        ],
    ),
    (
        1,
        &[
            // step4, field 2
            ".....#.", "....###", "...####", "..#####", "###.###", "....###", "....###", "....###",
            "....###", "....###", "....###", "....###", "....###", "....###", ".....##",
        ],
    ),
    (
        6,
        &[
            // step4, field 2
            "....####...",
            "..########.",
            ".###....##.",
            ".##........",
            ".##........",
            "###........",
            "###.#####..",
            "#####..###.",
            "###.....###",
            "###......##",
            "###......##",
            ".##......##",
            ".##.....###",
            "..###..###.",
            "...######..",
        ],
    ),
    (
        0,
        &[
            // step4, field 2
            "....###....",
            "..#######..",
            ".###...###.",
            ".##.....##.",
            "###.....###",
            "##.......##",
            "##.......##",
            "##.......##",
            "##.......##",
            "##.......##",
            "##.......##",
            "###.....##.",
            ".##.....##.",
            "..##...###.",
            "...#####...",
        ],
    ),
    (
        5,
        &[
            // step4, field 2
            ".########.",
            "#########.",
            "##........",
            "##........",
            "##........",
            "###.......",
            "#######...",
            ".....####.",
            ".......###",
            "........##",
            "........##",
            "........##",
            "#......###",
            "#########.",
            "..######..",
        ],
    ),
    (
        2,
        &[
            // step4, field 3
            "...####...",
            "..#######.",
            ".##....###",
            "##......##",
            "........##",
            "........##",
            ".......###",
            "......###.",
            ".....###..",
            "....##....",
            "..###.....",
            ".###......",
            ".##.......",
            "###.......",
            "##########",
        ],
    ),
    (
        5,
        &[
            // step4, field 3
            ".########.",
            "#########.",
            "##........",
            "##........",
            "##........",
            "##........",
            "#######...",
            ".....####.",
            ".......##.",
            ".......###",
            ".......###",
            ".......###",
            ".......##.",
            "#########.",
            ".#######..",
        ],
    ),
    (
        1,
        &[
            // live-enemy-standard-one, field 1
            ".....#", "...###", "...###", ".#####", "###.##", "....##", "....##", "....##",
            "....##", "....##", "....##", "....##", "....##", "....##", "....##",
        ],
    ),
    (
        2,
        &[
            // live-enemy-standard-one, field 1
            "...####...",
            "..#######.",
            ".##....###",
            "##......##",
            "........##",
            "........##",
            ".......###",
            "......###.",
            ".....###..",
            "....##....",
            "..###.....",
            ".###......",
            "###.......",
            "###.......",
            "##########",
        ],
    ),
    (
        8,
        &[
            // live-enemy-standard-one, field 1
            "....####...",
            "..########.",
            ".##.....###",
            ".##......##",
            ".##......##",
            ".###....###",
            "..########.",
            "..#######..",
            "..###.####.",
            ".##.....###",
            "###......##",
            "###......##",
            "###......##",
            ".###....###",
            "..########.",
        ],
    ),
    (
        3,
        &[
            // live-enemy-standard-one, field 1
            "...####....",
            ".########..",
            "###....###.",
            "##......##.",
            "........##.",
            ".......###.",
            "......###..",
            "...#####...",
            "......###..",
            "........##.",
            "........###",
            "........###",
            "#.......##.",
            ".#########.",
            "..#######..",
        ],
    ),
    (
        0,
        &[
            // live-enemy-standard-one, field 1
            "...####....",
            "..######...",
            ".##....##..",
            "##.....###.",
            "##......##.",
            "##......##.",
            "##......##.",
            "##......###",
            "##......##.",
            "##......##.",
            "##......##.",
            "##......##.",
            ".##....###.",
            ".###..###..",
            "..######...",
        ],
    ),
    (
        8,
        &[
            // live-enemy-standard-two, field 2
            "...####....",
            ".########..",
            "###....###.",
            "##......##.",
            "##......##.",
            "###....###.",
            ".########..",
            "..######...",
            ".########..",
            "##.....###.",
            "##......###",
            "##......###",
            "##......##.",
            "###....###.",
            ".########..",
        ],
    ),
    (
        5,
        &[
            // live-enemy-standard-two, field 2
            ".########.",
            "#########.",
            "##........",
            "##........",
            "##........",
            "##........",
            "#######...",
            ".....####.",
            ".......##.",
            ".......###",
            ".......###",
            ".......###",
            "#......##.",
            "#########.",
            ".#######..",
        ],
    ),
];

#[cfg(test)]
mod hp_tests {
    use super::*;
    #[test]
    fn ally_hp_rejects_missing_comma_damaged_digit_dim_text_and_invalid_roi() {
        let raw = image::open("tests/fixtures/step4.png").unwrap().to_rgba8();
        let frame = crate::vision::normalize(&raw, None).unwrap().0;
        let roi = Rect {
            x: 200,
            y: 623,
            width: 98,
            height: 18,
        };
        for (x, w, y, h) in [(247, 7, 635, 6), (262, 5, 623, 18)] {
            let mut damaged = frame.clone();
            for y in y..y + h {
                for x in x..x + w {
                    damaged.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
            assert!(read_ally_hp(&damaged, roi).is_none());
        }
        let mut dim = frame;
        for p in dim.pixels_mut() {
            for c in &mut p.0[..3] {
                *c /= 3;
            }
        }
        assert!(read_ally_hp(&dim, roi).is_none());
        assert!(read_ally_hp(&RgbaImage::new(10, 10), roi).is_none());
        assert!(read_ally_hp(&dim, Rect { x: u32::MAX, ..roi }).is_none());
    }

    #[test]
    fn ally_hp_real_frames() {
        for (name, expected) in [
            ("live-saber-retest-turn2", [15268, 10848, 17462]),
            ("live-saber-retest-turn3", [8427, 9668, 16372]),
            ("live-saber-turn1", [14907, 15380, 20196]),
            ("live-saber-turn1-charged", [15268, 15380, 20196]),
            ("live-saber-turn3", [5108, 6255, 11738]),
            ("step4", [14219, 16057, 21455]),
            ("step10", [17793, 16057, 20455]),
            ("owner-mac-turn", [17793, 16057, 20455]),
            ("live-enemy-standard-three", [16457, 14985, 16146]),
            ("live-enemy-standard-two", [16457, 14985, 15363]),
            ("owner-continuous-turn", [17793, 16057, 20455]),
        ] {
            let raw = image::open(format!("tests/fixtures/{name}.png"))
                .unwrap()
                .to_rgba8();
            let frame = crate::vision::normalize(&raw, None).unwrap().0;
            let got = std::array::from_fn::<_, 3, _>(|i| {
                read_ally_hp(
                    &frame,
                    Rect {
                        x: 200 + 318 * i as u32,
                        y: 623,
                        width: 98,
                        height: 18,
                    },
                )
            });
            assert_eq!(
                got.map(|v| v.map(|v| v.0)),
                expected.map(Some),
                "{name}: {got:?}"
            );
        }
    }
}
