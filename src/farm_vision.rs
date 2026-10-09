use crate::{
    control::Control,
    farming::{Apple, FarmState, RequiredFarmFacts},
    layout::UI_LAYOUT_V1,
    vision::{Point, Rect, Vision},
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
        let event = !self.fixed_farm
            && matcher.has(
                self.farm_templates
                    .as_ref()
                    .map_or("event", |farm| farm.event.as_str()),
            )?;
        let menu = matcher.has("main_menu")?;
        let mut team_indices = Vec::new();
        if self.fixed_farm && formation && facts.contains(RequiredFarmFacts::TEAM) {
            for (index, point) in UI_LAYOUT_V1.farm.formation_teams.iter().enumerate() {
                if self.has_template("formation_team")
                    && matcher
                        .find_in(
                            "formation_team",
                            Some(Rect {
                                x: point.x - 12,
                                y: 34,
                                width: 25,
                                height: 28,
                            }),
                        )?
                        .is_some()
                {
                    team_indices.push(index as u8 + 1);
                }
            }
        }
        let mut state = FarmState {
            event,
            menu,
            quest: if !self.fixed_farm
                && facts.contains(RequiredFarmFacts::QUEST)
                && event
                && menu
                && !blocked
            {
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
            rider_tab: !self.fixed_farm
                && support
                && facts.contains(RequiredFarmFacts::SUPPORT)
                && matcher.has(tab)?,
            support_filter: if !self.fixed_farm
                && support
                && facts.contains(RequiredFarmFacts::SUPPORT)
            {
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
            support_end: if !self.fixed_farm
                && support
                && facts.contains(RequiredFarmFacts::SUPPORT)
            {
                matcher.find("support_scroll")?.is_none_or(|m| m.y >= 685)
            } else {
                false
            },
            friend: if !self.fixed_farm && support && facts.contains(RequiredFarmFacts::SUPPORT) {
                self.support(frame, control)?
                    .map(|m| Point { x: m.x, y: m.y })
            } else {
                None
            },
            refresh_confirm,
            formation,
            level120: !self.fixed_farm
                && formation
                && facts.contains(RequiredFarmFacts::FORMATION)
                && matcher.has(
                    self.farm_templates
                        .as_ref()
                        .map_or("formation_support120", |farm| {
                            farm.formation_support.as_str()
                        }),
                )?,
            team_index: (team_indices.len() == 1).then(|| team_indices[0]),
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
        };
        if self.fixed_farm {
            // FGO-py uses the common MENU anchor for a fixed-position quest.
            // Keep all recognized dialogs/results and other scenes exclusive.
            state.event = menu && !blocked && state.scene_candidates().is_empty();
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scene;
    use image::imageops;
    use std::{path::PathBuf, time::Duration};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn vision() -> Vision {
        let mut vision = Vision::load(&root().join("assets/templates.json")).unwrap();
        vision.fixed_farm = true;
        vision
    }

    fn frame(name: &str) -> RgbaImage {
        let raw = image::open(root().join("tests/fixtures").join(name))
            .unwrap()
            .to_rgba8();
        crate::vision::normalize(&raw, None).unwrap().0
    }

    #[test]
    fn fixed_mode_recognizes_common_scenes_without_quest_or_level_templates() {
        let vision = vision();
        let control = Control::new(Duration::from_secs(30));
        for (name, expected) in [
            ("live-saber-quests-v.png", Scene::QuestList),
            ("live-saber-last-vii.png", Scene::QuestList),
            ("step2.png", Scene::SupportList),
            ("live-support-scrolled.png", Scene::SupportList),
            ("live-formation.png", Scene::Formation),
            ("live-quest-confirm.png", Scene::QuestConfirmation),
            ("live-repeat.png", Scene::Repeat),
            ("live-bond-result.png", Scene::Settlement),
        ] {
            let state = vision
                .farm_state_with_facts(
                    &frame(name),
                    &control,
                    RequiredFarmFacts::ALL.union(RequiredFarmFacts::TEAM),
                )
                .unwrap();
            assert_eq!(state.scene_candidates(), vec![expected], "{name}");
            assert!(
                state.quest.is_none()
                    && state.friend.is_none()
                    && !state.level120
                    && state.support_filter.is_none(),
                "{name}"
            );
        }
        let unknown = vision
            .farm_state(&RgbaImage::new(1280, 720), &control)
            .unwrap();
        assert_eq!(unknown.scene(), Scene::Unknown);
    }

    fn verify_team_tabs(vision: &Vision) {
        let control = Control::new(Duration::from_secs(30));
        let original = frame("live-formation.png");
        assert_eq!(
            vision
                .farm_state_with_facts(&original, &control, RequiredFarmFacts::TEAM)
                .unwrap()
                .team_index,
            None,
            "locked formation has no selectable team"
        );
        let marker = image::open(root().join("assets/common/teamindex.png"))
            .unwrap()
            .to_rgba8();
        for index in [1, 8, 15] {
            let point = UI_LAYOUT_V1.farm.formation_teams[index - 1];
            let mut selected = original.clone();
            imageops::replace(&mut selected, &marker, i64::from(point.x - 10), 38);
            let state = vision
                .farm_state_with_facts(&selected, &control, RequiredFarmFacts::TEAM)
                .unwrap();
            assert!(state.formation);
            assert_eq!(state.team_index, Some(index as u8));
            imageops::replace(&mut selected, &marker, 480, 38);
            assert_eq!(
                vision
                    .farm_state_with_facts(&selected, &control, RequiredFarmFacts::TEAM)
                    .unwrap()
                    .team_index,
                None,
                "conflicting tabs must stay unknown"
            );
        }
    }

    #[test]
    fn fixed_teams_require_one_unambiguous_selected_tab() {
        verify_team_tabs(&vision());
    }

    #[cfg(feature = "maa")]
    #[test]
    #[ignore = "requires FGO_MAA_LIBRARY; checks ordinal formation tabs with native recognition"]
    fn fixed_team_tabs_with_native_recognition() {
        let library = PathBuf::from(std::env::var_os("FGO_MAA_LIBRARY").unwrap());
        let mut vision = vision();
        vision
            .set_maa_backend(
                crate::maa::MaaRecognizer::new(&library, &[root().join("assets/maa")], None)
                    .unwrap(),
            )
            .unwrap();
        verify_team_tabs(&vision);
    }
}
