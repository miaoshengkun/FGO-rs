//! Verified battle actions. Rhai chooses the strategy; this layer owns transitions.
use crate::vision::Point;
use anyhow::{Result, bail, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Arts,
    Quick,
    Buster,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BattleState {
    pub turn: bool,
    pub first_turn: bool,
    pub iskandar: bool,
    pub taigong: bool,
    pub reines: bool,
    pub support: bool,
    pub skill_ready: [[bool; 3]; 3],
    pub master_open: bool,
    pub master_ready: [bool; 3],
    pub order_change: bool,
    pub selected: [bool; 6],
    pub target_count: u8,
    pub skill_error: bool,
    pub colors: [Option<Color>; 5],
    pub sealed: [bool; 5],
    pub support_cards: [bool; 5],
    pub resist: [f64; 5],
    pub critical: [f64; 5],
    pub np_glow: [bool; 3],
    pub np_blocked: [bool; 3],
    pub victory: bool,
    pub defeated: bool,
}

impl BattleState {
    pub fn cards(&self) -> bool {
        self.colors.iter().all(Option::is_some)
    }
    pub fn idle(&self) -> bool {
        self.turn
            && !self.cards()
            && !self.order_change
            && self.target_count == 0
            && !self.skill_error
    }
}

/// Time and screen acquisition are injected so transition tests need no device or real sleeps.
pub trait BattlePort {
    fn observe(&mut self) -> Result<BattleState>;
    fn tap(&mut self, point: Point) -> Result<()>;
    fn sleep(&mut self, ms: u64) -> Result<()>;
    fn elapsed_ms(&self) -> u64;
    fn log(&mut self, message: &str) -> Result<()>;
}

pub struct BattleDriver<'a, P: BattlePort + ?Sized> {
    pub port: &'a mut P,
}

impl<P: BattlePort + ?Sized> BattleDriver<'_, P> {
    fn observe(&mut self) -> Result<BattleState> {
        let s = self.port.observe()?;
        ensure!(!s.defeated, "battle defeated");
        ensure!(!s.skill_error, "skill failed or unrecognized battle dialog");
        Ok(s)
    }

    fn wait(
        &mut self,
        description: &str,
        timeout: u64,
        predicate: impl Fn(&BattleState) -> bool,
    ) -> Result<BattleState> {
        let deadline = self.port.elapsed_ms().saturating_add(timeout);
        loop {
            let state = self.observe()?;
            if predicate(&state) {
                return Ok(state);
            }
            ensure!(!state.victory, "battle ended while {description}");
            ensure!(
                self.port.elapsed_ms() < deadline,
                "timed out: {description}"
            );
            self.port.sleep(150)?;
        }
    }

    fn idle(&mut self) -> Result<BattleState> {
        self.wait("waiting for skill screen", 45000, BattleState::idle)
    }

    pub fn verify_opening(&mut self) -> Result<()> {
        let s = self.idle()?;
        ensure!(
            s.first_turn && s.iskandar && s.taigong && s.support,
            "opening requires untouched turn 1: Iskandar, Taigong, friend support"
        );
        ensure!(
            s.skill_ready[0][0] && s.skill_ready[0][1],
            "opening skills already used"
        );
        self.port
            .log("Opening verified; enemy target remains the game's current selection")
    }

    pub fn verify_support(&mut self) -> Result<()> {
        ensure!(
            self.idle()?.support,
            "friend support is no longer in field slot 3"
        );
        Ok(())
    }

    fn master_panel(&mut self, opened: bool) -> Result<BattleState> {
        for _ in 0..3 {
            let s = self.idle()?;
            if s.master_open == opened {
                return Ok(s);
            }
            self.port.tap(Point { x: 1200, y: 317 })?;
            self.port.sleep(500)?;
        }
        let s = self.idle()?;
        ensure!(
            s.master_open == opened,
            "master skill panel failed to change state"
        );
        Ok(s)
    }

    fn target_support(&mut self, count: u8) -> Result<()> {
        ensure!(
            (1..=3).contains(&count),
            "unrecognized skill target layout ({count})"
        );
        // Preserve the CN layout mapping in FGO-py: a merged region uses key 3,
        // separated regions use key 4. Both prefer the third field servant.
        self.port.tap(Point {
            x: if count == 1 { 649 } else { 875 },
            y: 440,
        })?;
        self.port.sleep(400)
    }

    pub fn cast_skill(&mut self, slot: usize, skill: usize, strict: bool) -> Result<bool> {
        ensure!(
            (1..=3).contains(&slot) && (1..=3).contains(&skill),
            "servant and skill must be 1..3"
        );
        let (i, j) = (slot - 1, skill - 1);
        let s = self.master_panel(false)?;
        ensure!(s.support, "friend support is no longer in field slot 3");
        if !s.skill_ready[i][j] {
            ensure!(
                !strict,
                "required servant {slot} skill {skill} is not ready"
            );
            return Ok(false);
        }
        self.port.log(&format!(
            "Servant {slot} skill {skill}; ally target: field slot 3"
        ))?;
        let x = [[73, 163, 257], [388, 483, 574], [704, 801, 891]][i][j];
        self.port.tap(Point { x, y: 573 })?;
        self.port.sleep(800)?;
        let deadline = self.port.elapsed_ms() + 35000;
        let mut target_sent = false;
        loop {
            let s = self.observe()?;
            ensure!(!s.victory, "battle ended during skill cast");
            if s.target_count > 0 && !target_sent {
                self.target_support(s.target_count)?;
                target_sent = true;
            } else if s.idle() && !s.skill_ready[i][j] {
                self.port.sleep(300)?;
                return Ok(true);
            }
            ensure!(
                self.port.elapsed_ms() < deadline,
                "servant {slot} skill {skill} did not enter cooldown"
            );
            self.port.sleep(150)?;
        }
    }

    pub fn order_change(&mut self) -> Result<()> {
        let s = self.master_panel(true)?;
        ensure!(
            s.iskandar && s.support && s.master_ready[2],
            "order change requires Iskandar in slot 1 and a ready master skill 3"
        );
        self.port
            .log("Order change: field slot 1 <-> reserve slot 5")?;
        self.port.tap(Point { x: 1084, y: 317 })?;
        self.port.sleep(500)?;
        let s = self.wait("opening order change", 15000, |s| s.order_change)?;
        ensure!(
            s.selected.iter().all(|v| !v),
            "order change already has selected servants"
        );
        self.port.tap(Point { x: 140, y: 360 })?;
        self.port.sleep(400)?;
        let s = self.wait("selecting order change slot 1", 5000, |s| {
            s.order_change && s.selected[0]
        })?;
        ensure!(
            s.selected.iter().filter(|v| **v).count() == 1,
            "unexpected reserve selection"
        );
        self.port.tap(Point { x: 940, y: 360 })?;
        self.port.sleep(400)?;
        let s = self.wait("selecting order change slot 5", 5000, |s| {
            s.order_change && s.selected[0] && s.selected[4]
        })?;
        ensure!(
            s.selected.iter().filter(|v| **v).count() == 2,
            "unexpected order change selections"
        );
        self.port.tap(Point { x: 640, y: 629 })?;
        self.port.sleep(1500)?;
        self.wait("confirming Reines in field slot 1", 45000, |s| {
            s.idle() && s.reines && s.support
        })?;
        self.port.sleep(500)
    }

    pub fn cast_master_skill(&mut self, skill: usize, strict: bool) -> Result<bool> {
        ensure!(
            (1..=2).contains(&skill),
            "use order_change() for master skill 3"
        );
        let i = skill - 1;
        let s = self.master_panel(true)?;
        ensure!(s.support, "friend support is no longer in field slot 3");
        if !s.master_ready[i] {
            self.master_panel(false)?;
            ensure!(!strict, "required master skill {skill} is not ready");
            return Ok(false);
        }
        self.port.log(&format!("Master skill {skill}"))?;
        self.port.tap(Point {
            x: [907, 995][i],
            y: 317,
        })?;
        self.port.sleep(800)?;
        let deadline = self.port.elapsed_ms() + 35000;
        let mut target_sent = false;
        loop {
            let s = self.observe()?;
            ensure!(!s.victory, "battle ended during master skill");
            if s.target_count > 0 && !target_sent {
                self.target_support(s.target_count)?;
                target_sent = true;
            } else if s.idle() {
                break;
            }
            ensure!(
                self.port.elapsed_ms() < deadline,
                "master skill target/animation timed out"
            );
            self.port.sleep(150)?;
        }
        self.port.sleep(1000)?;
        let s = self.master_panel(true)?;
        ensure!(
            !s.master_ready[i],
            "master skill {skill} did not enter cooldown"
        );
        self.master_panel(false)?;
        Ok(true)
    }

    pub fn attack(&mut self, first: bool) -> Result<()> {
        let before = self.master_panel(false)?;
        ensure!(
            before.support,
            "friend support is no longer in field slot 3"
        );
        self.port.tap(Point { x: 1130, y: 600 })?;
        self.port.sleep(1800)?;
        let first_frame = self.wait("opening command cards", 15000, BattleState::cards)?;
        self.port.sleep(150)?;
        let current = self.wait("confirming command cards", 5000, BattleState::cards)?;
        let ready = std::array::from_fn(|i| {
            !first_frame.np_blocked[i]
                && !current.np_blocked[i]
                && (first_frame.np_glow[i] || current.np_glow[i])
        });
        let cards = if first {
            if !ready.iter().all(|v| *v) {
                self.port.tap(Point { x: 1210, y: 683 })?;
                bail!("opening requires all three noble phantasms, ready={ready:?}");
            }
            [6, 7, 8]
        } else {
            rank_cards(&current, ready)?
        };
        self.port.log(&format!("Select cards: {cards:?}"))?;
        for (i, card) in cards.into_iter().enumerate() {
            let p = if card <= 5 {
                Point {
                    x: [185, 399, 649, 875, 1101][card - 1],
                    y: 440,
                }
            } else {
                Point {
                    x: [431, 651, 845][card - 6],
                    y: 203,
                }
            };
            self.port.tap(p)?;
            self.port.sleep(if i == 2 { 1800 } else { 400 })?;
        }
        self.wait("starting attack", 15000, |s| {
            !s.turn && !s.cards() && !s.order_change && s.target_count == 0
        })?;
        Ok(())
    }

    pub fn wait_next(&mut self) -> Result<bool> {
        let s = self.wait("waiting for next turn or battle result", 180000, |s| {
            s.victory || s.idle()
        })?;
        Ok(!s.victory)
    }
}

/// NP order first, followed by unsealed support face cards, then damage score.
/// Sealed cards are excluded rather than clicked as a fallback.
pub fn rank_cards(state: &BattleState, ready: [bool; 3]) -> Result<[usize; 3]> {
    ensure!(state.cards(), "command card colors are not all recognized");
    let mut cards: Vec<_> = (0..3).filter(|i| ready[*i]).map(|i| i + 6).collect();
    let mut faces: Vec<_> = (0..5).filter(|i| !state.sealed[*i]).collect();
    let score=|i:usize|match state.colors[i].expect("validated colors") {
        Color::Arts=>0.8,Color::Quick=>1.0,Color::Buster=>1.1
    } * state.resist[i] * (1.0+state.critical[i]);
    faces.sort_by(|a, b| {
        state.support_cards[*b]
            .cmp(&state.support_cards[*a])
            .then_with(|| score(*b).total_cmp(&score(*a)))
            .then_with(|| a.cmp(b))
    });
    cards.extend(faces.into_iter().map(|i| i + 1));
    ensure!(cards.len() >= 3, "fewer than three usable cards");
    Ok([cards[0], cards[1], cards[2]])
}
