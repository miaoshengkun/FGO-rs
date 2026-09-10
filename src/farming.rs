use crate::{battle::BattlePort, vision::Point};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Apple {
    #[default]
    Gold,
    Silver,
    Bronze,
}
impl Apple {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gold => "gold",
            Self::Silver => "silver",
            Self::Bronze => "bronze",
        }
    }
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "gold" => Ok(Self::Gold),
            "silver" => Ok(Self::Silver),
            "bronze" => Ok(Self::Bronze),
            _ => bail!("unsupported apple kind"),
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
    pub support_end: bool,
    pub friend: Option<Point>,
    pub refresh_confirm: bool,
    pub formation: bool,
    pub level120: bool,
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
}

pub trait FarmPort: BattlePort {
    fn observe_farm(&mut self) -> Result<FarmState>;
    fn swipe(&mut self, from: Point, to: Point, ms: u64) -> Result<()>;
}

#[derive(Debug, Serialize)]
pub struct FarmSession {
    pub max_battles: u32,
    pub completed: u32,
    pub apples_remaining: u32,
    pub apple: Apple,
    pub active: bool,
    pub refresh_limit: u32,
}
impl Default for FarmSession {
    fn default() -> Self {
        Self {
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
    fn observe(&mut self) -> Result<FarmState> {
        let s = self.port.observe_farm()?;
        ensure!(!s.defeated, "battle defeated");
        Ok(s)
    }
    fn wait(
        &mut self,
        label: &str,
        ms: u64,
        predicate: impl Fn(&FarmState) -> bool,
    ) -> Result<FarmState> {
        let deadline = self.port.elapsed_ms() + ms;
        loop {
            let s = self.observe()?;
            if predicate(&s) {
                return Ok(s);
            }
            ensure!(self.port.elapsed_ms() < deadline, "timed out: {label}");
            self.port.sleep(300)?;
        }
    }
    pub fn enter(&mut self) -> Result<bool> {
        ensure!(
            !self.session.active,
            "finish the current battle before entering another"
        );
        if self.session.max_battles > 0 && self.session.completed >= self.session.max_battles {
            return Ok(false);
        }
        let mut s = self.observe()?;
        if s.event && s.menu && !s.quest_confirm {
            let mut found = false;
            for (from, to) in [
                (Point { x: 1000, y: 550 }, Point { x: 1000, y: 250 }),
                (Point { x: 1000, y: 250 }, Point { x: 1000, y: 550 }),
            ] {
                for _ in 0..12 {
                    if let Some(point) = s.quest {
                        self.port.tap(point)?;
                        found = true;
                        break;
                    }
                    self.port.swipe(from, to, 500)?;
                    self.port.sleep(400)?;
                    s = self.observe()?;
                    ensure!(s.event && s.menu, "left rider event while finding quest");
                }
                if found {
                    break;
                }
            }
            ensure!(found, "Rider Grand VII not found in event list");
            s = self.wait("quest confirmation / AP prompt", 15000, |s| {
                s.quest_confirm || s.ready()
            })?;
        }
        if s.quest_confirm || s.repeat {
            self.port.tap(Point { x: 801, y: 573 })?;
            self.port.sleep(700)?;
            s = self.wait("quest loading / AP prompt", 45000, FarmState::ready)?;
        }
        ensure!(
            s.ready(),
            "start from Rider Grand quest/support list, formation, repeat, or untouched first turn"
        );
        while s.ap_empty {
            if !self.restore_ap()? {
                return Ok(false);
            }
            s = self.wait("AP recovery outcome", 45000, |s| {
                s.ready() && s.apple_confirm.is_none()
            })?;
        }
        if s.list() {
            self.select_support()?;
            s = self.wait("formation", 90000, |s| s.formation || s.turn)?;
        }
        if s.formation {
            ensure!(
                s.level120,
                "formation slot 3 is not recognized as level 120"
            );
            self.port.tap(Point { x: 1231, y: 687 })?;
            self.port.sleep(1500)?;
        }
        self.wait("battle start", 120000, |s| s.turn)?;
        self.session.active = true;
        self.port.log(&format!(
            "Starting battle {}; apples remaining: {}",
            self.session.completed + 1,
            self.session.apples_remaining
        ))?;
        Ok(true)
    }

    pub fn select_support(&mut self) -> Result<()> {
        let mut refreshes = 0;
        let mut last_refresh = None;
        loop {
            let mut s = self.wait("support list", 45000, FarmState::list)?;
            if s.support {
                self.port.tap(Point { x: 363, y: 130 })?;
                self.port.sleep(600)?;
                s = self.wait("rider filter", 15000, |s| s.support && s.rider_tab)?;
                for page in 0..25 {
                    ensure!(
                        s.support && s.rider_tab,
                        "support list/filter changed unexpectedly"
                    );
                    if let Some(friend) = s.friend {
                        self.port.tap(friend)?;
                        self.port.sleep(800)?;
                        let next = self.wait("selected friend / formation", 90000, |s| {
                            s.formation || s.turn
                        })?;
                        ensure!(
                            !next.formation || next.level120,
                            "selected support is not level 120 in formation"
                        );
                        self.port.log(&format!(
                            "Selected level 120 rider friend after {refreshes} refreshes"
                        ))?;
                        return Ok(());
                    }
                    if s.support_end || page == 24 {
                        break;
                    }
                    self.port
                        .swipe(Point { x: 400, y: 590 }, Point { x: 400, y: 290 }, 500)?;
                    self.port.sleep(500)?;
                    s = self.wait("scrolled support list", 15000, FarmState::list)?;
                    if s.no_support {
                        break;
                    }
                }
            }
            ensure!(
                refreshes < self.session.refresh_limit,
                "no level 120 friend after {refreshes} refreshes"
            );
            if let Some(last) = last_refresh {
                let elapsed = self.port.elapsed_ms() - last;
                if elapsed < 10000 {
                    self.port.sleep(10000 - elapsed)?;
                }
            }
            self.wait("support list before refresh", 15000, FarmState::list)?;
            self.port.tap(Point { x: 927, y: 131 })?;
            self.port.sleep(500)?;
            self.wait("support refresh confirmation", 15000, |s| s.refresh_confirm)?;
            self.port.tap(Point { x: 801, y: 573 })?;
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
            self.port.tap(Point { x: 640, y: 629 })?;
            self.wait("closing AP recovery", 10000, |s| !s.ap_empty)?;
            self.port
                .log("AP insufficient and apple budget exhausted; stopped")?;
            return Ok(false);
        }
        // Normalize the list position before choosing a row. Confirm kind and
        // quantity on the next screen before any resource is consumed.
        let (from, to) = if self.session.apple == Apple::Bronze {
            (Point { x: 850, y: 550 }, Point { x: 850, y: 200 })
        } else {
            (Point { x: 850, y: 200 }, Point { x: 850, y: 550 })
        };
        self.port.swipe(from, to, 600)?;
        self.port.sleep(700)?;
        let point = match self.session.apple {
            Apple::Gold => Point { x: 907, y: 317 },
            Apple::Silver => Point { x: 875, y: 440 },
            Apple::Bronze => Point { x: 875, y: 500 },
        };
        self.port.tap(point)?;
        self.port.sleep(600)?;
        let expected = self.session.apple;
        let mut confirmation = self.wait("apple-kind confirmation", 15000, |s| {
            s.apple_confirm == Some(expected)
        })?;
        if expected != Apple::Gold {
            // The game enforces a minimum batch sufficient for this quest.
            self.port.tap(Point { x: 422, y: 300 })?;
            self.port.sleep(700)?;
            confirmation = self.wait("minimum apple quantity", 15000, |s| {
                s.apple_confirm == Some(expected) && s.apple_count.is_some()
            })?;
        }
        let count = confirmation
            .apple_count
            .context("unrecognized apple quantity")?;
        ensure!((1..=4).contains(&count), "unsupported apple batch size");
        if count > self.session.apples_remaining {
            self.port.tap(Point { x: 440, y: 573 })?;
            self.wait("cancel apple confirmation", 10000, |s| {
                s.ap_empty && s.apple_confirm.is_none()
            })?;
            self.port.tap(Point { x: 640, y: 629 })?;
            self.wait("closing AP recovery", 10000, |s| !s.ap_empty)?;
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
        self.port.tap(Point { x: 891, y: 573 })?;
        self.port.sleep(1200)?;
        self.wait("AP recovery outcome", 45000, |s| {
            s.ready() && s.apple_confirm.is_none()
        })?;
        Ok(true)
    }

    pub fn finish(&mut self) -> Result<()> {
        if !self.session.active {
            let s = self.observe()?;
            ensure!(
                s.reward || s.next || s.add_friend || s.special_drop,
                "no active battle or recognized result screen to settle"
            );
            self.session.active = true;
        }
        let deadline = self.port.elapsed_ms() + 120000;
        loop {
            let s = self.observe()?;
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
                self.port.tap(Point { x: 173, y: 621 })?;
                self.port.sleep(500)?;
            } else if s.special_drop {
                self.port.tap(Point { x: 40, y: 40 })?;
                self.port.sleep(500)?;
            } else if s.reward || s.next {
                self.port.tap(Point { x: 1231, y: 687 })?;
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
