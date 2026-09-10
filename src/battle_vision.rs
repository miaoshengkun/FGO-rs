use crate::{
    battle::{BattleState, Color},
    control::Control,
    vision::Vision,
};
use anyhow::Result;
use image::RgbaImage;

impl Vision {
    /// CN battle facts computed from one normalized frame; no device input.
    pub fn battle_state(&self, frame: &RgbaImage, control: &Control) -> Result<BattleState> {
        Self::require_canvas(frame)?;
        let has = |name: &str| -> Result<bool> { Ok(self.find(frame, name, control)?.is_some()) };
        let mut s = BattleState {
            turn: has("turn_begin")?,
            first_turn: has("first_turn")?,
            iskandar: has("iskandar")?,
            taigong: has("taigong")?,
            reines: has("reines")?,
            support: has("support_slot_3")?,
            order_change: has("order_change")?,
            master_open: has("master_ready")? || has("master_used")?,
            skill_error: has("skill_error")? || has("skill_none_1")? || has("skill_none_2")?,
            victory: has("victory")? || (self.has_template("bond_result") && has("bond_result")?),
            defeated: has("defeated")?,
            resist: [1.0; 5],
            ..Default::default()
        };
        if s.order_change {
            for i in 0..6 {
                s.selected[i] = has(&format!("selected_{}", i + 1))?;
            }
        } else if has("target_cross")? && !s.skill_error {
            if has("target_header")? {
                s.target_count = target_regions(frame);
            } else {
                s.skill_error = true;
            }
        }
        for i in 0..5 {
            let mut best = None;
            for (name, color) in [
                ("arts", Color::Arts),
                ("quick", Color::Quick),
                ("buster", Color::Buster),
            ] {
                if let Some(m) = self.find(frame, &format!("{name}_{}", i + 1), control)?
                    && best.is_none_or(|(score, _)| m.score < score)
                {
                    best = Some((m.score, color));
                }
            }
            s.colors[i] = best.map(|(_, color)| color);
        }
        if s.idle() {
            for i in 0..3 {
                for j in 0..3 {
                    s.skill_ready[i][j] = !has(&format!("skill_cd_{}_{}", i + 1, j + 1))?;
                }
                if s.master_open {
                    s.master_ready[i] = !has(&format!("master_cd_{}", i + 1))?;
                }
            }
        }
        // The gauge sample belongs to the command-card screen, not the skill
        // screen: a bright pixel there before opening cards is not proof of NP readiness.
        if s.cards() {
            for i in 0..3 {
                let mut sum = 0u32;
                for y in 679..684 {
                    for x in 144 + 319 * i..156 + 319 * i {
                        let p = frame.get_pixel(x as u32, y);
                        sum += p[0] as u32 + p[1] as u32 + p[2] as u32;
                    }
                }
                s.np_glow[i] = sum as f64 / (12.0 * 5.0 * 3.0) > 55.0;
                if s.cards() {
                    s.np_blocked[i] = has(&format!("np_sealed_{}", i + 1))?
                        || has(&format!("np_chara_sealed_{}", i + 1))?;
                }
            }
        }
        if s.cards() {
            for i in 0..5 {
                s.support_cards[i] = has(&format!("support_card_{}", i + 1))?;
                s.sealed[i] = has(&format!("chara_sealed_{}", i + 1))?;
                for color in ["arts", "quick", "buster"] {
                    s.sealed[i] |= has(&format!("sealed_{color}_{}", i + 1))?;
                }
                let weak = self.find(frame, &format!("weak_{}", i + 1), control)?;
                let resist = self.find(frame, &format!("resist_{}", i + 1), control)?;
                s.resist[i] = match (weak, resist) {
                    (Some(a), Some(b)) => {
                        if a.score <= b.score {
                            1.7
                        } else {
                            0.6
                        }
                    }
                    (Some(_), None) => 1.7,
                    (None, Some(_)) => 0.6,
                    _ => 1.0,
                };
                let mut best = None;
                for n in 1..=10 {
                    if let Some(m) =
                        self.find(frame, &format!("critical_{n}_{}", i + 1), control)?
                        && best.is_none_or(|(score, _)| m.score < score)
                    {
                        best = Some((m.score, n));
                    }
                }
                s.critical[i] = best.map_or(0.0, |(_, n)| n as f64 / 10.0);
            }
        }
        Ok(s)
    }
}

/// Port of the original CN horizontal projection and 66-pixel dilation.
/// Returns 255 for a visible dialog with no recognized target layout.
fn target_regions(frame: &RgbaImage) -> u8 {
    let raw: Vec<_> = (306..973)
        .map(|x| {
            (320..547).any(|y| {
                let p = frame.get_pixel(x, y);
                p[0].max(p[1]).max(p[2]) > 67
            })
        })
        .collect();
    let mut count = 0;
    let mut previous = false;
    for x in 0..raw.len() {
        let on = raw[x.saturating_sub(33)..(x + 33).min(raw.len())]
            .iter()
            .any(|v| *v);
        if on && !previous {
            count += 1;
        }
        previous = on;
    }
    if count == 0 { 255 } else { count }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_projection_preserves_separate_regions_and_rejects_empty_dialog() {
        let mut frame = RgbaImage::new(1280, 720);
        assert_eq!(target_regions(&frame), 255);
        frame.put_pixel(400, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 1);
        frame.put_pixel(650, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 2);
        frame.put_pixel(900, 400, image::Rgba([255, 255, 255, 255]));
        assert_eq!(target_regions(&frame), 3);
    }
}
