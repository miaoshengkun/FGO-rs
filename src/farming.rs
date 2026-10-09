use crate::{
    domain::{Command, Scene},
    executor::Executor,
    layout::{Swipe, UI_LAYOUT_V1},
    observation::SceneFacts,
    vision::Point,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Coordinates use the same normalized canvas as recognition and input mapping.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct FixedFarmConfig {
    pub quest: Point,
    /// Left to right, like FGO-py. Zero preserves the game's current formation.
    pub team: u8,
}

impl FixedFarmConfig {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.quest.x < 1280 && self.quest.y < 720,
            "quest point outside 1280x720"
        );
        ensure!(self.team <= 15, "team index must be 0..15");
        Ok(())
    }
}

pub use crate::domain::Apple;

/// Optional farm-screen details requested by the current workflow step.
/// Primary scene and modal-safety detectors always run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequiredFarmFacts(u8);

impl RequiredFarmFacts {
    pub const QUEST: Self = Self(1 << 0);
    pub const SUPPORT: Self = Self(1 << 1);
    pub const FORMATION: Self = Self(1 << 2);
    pub const APPLE: Self = Self(1 << 3);
    pub const TEAM: Self = Self(1 << 4);
    pub const ALL: Self = Self((1 << 4) - 1);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl Apple {
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "gold" => Ok(Self::Gold),
            "silver" => Ok(Self::Silver),
            "bronze" => Ok(Self::Bronze),
            _ => bail!("unsupported apple kind"),
        }
    }
    fn layout_index(self) -> usize {
        match self {
            Self::Gold => 0,
            Self::Silver => 1,
            Self::Bronze => 2,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct FarmState {
    pub event: bool,
    pub menu: bool,
    pub quest: Option<Point>,
    pub quest_confirm: bool,
    pub support: bool,
    pub no_support: bool,
    pub rider_tab: bool,
    /// Recognized filter button for a configured non-Rider profile.
    pub support_filter: Option<Point>,
    pub support_end: bool,
    pub friend: Option<Point>,
    pub refresh_confirm: bool,
    pub formation: bool,
    pub level120: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_index: Option<u8>,
    pub turn: bool,
    pub ap_empty: bool,
    pub apple_confirm: Option<Apple>,
    pub apple_count: Option<u32>,
    pub repeat: bool,
    pub reward: bool,
    pub next: bool,
    pub add_friend: bool,
    pub special_drop: bool,
    pub defeated: bool,
}
impl FarmState {
    fn ready(&self) -> bool {
        self.support || self.no_support || self.formation || self.turn || self.ap_empty
    }
    fn list(&self) -> bool {
        self.support || self.no_support
    }

    pub fn scene(&self) -> Scene {
        self.scene_candidates()
            .first()
            .copied()
            .unwrap_or(Scene::Unknown)
    }

    pub fn scene_candidates(&self) -> Vec<Scene> {
        let mut scenes = Vec::new();
        if self.defeated {
            scenes.push(Scene::Defeated);
        }
        if self.apple_confirm.is_some() {
            scenes.push(Scene::AppleConfirmation);
        } else if self.ap_empty {
            // The AP recovery window remains visible behind apple confirmation.
            scenes.push(Scene::ApRecovery);
        }
        if self.refresh_confirm {
            scenes.push(Scene::SupportRefreshConfirmation);
        }
        if self.quest_confirm {
            scenes.push(Scene::QuestConfirmation);
        }
        let modal = self.apple_confirm.is_some()
            || self.ap_empty
            || self.refresh_confirm
            || self.quest_confirm;
        if !modal {
            if self.formation {
                scenes.push(Scene::Formation);
            }
            if self.support {
                scenes.push(Scene::SupportList);
            }
            if self.no_support {
                scenes.push(Scene::SupportUnavailable);
            }
            if self.turn {
                scenes.push(Scene::BattleTurn);
            }
            if self.repeat {
                scenes.push(Scene::Repeat);
            }
            if self.add_friend {
                scenes.push(Scene::AddFriend);
            }
            if self.special_drop {
                scenes.push(Scene::SpecialDrop);
            }
            if self.reward || self.next {
                scenes.push(Scene::Settlement);
            }
            if self.event && self.menu {
                scenes.push(Scene::QuestList);
            }
        }
        scenes
    }
}

impl SceneFacts for FarmState {
    fn scene_candidates(&self) -> Vec<Scene> {
        FarmState::scene_candidates(self)
    }
}

pub trait FarmPort: Executor {
    fn observe_farm(&mut self) -> Result<FarmState>;
    fn observe_farm_required(&mut self, _facts: RequiredFarmFacts) -> Result<FarmState> {
        self.observe_farm()
    }
}

#[derive(Debug, Serialize)]
pub struct FarmSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<FixedFarmConfig>,
    pub max_battles: u32,
    pub completed: u32,
    pub apples_remaining: u32,
    pub apple: Apple,
    pub active: bool,
    pub refresh_limit: u32,
}

const SUPPORT_SCROLLS_PER_REFRESH: usize = 5;
impl Default for FarmSession {
    fn default() -> Self {
        Self {
            fixed: None,
            max_battles: 1,
            completed: 0,
            apples_remaining: 0,
            apple: Apple::Gold,
            active: false,
            refresh_limit: 10,
        }
    }
}

pub struct FarmDriver<'a, P: FarmPort + ?Sized> {
    pub port: &'a mut P,
    pub session: &'a mut FarmSession,
}
impl<P: FarmPort + ?Sized> FarmDriver<'_, P> {
    fn swipe(&mut self, swipe: Swipe) -> Result<()> {
        self.port.swipe(swipe.from, swipe.to, swipe.duration_ms)
    }

    fn observe(&mut self, facts: RequiredFarmFacts) -> Result<FarmState> {
        let s = self.port.observe_farm_required(facts)?;
        ensure!(!s.defeated, "battle defeated");
        Ok(s)
    }
    fn wait(
        &mut self,
        label: &str,
        ms: u64,
        facts: RequiredFarmFacts,
        predicate: impl Fn(&FarmState) -> bool,
    ) -> Result<FarmState> {
        let deadline = self.port.elapsed_ms() + ms;
        loop {
            let s = self.observe(facts)?;
            if predicate(&s) {
                return Ok(s);
            }
            ensure!(self.port.elapsed_ms() < deadline, "timed out: {label}");
            self.port.sleep(300)?;
        }
    }
    // Match coordinates are not actionable while the list is still coasting.
    // Require three observations spanning at least 600 ms near one anchor.
    fn stable_quest(
        &mut self,
        state: &mut FarmState,
        facts: RequiredFarmFacts,
    ) -> Result<Option<Point>> {
        let mut anchor = state.quest;
        let mut consecutive = usize::from(anchor.is_some());
        for _ in 0..20 {
            self.port.sleep(300)?;
            *state = self.observe(facts)?;
            ensure!(
                state.scene_candidates() == [Scene::QuestList],
                "left event list while checking quest stability"
            );
            let Some(point) = state.quest else {
                return Ok(None);
            };
            if anchor.is_some_and(|previous| {
                point.x.abs_diff(previous.x) <= 2 && point.y.abs_diff(previous.y) <= 2
            }) {
                consecutive += 1;
            } else {
                anchor = Some(point);
                consecutive = 1;
            }
            if consecutive >= 3 {
                return Ok(Some(point));
            }
        }
        anyhow::bail!("quest list did not stabilize; no quest selected")
    }

    pub fn enter(&mut self) -> Result<bool> {
        ensure!(
            !self.session.active,
            "finish the current battle before entering another"
        );
        if self.session.max_battles > 0 && self.session.completed >= self.session.max_battles {
            return Ok(false);
        }
        let fixed = self.session.fixed;
        if let Some(config) = fixed {
            config.validate()?;
        }
        let formation_facts = match fixed {
            Some(config) if config.team > 0 => RequiredFarmFacts::TEAM,
            Some(_) => RequiredFarmFacts::default(),
            None => RequiredFarmFacts::FORMATION,
        };
        let entry_facts = if fixed.is_some() {
            formation_facts
        } else {
            RequiredFarmFacts::QUEST.union(formation_facts)
        };
        let mut s = self.observe(entry_facts)?;
        if s.event && s.menu && !s.quest_confirm {
            if let Some(config) = fixed {
                ensure!(
                    s.scene_candidates() == [Scene::QuestList],
                    "fixed quest selection requires a recognized quest list"
                );
                self.port.sleep(300)?;
                s = self.observe(entry_facts)?;
                ensure!(
                    s.scene_candidates() == [Scene::QuestList],
                    "quest list changed before fixed-position selection"
                );
                self.port.command(Command::SelectQuest)?;
                self.port.tap(config.quest)?;
            } else {
                let mut found = false;
                for swipe in [
                    UI_LAYOUT_V1.farm.quest_scroll_down,
                    UI_LAYOUT_V1.farm.quest_scroll_up,
                ] {
                    for _ in 0..12 {
                        if s.quest.is_some()
                            && let Some(point) = self.stable_quest(&mut s, entry_facts)?
                        {
                            self.port.command(Command::SelectQuest)?;
                            self.port.tap(point)?;
                            found = true;
                            break;
                        }
                        self.swipe(swipe)?;
                        self.port.sleep(400)?;
                        s = self.observe(entry_facts)?;
                        ensure!(
                            s.event && s.menu,
                            "left configured event while finding quest"
                        );
                    }
                    if found {
                        break;
                    }
                }
                ensure!(found, "configured quest not found in event list");
            }
            s = self.wait(
                "quest confirmation / AP prompt",
                15000,
                formation_facts,
                |s| s.quest_confirm || s.ready(),
            )?;
        }
        if s.quest_confirm || s.repeat {
            self.port.command(Command::ConfirmQuest)?;
            self.port.tap(if s.repeat {
                UI_LAYOUT_V1.farm.repeat_confirm
            } else {
                UI_LAYOUT_V1.farm.dialog_confirm
            })?;
            self.port.sleep(700)?;
            s = self.wait(
                "quest loading / AP prompt",
                45000,
                formation_facts,
                FarmState::ready,
            )?;
        }
        ensure!(
            s.ready(),
            "start from configured quest/support list, formation, repeat, or untouched first turn"
        );
        while s.ap_empty {
            if !self.restore_ap()? {
                return Ok(false);
            }
            s = self.wait("AP recovery outcome", 45000, formation_facts, |s| {
                (s.ready() || s.quest_confirm) && s.apple_confirm.is_none()
            })?;
            // The game returns to the quest confirmation dialog after an AP
            // recovery made from that dialog. Confirm the quest again before
            // continuing into support/formation.
            if s.quest_confirm {
                self.port.command(Command::ConfirmQuest)?;
                self.port.tap(UI_LAYOUT_V1.farm.dialog_confirm)?;
                self.port.sleep(700)?;
                s = self.wait(
                    "quest loading after AP recovery",
                    45000,
                    formation_facts,
                    FarmState::ready,
                )?;
            }
        }
        if s.list() {
            self.select_support()?;
            s = self.wait("formation", 90000, formation_facts, |s| {
                s.formation || s.turn
            })?;
        }
        if s.formation {
            if let Some(config) = fixed {
                if config.team > 0 {
                    ensure!(
                        s.team_index.is_some(),
                        "team selector is unavailable or unrecognized; use team 0 for a locked formation"
                    );
                    if s.team_index != Some(config.team) {
                        self.port
                            .command(Command::SelectTeam { index: config.team })?;
                        self.port
                            .tap(UI_LAYOUT_V1.farm.formation_teams[usize::from(config.team - 1)])?;
                        self.port.sleep(500)?;
                    }
                    // Confirm the requested tab on fresh frames before starting.
                    for _ in 0..2 {
                        self.wait("selected team", 10000, RequiredFarmFacts::TEAM, |s| {
                            s.formation && s.team_index == Some(config.team)
                        })?;
                        self.port.sleep(200)?;
                    }
                }
            } else {
                ensure!(
                    s.level120,
                    "formation slot 3 is not recognized as level 120"
                );
            }
            self.port.command(Command::StartBattle)?;
            self.port.tap(UI_LAYOUT_V1.farm.formation_start)?;
            self.port.sleep(1500)?;
        }
        self.wait("battle start", 120000, RequiredFarmFacts::default(), |s| {
            s.turn
        })?;
        self.session.active = true;
        self.port.log(&format!(
            "Starting battle {}; apples remaining: {}",
            self.session.completed + 1,
            self.session.apples_remaining
        ))?;
        Ok(true)
    }

    pub fn select_support(&mut self) -> Result<()> {
        let fixed = self.session.fixed.is_some();
        let mut refreshes = 0;
        let mut last_refresh = None;
        loop {
            let mut s = self.wait(
                "support list",
                45000,
                if fixed {
                    RequiredFarmFacts::default()
                } else {
                    RequiredFarmFacts::SUPPORT
                },
                FarmState::list,
            )?;
            if s.support {
                if fixed {
                    self.port.command(Command::SelectFirstSupport)?;
                    self.port.tap(UI_LAYOUT_V1.farm.first_support)?;
                    self.port.sleep(800)?;
                    self.wait(
                        "first support / formation",
                        90000,
                        RequiredFarmFacts::default(),
                        |s| s.formation || s.turn,
                    )?;
                    self.port.log("Selected first support")?;
                    return Ok(());
                }
                self.port.tap(
                    s.support_filter
                        .unwrap_or(UI_LAYOUT_V1.farm.support_rider_filter),
                )?;
                self.port.sleep(600)?;
                s = self.wait("support filter", 15000, RequiredFarmFacts::SUPPORT, |s| {
                    s.support && s.rider_tab
                })?;
                // The support list is virtualized and repeatedly scrolling to
                // the bottom can skip freshly loaded candidates. Search five
                // viewport advances, then refresh and start from the top.
                for page in 0..=SUPPORT_SCROLLS_PER_REFRESH {
                    ensure!(
                        s.support && s.rider_tab,
                        "support list/filter changed unexpectedly"
                    );
                    if let Some(friend) = s.friend {
                        self.port.command(Command::SelectSupport { level: 120 })?;
                        self.port.tap(friend)?;
                        self.port.sleep(800)?;
                        let next = self.wait(
                            "selected friend / formation",
                            90000,
                            RequiredFarmFacts::FORMATION,
                            |s| s.formation || s.turn,
                        )?;
                        ensure!(
                            !next.formation || next.level120,
                            "selected support is not level 120 in formation"
                        );
                        self.port.log(&format!(
                            "Selected level 120 friend after {refreshes} refreshes"
                        ))?;
                        return Ok(());
                    }
                    if s.support_end || page == SUPPORT_SCROLLS_PER_REFRESH {
                        break;
                    }
                    self.swipe(UI_LAYOUT_V1.farm.support_scroll)?;
                    self.port.sleep(500)?;
                    s = self.wait(
                        "scrolled support list",
                        15000,
                        RequiredFarmFacts::SUPPORT,
                        FarmState::list,
                    )?;
                    if s.no_support {
                        break;
                    }
                }
            }
            ensure!(
                refreshes < self.session.refresh_limit,
                "no {} after {refreshes} refreshes",
                if fixed {
                    "available support"
                } else {
                    "level 120 friend"
                }
            );
            if let Some(last) = last_refresh {
                let elapsed = self.port.elapsed_ms() - last;
                if elapsed < 10000 {
                    self.port.sleep(10000 - elapsed)?;
                }
            }
            self.wait(
                "support list before refresh",
                15000,
                RequiredFarmFacts::default(),
                FarmState::list,
            )?;
            self.port.command(Command::RefreshSupport {
                attempt: refreshes + 1,
            })?;
            self.port.tap(UI_LAYOUT_V1.farm.support_refresh)?;
            self.port.sleep(500)?;
            self.wait(
                "support refresh confirmation",
                15000,
                RequiredFarmFacts::default(),
                |s| s.refresh_confirm,
            )?;
            self.port.tap(UI_LAYOUT_V1.farm.dialog_confirm)?;
            refreshes += 1;
            last_refresh = Some(self.port.elapsed_ms());
            self.port.log(&format!(
                "Support refresh {refreshes}/{}",
                self.session.refresh_limit
            ))?;
            self.port.sleep(1000)?;
        }
    }

    fn restore_ap(&mut self) -> Result<bool> {
        if self.session.apples_remaining == 0 {
            self.port.command(Command::DeclineApRecovery)?;
            self.port.tap(UI_LAYOUT_V1.farm.ap_close)?;
            self.wait(
                "closing AP recovery",
                10000,
                RequiredFarmFacts::default(),
                |s| !s.ap_empty,
            )?;
            self.port
                .log("AP insufficient and apple budget exhausted; stopped")?;
            return Ok(false);
        }
        // Normalize the list position before choosing a row. Confirm kind and
        // quantity on the next screen before any resource is consumed.
        let swipe = if self.session.apple == Apple::Bronze {
            UI_LAYOUT_V1.farm.apple_scroll_bottom
        } else {
            UI_LAYOUT_V1.farm.apple_scroll_top
        };
        self.swipe(swipe)?;
        self.port.sleep(700)?;
        let point = UI_LAYOUT_V1.farm.apple_choices[self.session.apple.layout_index()];
        self.port.tap(point)?;
        self.port.sleep(600)?;
        let expected = self.session.apple;
        let mut confirmation = self.wait(
            "apple-kind confirmation",
            15000,
            RequiredFarmFacts::APPLE,
            |s| s.apple_confirm == Some(expected),
        )?;
        if expected != Apple::Gold {
            // The game enforces a minimum batch sufficient for this quest.
            self.port.tap(UI_LAYOUT_V1.farm.apple_minimum)?;
            self.port.sleep(700)?;
            confirmation = self.wait(
                "minimum apple quantity",
                15000,
                RequiredFarmFacts::APPLE,
                |s| s.apple_confirm == Some(expected) && s.apple_count.is_some(),
            )?;
        }
        let count = confirmation
            .apple_count
            .context("unrecognized apple quantity")?;
        ensure!((1..=4).contains(&count), "unsupported apple batch size");
        if count > self.session.apples_remaining {
            self.port.command(Command::CancelApRecovery {
                requested: count,
                budget: self.session.apples_remaining,
            })?;
            self.port.tap(UI_LAYOUT_V1.farm.dialog_cancel)?;
            self.wait(
                "cancel apple confirmation",
                10000,
                RequiredFarmFacts::default(),
                |s| s.ap_empty && s.apple_confirm.is_none(),
            )?;
            self.port.tap(UI_LAYOUT_V1.farm.ap_close)?;
            self.wait(
                "closing AP recovery",
                10000,
                RequiredFarmFacts::default(),
                |s| !s.ap_empty,
            )?;
            self.port
                .log("Minimum apple batch exceeds remaining budget; stopped")?;
            return Ok(false);
        }
        // Reserve before sending confirmation; ambiguous failures never retry spending.
        self.session.apples_remaining -= count;
        self.port.log(&format!(
            "Confirm {count} {} apple(s); remaining budget {}",
            expected.name(),
            self.session.apples_remaining
        ))?;
        self.port.command(Command::RestoreAp {
            apple: expected,
            count,
        })?;
        self.port.tap(UI_LAYOUT_V1.farm.apple_confirm)?;
        self.port.sleep(1200)?;
        self.wait(
            "AP recovery outcome",
            45000,
            RequiredFarmFacts::default(),
            |s| (s.ready() || s.quest_confirm) && s.apple_confirm.is_none(),
        )?;
        Ok(true)
    }

    pub fn finish(&mut self) -> Result<()> {
        if !self.session.active {
            let s = self.observe(RequiredFarmFacts::default())?;
            ensure!(
                s.reward || s.next || s.add_friend || s.special_drop,
                "no active battle or recognized result screen to settle"
            );
            self.session.active = true;
        }
        let deadline = self.port.elapsed_ms() + 120000;
        loop {
            let s = self.observe(RequiredFarmFacts::default())?;
            if s.repeat || (s.event && s.menu) {
                self.session.completed += 1;
                self.session.active = false;
                self.port.log(&format!(
                    "Battle {} completed; {} apples remaining",
                    self.session.completed, self.session.apples_remaining
                ))?;
                return Ok(());
            }
            if s.add_friend {
                self.port.command(Command::DeclineFriendRequest)?;
                self.port.tap(UI_LAYOUT_V1.farm.decline_friend)?;
                self.port.sleep(500)?;
            } else if s.special_drop {
                self.port.command(Command::CloseSpecialDrop)?;
                self.port.tap(UI_LAYOUT_V1.farm.close_special_drop)?;
                self.port.sleep(500)?;
            } else if s.reward || s.next {
                self.port.command(Command::AdvanceSettlement)?;
                self.port.tap(UI_LAYOUT_V1.farm.settlement_next)?;
                self.port.sleep(700)?;
            } else {
                self.port.sleep(300)?;
            }
            ensure!(
                self.port.elapsed_ms() < deadline,
                "settlement timed out on an unrecognized screen"
            );
        }
    }
}
