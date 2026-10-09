use crate::{
    control::Control,
    farming::{Apple, FarmState, RequiredFarmFacts},
    vision::{Point, Vision},
};
use anyhow::Result;
use image::RgbaImage;

impl Vision {
    pub fn farm_state(&self, frame: &RgbaImage, control: &Control) -> Result<FarmState> {
        self.farm_state_with_facts(frame, control, RequiredFarmFacts::ALL)
    }

    pub fn farm_state_with_facts(
        &self,
        frame: &RgbaImage,
        control: &Control,
        facts: RequiredFarmFacts,
    ) -> Result<FarmState> {
        Self::require_canvas(frame)?;
        let mut matcher = self.frame_matcher(frame, control);
        let refresh_confirm = matcher.has("refresh_confirm")?;
        let quest_confirm = matcher.has("quest_confirm")?;
        let ap_empty = matcher.has("ap_empty")?;
        let mut apple_confirm = None;
        for apple in [Apple::Gold, Apple::Silver, Apple::Bronze] {
            if matcher.has_optional(&format!("apple_confirm_{}", apple.name()))? {
                apple_confirm = Some(apple);
                break;
            }
        }
        let mut apple_count =
            if facts.contains(RequiredFarmFacts::APPLE) && apple_confirm == Some(Apple::Gold) {
                Some(1)
            } else {
                None
            };
        if facts.contains(RequiredFarmFacts::APPLE)
            && apple_confirm.is_some()
            && apple_count.is_none()
        {
            let mut counts = Vec::new();
            for count in 1..=4 {
                if matcher.has_optional(&format!("apple_count_{count}"))? {
                    counts.push(count);
                }
            }
            if counts.len() == 1 {
                apple_count = Some(counts[0]);
            }
        }
        let blocked = refresh_confirm || quest_confirm || ap_empty || apple_confirm.is_some();
        let tab = self
            .farm_templates
            .as_ref()
            .map_or("rider_tab", |farm| farm.support_tab.as_str());
        let support = !blocked
            && (matcher.has("choose_friend")?
                || matcher.has("choose_friend_extra")?
                || matcher.has(tab)?);
        let no_support = !blocked && matcher.has("no_friend")?;
        let formation = !blocked && matcher.has("formation")?;
        let event = matcher.has(
            self.farm_templates
                .as_ref()
                .map_or("event", |farm| farm.event.as_str()),
        )?;
        let menu = matcher.has("main_menu")?;
        Ok(FarmState {
            event,
            menu,
            quest: if facts.contains(RequiredFarmFacts::QUEST) && event && menu && !blocked {
                matcher
                    .find(
                        self.farm_templates
                            .as_ref()
                            .map_or("quest", |farm| farm.quest.as_str()),
                    )?
                    .map(|m| Point { x: m.x, y: m.y })
            } else {
                None
            },
            quest_confirm,
            support,
            no_support,
            rider_tab: support && facts.contains(RequiredFarmFacts::SUPPORT) && matcher.has(tab)?,
            support_filter: if support && facts.contains(RequiredFarmFacts::SUPPORT) {
                if let Some(farm) = &self.farm_templates {
                    let found = matcher.find(&farm.support_filter)?.ok_or_else(|| {
                        anyhow::anyhow!("configured support filter is not visible")
                    })?;
                    Some(Point {
                        x: found.x,
                        y: found.y,
                    })
                } else {
                    None
                }
            } else {
                None
            },
            support_end: if support && facts.contains(RequiredFarmFacts::SUPPORT) {
                matcher.find("support_scroll")?.is_none_or(|m| m.y >= 685)
            } else {
                false
            },
            friend: if support && facts.contains(RequiredFarmFacts::SUPPORT) {
                self.support(frame, control)?
                    .map(|m| Point { x: m.x, y: m.y })
            } else {
                None
            },
            refresh_confirm,
            formation,
            level120: formation
                && facts.contains(RequiredFarmFacts::FORMATION)
                && matcher.has(
                    self.farm_templates
                        .as_ref()
                        .map_or("formation_support120", |farm| {
                            farm.formation_support.as_str()
                        }),
                )?,
            turn: !blocked && matcher.has("turn_begin")?,
            ap_empty,
            apple_confirm,
            apple_count,
            repeat: !blocked && matcher.has("repeat")?,
            reward: matcher.has("victory")?,
            next: matcher.has_optional("settlement_next")?
                || matcher.has_optional("bond_result")?
                || matcher.has_optional("master_result")?
                || (matcher.has_optional("bond_level_up")?
                    && matcher.has_optional("result_tap")?),
            add_friend: matcher.has("add_friend")?,
            special_drop: matcher.has("special_drop")?
                || (matcher.has_optional("ce_reward_label")?
                    && matcher.has_optional("ce_reward_tap")?),
            defeated: matcher.has("defeated")?,
        })
    }
}
