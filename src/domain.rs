use serde::{Deserialize, Serialize};

/// Mutually exclusive primary scene inferred from an observation.
///
/// Secondary facts such as the selected party member or an open master panel
/// remain in their scene-specific observation structs.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    QuestList,
    QuestConfirmation,
    SupportList,
    SupportUnavailable,
    SupportRefreshConfirmation,
    Formation,
    ApRecovery,
    AppleConfirmation,
    BattleTurn,
    BattleAnimation,
    SkillTarget,
    SkillConfirmation,
    OrderChange,
    CommandCards,
    Victory,
    Defeated,
    Settlement,
    AddFriend,
    SpecialDrop,
    Repeat,
    UnexpectedDialog,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Started,
    Succeeded,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Apple {
    #[default]
    Gold,
    Silver,
    Bronze,
}

impl Apple {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gold => "gold",
            Self::Silver => "silver",
            Self::Bronze => "bronze",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum FieldSlot {
    One,
    Two,
    Three,
}

impl FieldSlot {
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl TryFrom<usize> for FieldSlot {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            _ => Err("field slot must be 1..3"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum SkillSlot {
    One,
    Two,
    Three,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum PartySlot {
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
}

impl PartySlot {
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum EnemySlot {
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
}

impl EnemySlot {
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl TryFrom<usize> for EnemySlot {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            4 => Ok(Self::Four),
            5 => Ok(Self::Five),
            6 => Ok(Self::Six),
            _ => Err("enemy slot must be 1..6"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Ally { slot: FieldSlot },
    Enemy { slot: EnemySlot },
}

impl SkillSlot {
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl TryFrom<usize> for SkillSlot {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            _ => Err("skill slot must be 1..3"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    ConfirmSkill,
    SetDoubleSpeed,
    SelectCards,
    CastServantSkill {
        servant: FieldSlot,
        skill: SkillSlot,
        target: Target,
        strict: bool,
    },
    CastMasterSkill {
        skill: SkillSlot,
        target: Target,
        strict: bool,
    },
    OrderChange {
        field: PartySlot,
        reserve: PartySlot,
    },
    SelectEnemy {
        slot: EnemySlot,
    },
    Attack {
        opening: bool,
    },
    SelectQuest,
    ConfirmQuest,
    SelectSupport {
        level: u16,
    },
    RefreshSupport {
        attempt: u32,
    },
    StartBattle,
    RestoreAp {
        apple: Apple,
        count: u32,
    },
    DeclineApRecovery,
    CancelApRecovery {
        requested: u32,
        budget: u32,
    },
    DeclineFriendRequest,
    CloseSpecialDrop,
    AdvanceSettlement,
}
