use crate::vision::Rect;
use image::RgbaImage;

const HEIGHT: usize = 13;

struct Glyph {
    value: u8,
    rows: [&'static str; HEIGHT],
}

const GLYPHS: [Glyph; 10] = [
    Glyph {
        value: 0,
        rows: [
            "...###...",
            ".###.##..",
            "##....##.",
            "##.....#.",
            "##.....##",
            "##.....##",
            "##.....##",
            "##.....##",
            "##.....##",
            "##.....##",
            "##....##.",
            ".##..###.",
            "..#####..",
        ],
    },
    Glyph {
        value: 1,
        rows: [
            "...#.", "..###", ".####", "#####", "#..##", "...##", "...##", "...##", "...##",
            "...##", "...##", "...##", "...##",
        ],
    },
    Glyph {
        value: 2,
        rows: [
            "..#####..",
            ".##..###.",
            "##.....##",
            "#......##",
            ".......##",
            "......##.",
            ".....##..",
            "...###...",
            "..###....",
            ".##......",
            "###......",
            "###....#.",
            "########.",
        ],
    },
    Glyph {
        value: 3,
        rows: [
            "..#####..",
            ".##..###.",
            "##....##.",
            ".......#.",
            "......##.",
            "....###..",
            "...####..",
            "......##.",
            ".......##",
            ".......##",
            "#.....###",
            "########.",
            "..#####..",
        ],
    },
    Glyph {
        value: 4,
        rows: [
            ".......#..",
            "......##..",
            ".....###..",
            "....####..",
            "....#.##..",
            "...#...#..",
            "..##...#..",
            ".##....#..",
            "###...###.",
            "##########",
            "......###.",
            ".......#..",
            ".......#..",
        ],
    },
    Glyph {
        value: 5,
        rows: [
            "########.",
            "###......",
            "##.......",
            "##.......",
            "##.......",
            "#######..",
            ".....###.",
            ".......##",
            ".......##",
            ".......##",
            "#.....###",
            "########.",
            "..#####..",
        ],
    },
    Glyph {
        value: 6,
        rows: [
            "...####..",
            "..#######",
            ".##.....#",
            "##.......",
            "##.......",
            "#######..",
            "####..###",
            "###....##",
            "##.....##",
            "##.....##",
            ".##....##",
            ".###..##.",
            "...####..",
        ],
    },
    Glyph {
        value: 7,
        rows: [
            ".......#", "########", "......##", ".....##.", ".....##.", "....##..", "....#...",
            "...##...", "...##...", "..##....", "..##....", "..##....", "..#.....",
        ],
    },
    Glyph {
        value: 8,
        rows: [
            "...####...",
            "..##..###.",
            ".##....##.",
            ".##.....##",
            ".##....##.",
            "..#######.",
            "..######..",
            ".###..###.",
            ".##....###",
            "##......##",
            "##......##",
            ".##....##.",
            "..######..",
        ],
    },
    Glyph {
        value: 9,
        rows: [
            "..####...",
            ".##..###.",
            "##....##.",
            "##.....##",
            "##.....##",
            "##....###",
            "###..####",
            "..#######",
            ".......##",
            ".......##",
            "#.....##.",
            "#######..",
            "..####...",
        ],
    },
];

// Native 1280x720 captures rasterize a few digits differently from screenshots
// downscaled from 1920x1080. Keep those variants explicit instead of relaxing
// the foreground threshold for every UI element.
const NATIVE_GLYPHS: [Glyph; 3] = [
    Glyph {
        value: 5,
        rows: [
            ".#######", "#######.", "##......", "##......", "##......", "######..", ".....###",
            "......##", "......##", ".......#", "......##", "###.####", ".######.",
        ],
    },
    Glyph {
        value: 6,
        rows: [
            "...###...",
            ".#######.",
            "##....##.",
            "##.......",
            "##.......",
            "##.####..",
            "###..###.",
            "##....##.",
            "##.....##",
            "##.....##",
            "##.....#.",
            ".##...##.",
            "..#####..",
        ],
    },
    Glyph {
        value: 9,
        rows: [
            "...####...",
            "..##.###..",
            ".##....##.",
            "##.....##.",
            "##......##",
            "##.....###",
            ".##...####",
            "..########",
            "........##",
            "........#.",
            ".......##.",
            ".##..###..",
            "..#####...",
        ],
    },
];

#[derive(Debug)]
pub(crate) struct Component {
    pub(crate) min_x: usize,
    pub(crate) max_x: usize,
    pub(crate) min_y: usize,
    pub(crate) max_y: usize,
    pub(crate) points: Vec<(usize, usize)>,
}

pub(crate) fn read_white_digits(frame: &RgbaImage, region: Rect, row_offset: u32) -> Option<u32> {
    read_digits(frame, region, row_offset, HEIGHT, 4, 10, 25)
}

pub(crate) fn read_large_white_digits(
    frame: &RgbaImage,
    region: Rect,
    row_offset: u32,
) -> Option<u32> {
    read_digits(frame, region, row_offset, 16, 5, 13, 35)
}

fn read_digits(
    frame: &RgbaImage,
    region: Rect,
    row_offset: u32,
    source_height: usize,
    min_width: usize,
    max_width: usize,
    min_points: usize,
) -> Option<u32> {
    let mut components: Vec<_> = white_components(frame, region, row_offset, source_height)?
        .into_iter()
        .filter(|c| {
            c.min_y <= 1
                && c.max_y == source_height
                && (min_width..=max_width).contains(&(c.max_x - c.min_x))
                && c.points.len() >= min_points
        })
        .collect();
    components.sort_by_key(|component| component.min_x);
    if !(1..=7).contains(&components.len()) {
        return None;
    }

    let mut result = 0u32;
    for component in components {
        let digit = classify(&component)?;
        result = result.checked_mul(10)?.checked_add(digit as u32)?;
    }
    (result > 0).then_some(result)
}

pub(crate) fn white_components(
    frame: &RgbaImage,
    region: Rect,
    row_offset: u32,
    source_height: usize,
) -> Option<Vec<Component>> {
    if row_offset
        .checked_add(source_height as u32)
        .is_none_or(|v| v > region.height)
        || region
            .x
            .checked_add(region.width)
            .is_none_or(|v| v > frame.width())
        || region
            .y
            .checked_add(region.height)
            .is_none_or(|v| v > frame.height())
    {
        return None;
    }
    let width = region.width as usize;
    let mut foreground = vec![false; width * source_height];
    for y in 0..source_height {
        for x in 0..width {
            let pixel = frame.get_pixel(region.x + x as u32, region.y + row_offset + y as u32);
            let low = pixel[0].min(pixel[1]).min(pixel[2]);
            let high = pixel[0].max(pixel[1]).max(pixel[2]);
            foreground[y * width + x] = low >= 150 && high - low <= 25;
        }
    }

    let mut seen = vec![false; foreground.len()];
    let mut components = Vec::new();
    for start_y in 0..source_height {
        for start_x in 0..width {
            let start = start_y * width + start_x;
            if !foreground[start] || seen[start] {
                continue;
            }
            seen[start] = true;
            let mut stack = vec![(start_x, start_y)];
            let mut points = Vec::new();
            while let Some((x, y)) = stack.pop() {
                points.push((x, y));
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let next_x = x as isize + dx;
                        let next_y = y as isize + dy;
                        if next_x < 0
                            || next_x >= width as isize
                            || next_y < 0
                            || next_y >= source_height as isize
                        {
                            continue;
                        }
                        let next = next_y as usize * width + next_x as usize;
                        if foreground[next] && !seen[next] {
                            seen[next] = true;
                            stack.push((next_x as usize, next_y as usize));
                        }
                    }
                }
            }
            let min_x = points.iter().map(|point| point.0).min()?;
            let max_x = points.iter().map(|point| point.0).max()? + 1;
            let min_y = points.iter().map(|point| point.1).min()?;
            let max_y = points.iter().map(|point| point.1).max()? + 1;
            components.push(Component {
                min_x,
                max_x,
                min_y,
                max_y,
                points,
            });
        }
    }
    Some(components)
}

fn classify(component: &Component) -> Option<u8> {
    let width = component.max_x - component.min_x;
    let height = component.max_y - component.min_y;
    let mut source = vec![false; width * height];
    for &(x, y) in &component.points {
        source[(y - component.min_y) * width + x - component.min_x] = true;
    }
    let mut scores = [(f64::INFINITY, 0u8); 10];
    for glyph in GLYPHS.iter().chain(NATIVE_GLYPHS.iter()) {
        let target_width = glyph.rows[0].len();
        let mut mismatches = 0;
        for y in 0..HEIGHT {
            for x in 0..target_width {
                let source_x = x * width / target_width;
                let source_y = y * height / HEIGHT;
                let expected = glyph.rows[y].as_bytes()[x] == b'#';
                mismatches += usize::from(source[source_y * width + source_x] != expected);
            }
        }
        let pixel_score = mismatches as f64 / (target_width * HEIGHT) as f64;
        let aspect_score = width.abs_diff(target_width) as f64 / target_width as f64;
        let score = pixel_score + aspect_score * 0.15;
        let entry = &mut scores[glyph.value as usize];
        if score < entry.0 {
            *entry = (score, glyph.value);
        }
    }
    let mut scores = scores.to_vec();
    scores.sort_by(|left, right| left.0.total_cmp(&right.0));
    let (best_score, value) = scores[0];
    (best_score <= 0.38 && scores[1].0 - best_score >= 0.015).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision::normalize;
    use std::path::PathBuf;

    #[test]
    fn reads_all_six_enemy_hp_values_from_real_frame() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let raw = image::open(root.join("tests/fixtures/step4.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        let regions = [
            Rect {
                x: 190,
                y: 28,
                width: 97,
                height: 25,
            },
            Rect {
                x: 390,
                y: 28,
                width: 97,
                height: 25,
            },
            Rect {
                x: 590,
                y: 28,
                width: 97,
                height: 25,
            },
            Rect {
                x: 90,
                y: 127,
                width: 97,
                height: 25,
            },
            Rect {
                x: 290,
                y: 127,
                width: 97,
                height: 25,
            },
            Rect {
                x: 490,
                y: 127,
                width: 97,
                height: 25,
            },
        ];
        assert_eq!(
            regions.map(|region| read_white_digits(&frame, region, 6)),
            [
                Some(704_487),
                Some(735_728),
                Some(351_884),
                Some(353_590),
                Some(355_298),
                Some(3_152_503),
            ]
        );
    }

    #[test]
    fn reads_native_resolution_enemy_hp_after_other_enemies_leave() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let raw = image::open(root.join("tests/fixtures/live-enemy-reflow-one.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        assert_eq!(
            read_white_digits(
                &frame,
                Rect {
                    x: 190,
                    y: 28,
                    width: 97,
                    height: 25,
                },
                6,
            ),
            Some(145_196)
        );
    }

    #[test]
    fn reads_large_standard_layout_enemy_hp() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let raw = image::open(root.join("tests/fixtures/live-enemy-standard-one.png"))
            .unwrap()
            .to_rgba8();
        let frame = normalize(&raw, None).unwrap().0;
        assert_eq!(
            read_large_white_digits(
                &frame,
                Rect {
                    x: 350,
                    y: 40,
                    width: 122,
                    height: 25,
                },
                5,
            ),
            Some(252_064)
        );
    }

    #[test]
    fn reads_real_two_and_three_enemy_standard_hp() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let read = |name: &str| {
            let raw = image::open(root.join("tests/fixtures").join(name))
                .unwrap()
                .to_rgba8();
            let frame = normalize(&raw, None).unwrap().0;
            crate::layout::UI_LAYOUT_V1
                .battle
                .enemy_hp_standard
                .map(|region| read_large_white_digits(&frame, region, 5))
        };

        assert_eq!(
            read("live-enemy-standard-two.png"),
            [None, Some(1_414), Some(2_340)]
        );
        assert_eq!(
            read("live-enemy-standard-three.png"),
            [Some(1_414), Some(1_344), Some(1_414)]
        );
    }
}
