use crate::{
    control::Control,
    farming::{Apple, FarmState},
    vision::{Point, Vision},
};
use anyhow::Result;
use image::RgbaImage;

impl Vision {
    pub fn farm_state(&self, frame: &RgbaImage, control: &Control) -> Result<FarmState> {
        Self::require_canvas(frame)?;
        let has = |name: &str| -> Result<bool> { Ok(self.find(frame, name, control)?.is_some()) };
        let optional = |name: &str| -> Result<bool> { Ok(self.has_template(name) && has(name)?) };
        let refresh_confirm = has("refresh_confirm")?;
        let quest_confirm = has("quest_confirm")?;
        let ap_empty = has("ap_empty")?;
        let mut apple_confirm = None;
        for apple in [Apple::Gold, Apple::Silver, Apple::Bronze] {
            if optional(&format!("apple_confirm_{}", apple.name()))? {
                apple_confirm = Some(apple);
                break;
            }
        }
        let mut apple_count = if apple_confirm == Some(Apple::Gold) {
            Some(1)
        } else {
            None
        };
        if apple_confirm.is_some() && apple_count.is_none() {
            let mut counts = Vec::new();
            for count in 1..=4 {
                if optional(&format!("apple_count_{count}"))? {
                    counts.push(count);
                }
            }
            if counts.len() == 1 {
                apple_count = Some(counts[0]);
            }
        }
        let blocked = refresh_confirm || quest_confirm || ap_empty || apple_confirm.is_some();
        let support = !blocked && (has("choose_friend")? || has("choose_friend_extra")?);
        let no_support = !blocked && has("no_friend")?;
        let formation = !blocked && has("formation")?;
        let event = has("event")?;
        let menu = has("main_menu")?;
        Ok(FarmState {
            event,
            menu,
            quest: if event && menu && !blocked {
                self.find(frame, "quest", control)?
                    .map(|m| Point { x: m.x, y: m.y })
            } else {
                None
            },
            quest_confirm,
            support,
            no_support,
            rider_tab: support && has("rider_tab")?,
            support_end: if support {
                self.find(frame, "support_scroll", control)?
                    .is_none_or(|m| m.y >= 685)
            } else {
                false
            },
            friend: if support {
                self.support(frame, control)?
                    .map(|m| Point { x: m.x, y: m.y })
            } else {
                None
            },
            refresh_confirm,
            formation,
            level120: formation && has("formation_support120")?,
            turn: !blocked && has("turn_begin")?,
            ap_empty,
            apple_confirm,
            apple_count,
            repeat: !blocked && has("repeat")?,
            reward: has("victory")?,
            next: optional("settlement_next")?
                || optional("bond_result")?
                || optional("master_result")?,
            add_friend: has("add_friend")?,
            special_drop: has("special_drop")?,
            defeated: has("defeated")?,
        })
    }
}
