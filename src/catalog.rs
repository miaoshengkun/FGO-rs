use crate::{domain::PartySlot, profile::PartyProfile};
use anyhow::{Context, Result, bail, ensure};
use image::{ImageFormat, RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    fs::{self, File},
    io::{Cursor, Write},
    path::{Component, Path, PathBuf},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const CATALOG_SCHEMA: u32 = 1;
pub const SERVANT_DETAIL_SCHEMA: u32 = 3;
pub const DEFAULT_ATLAS_API: &str = "https://api.atlasacademy.io";
const INFO_LIMIT: u64 = 1024 * 1024;
const INDEX_LIMIT: u64 = 4 * 1024 * 1024;
const NICE_SERVANT_LIMIT: u64 = 4 * 1024 * 1024;
const COMMAND_CARD_ASSET_LIMIT: u64 = 2 * 1024 * 1024;
const COMMAND_CARD_SOURCE_SIZE: u32 = 256;
const COMMAND_TEMPLATE_X: u32 = 79;
const COMMAND_TEMPLATE_Y: u32 = 61;
const COMMAND_TEMPLATE_WIDTH: u32 = 98;
const COMMAND_TEMPLATE_HEIGHT: u32 = 47;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Region {
    Jp,
    Na,
    #[default]
    Cn,
    Kr,
    Tw,
}

impl Region {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Jp => "JP",
            Self::Na => "NA",
            Self::Cn => "CN",
            Self::Kr => "KR",
            Self::Tw => "TW",
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl FromStr for Region {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "JP" => Ok(Self::Jp),
            "NA" => Ok(Self::Na),
            "CN" => Ok(Self::Cn),
            "KR" => Ok(Self::Kr),
            "TW" => Ok(Self::Tw),
            _ => Err("region must be one of JP, NA, CN, KR, TW".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSource {
    pub provider: String,
    pub region: Region,
    pub data_hash: String,
    pub data_timestamp: u64,
    pub fetched_at: u64,
    pub index_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraitRecord {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServantRecord {
    pub id: u32,
    pub collection_no: u32,
    pub name: String,
    pub original_name: String,
    pub servant_type: String,
    pub class_id: u32,
    pub class_name: String,
    pub attribute: String,
    pub rarity: u8,
    pub atk_max: u32,
    pub hp_max: u32,
    pub face_url: Option<String>,
    pub traits: Vec<TraitRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServantCatalog {
    pub schema: u32,
    pub source: CatalogSource,
    pub servants: Vec<ServantRecord>,
}

impl ServantCatalog {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let catalog: Self =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn load_latest(root: &Path, region: Region) -> Result<(Self, PathBuf)> {
        let region_dir = root.join(region.code());
        let entries = fs::read_dir(&region_dir)
            .with_context(|| format!("read catalog directory {}", region_dir.display()))?;
        let mut latest: Option<(Self, PathBuf)> = None;
        for entry in entries {
            let path = entry?.path().join("servants.json");
            if !path.is_file() {
                continue;
            }
            let Ok(candidate) = Self::load(&path) else {
                continue;
            };
            if candidate.source.region != region {
                continue;
            }
            let replace = latest.as_ref().is_none_or(|(current, _)| {
                (candidate.source.data_timestamp, candidate.source.fetched_at)
                    > (current.source.data_timestamp, current.source.fetched_at)
            });
            if replace {
                latest = Some((candidate, path));
            }
        }
        latest.with_context(|| format!("no valid {region} servant catalog in {}", root.display()))
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == CATALOG_SCHEMA,
            "unsupported catalog schema {}",
            self.schema
        );
        validate_hash(&self.source.data_hash)?;
        ensure!(self.source.data_timestamp > 0, "invalid Atlas timestamp");
        ensure!(!self.servants.is_empty(), "servant catalog is empty");
        let mut ids = HashSet::with_capacity(self.servants.len());
        for servant in &self.servants {
            ensure!(servant.id > 0, "servant id must be positive");
            ensure!(!servant.name.trim().is_empty(), "servant name is empty");
            ensure!(
                ids.insert(servant.id),
                "duplicate servant id {}",
                servant.id
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    Arts,
    Buster,
    Quick,
}

impl CardType {
    fn parse_atlas(value: &str) -> Result<Self> {
        match value {
            "1" | "arts" => Ok(Self::Arts),
            "2" | "buster" => Ok(Self::Buster),
            "3" | "quick" => Ok(Self::Quick),
            _ => bail!("unsupported Atlas card type {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServantDetailSource {
    pub provider: String,
    pub region: Region,
    pub data_hash: String,
    pub data_timestamp: u64,
    pub fetched_at: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoblePhantasmRecord {
    pub id: u32,
    pub name: String,
    pub card: CardType,
    pub priority: i32,
    pub condition_quest_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCardAsset {
    pub ascension: String,
    pub url: String,
    pub template_file: PathBuf,
}

/// All active skill versions supplied for this servant, without guessing which
/// strengthening quests the player's account has completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: u32,
    pub slot: u8,
    pub name: String,
    pub priority: i32,
    pub strength_status: u32,
    pub condition_quest_id: u32,
    pub condition_quest_phase: u32,
    pub condition_level: u32,
    pub condition_limit_count: u32,
    pub cooldown: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillUnlockStatus {
    Satisfied,
    Unmet,
    Unknown,
}

impl SkillRecord {
    /// Evaluate only the conditions represented in the local Atlas record.
    /// This does not discover account state or choose an active skill version.
    pub fn unlock_status(&self, state: &crate::profile::ServantAccountState) -> SkillUnlockStatus {
        let mut unknown = self.condition_quest_id > 0 && self.condition_quest_phase == 0;
        let quest_requirement = (self.condition_quest_id > 0 && self.condition_quest_phase > 0)
            .then_some(self.condition_quest_phase);
        for (actual, required) in [
            (
                state.level,
                (self.condition_level > 0).then_some(self.condition_level),
            ),
            (
                state.ascension,
                (self.condition_limit_count > 0).then_some(self.condition_limit_count),
            ),
            (
                state.quest_phases.get(&self.condition_quest_id).copied(),
                quest_requirement,
            ),
        ] {
            if let Some(required) = required {
                match actual {
                    Some(actual) if actual < required => return SkillUnlockStatus::Unmet,
                    None => unknown = true,
                    _ => {}
                }
            }
        }
        if unknown {
            SkillUnlockStatus::Unknown
        } else {
            SkillUnlockStatus::Satisfied
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServantDetail {
    pub schema: u32,
    pub source: ServantDetailSource,
    pub id: u32,
    pub collection_no: u32,
    pub name: String,
    pub cards: [CardType; 5],
    pub noble_phantasms: Vec<NoblePhantasmRecord>,
    #[serde(default)]
    pub skills: Vec<SkillRecord>,
    pub command_cards: Vec<CommandCardAsset>,
}

impl ServantDetail {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let detail: Self =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        detail.validate()?;
        Ok(detail)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.schema, 2 | SERVANT_DETAIL_SCHEMA),
            "unsupported servant detail schema {}",
            self.schema
        );
        validate_hash(&self.source.data_hash)?;
        ensure!(self.source.data_timestamp > 0, "invalid Atlas timestamp");
        ensure!(self.id > 0, "servant detail ID must be positive");
        ensure!(!self.name.trim().is_empty(), "servant detail name is empty");
        ensure!(
            !self.noble_phantasms.is_empty(),
            "servant detail has no noble phantasm"
        );
        ensure!(
            self.schema != SERVANT_DETAIL_SCHEMA || !self.skills.is_empty(),
            "servant detail has no active skills"
        );
        let mut skill_ids = HashSet::new();
        for skill in &self.skills {
            ensure!(
                skill.id > 0 && (1..=3).contains(&skill.slot),
                "invalid active skill ID or slot"
            );
            ensure!(!skill.name.trim().is_empty(), "active skill name is empty");
            ensure!(
                skill_ids.insert((skill.slot, skill.id)),
                "duplicate active skill ID {} in slot {}",
                skill.id,
                skill.slot
            );
            ensure!(
                skill.cooldown.len() == 10 && skill.cooldown.iter().all(|turns| *turns > 0),
                "active skill {} needs ten positive cooldown values",
                skill.id
            );
        }
        let mut np_ids = HashSet::with_capacity(self.noble_phantasms.len());
        for noble_phantasm in &self.noble_phantasms {
            ensure!(noble_phantasm.id > 0, "noble phantasm ID must be positive");
            ensure!(
                !noble_phantasm.name.trim().is_empty(),
                "noble phantasm name is empty"
            );
            ensure!(
                np_ids.insert(noble_phantasm.id),
                "duplicate noble phantasm ID {}",
                noble_phantasm.id
            );
        }
        ensure!(
            !self.command_cards.is_empty(),
            "servant detail has no command-card assets"
        );
        let mut ascensions = HashSet::with_capacity(self.command_cards.len());
        let mut files = HashSet::with_capacity(self.command_cards.len());
        for asset in &self.command_cards {
            ensure!(
                !asset.ascension.is_empty()
                    && asset.ascension.len() <= 32
                    && asset
                        .ascension
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
                "invalid command-card ascension"
            );
            ensure!(!asset.url.trim().is_empty(), "empty command-card URL");
            ensure!(
                !asset.template_file.as_os_str().is_empty()
                    && asset
                        .template_file
                        .components()
                        .all(|component| matches!(component, Component::Normal(_))),
                "invalid command-card template path"
            );
            ensure!(
                ascensions.insert(&asset.ascension),
                "duplicate command-card ascension"
            );
            ensure!(
                files.insert(&asset.template_file),
                "duplicate command-card template path"
            );
        }
        Ok(())
    }

    pub fn load_bundle(path: &Path) -> Result<(Self, Vec<RgbaImage>)> {
        let detail = Self::load(path)?;
        let base = path.parent().context("servant detail path has no parent")?;
        let mut templates = Vec::with_capacity(detail.command_cards.len());
        for asset in &detail.command_cards {
            let template_path = base.join(&asset.template_file);
            let template = image::open(&template_path)
                .with_context(|| format!("load {}", template_path.display()))?
                .to_rgba8();
            ensure!(
                template.dimensions() == (COMMAND_TEMPLATE_WIDTH, COMMAND_TEMPLATE_HEIGHT),
                "command-card template {} has invalid dimensions",
                template_path.display()
            );
            templates.push(template);
        }
        Ok((detail, templates))
    }

    pub fn noble_phantasm_card(&self, id: Option<u32>) -> Option<CardType> {
        if let Some(id) = id {
            return self
                .noble_phantasms
                .iter()
                .find(|noble_phantasm| noble_phantasm.id == id)
                .map(|noble_phantasm| noble_phantasm.card);
        }
        let first = self.noble_phantasms.first()?.card;
        self.noble_phantasms
            .iter()
            .all(|noble_phantasm| noble_phantasm.card == first)
            .then_some(first)
    }
}

pub struct PartyServantData {
    pub slot: PartySlot,
    pub atlas_id: u32,
    pub noble_phantasm_card: CardType,
    pub command_card_templates: Vec<RgbaImage>,
}

pub struct SupportServantData {
    pub atlas_id: u32,
    pub noble_phantasm_card: CardType,
    pub command_card_templates: Vec<RgbaImage>,
}

fn servant_detail_path(version_dir: &Path, id: u32) -> PathBuf {
    let path = version_dir.join("details-v3").join(format!("{id}.json"));
    if path.exists() {
        path
    } else {
        version_dir.join("details").join(format!("{id}.json"))
    }
}

pub fn load_party_servant_data(
    root: &Path,
    region: Region,
    profile: &PartyProfile,
) -> Result<Vec<PartyServantData>> {
    if profile.servants.is_empty() {
        return Ok(Vec::new());
    }
    let (catalog, catalog_path) = ServantCatalog::load_latest(root, region).with_context(|| {
        format!(
            "load Atlas data for party profile {}; run sync-data --profile first",
            profile.id
        )
    })?;
    let version_dir = catalog_path
        .parent()
        .context("catalog path has no parent")?;
    let mut result = Vec::with_capacity(profile.servants.len());
    for reference in &profile.servants {
        let detail_path = servant_detail_path(version_dir, reference.atlas_id);
        let (detail, templates) = ServantDetail::load_bundle(&detail_path).with_context(|| {
            format!(
                "load Atlas servant {} for party profile {}; run sync-data --profile first",
                reference.atlas_id, profile.id
            )
        })?;
        ensure!(
            detail.id == reference.atlas_id,
            "servant detail ID mismatch"
        );
        ensure!(
            detail.source.region == region
                && detail.source.data_hash == catalog.source.data_hash
                && detail.source.data_timestamp == catalog.source.data_timestamp,
            "servant detail does not match the latest {region} catalog"
        );
        for rule in profile
            .skill_rules
            .iter()
            .filter(|rule| rule.servant == reference.slot)
        {
            if let Some(id) = rule.atlas_skill_id {
                ensure!(
                    detail.schema == SERVANT_DETAIL_SCHEMA,
                    "Atlas skill references require detail schema 3; run sync-data --profile first"
                );
                let skill = detail
                    .skills
                    .iter()
                    .find(|skill| {
                        skill.id == id && usize::from(skill.slot) == rule.skill.index() + 1
                    })
                    .with_context(|| {
                        format!(
                            "Atlas servant {} does not contain skill {id} in slot {}",
                            reference.atlas_id,
                            rule.skill.index() + 1
                        )
                    })?;
                if let Some(state) = &reference.account_state {
                    let status = skill.unlock_status(state);
                    ensure!(
                        status == SkillUnlockStatus::Satisfied,
                        "Atlas servant {} skill {id} unlock conditions are {status:?} for declared account state (level {}, ascension {}, quest {} phase {} required)",
                        reference.atlas_id,
                        skill.condition_level,
                        skill.condition_limit_count,
                        skill.condition_quest_id,
                        skill.condition_quest_phase
                    );
                }
            }
        }
        let noble_phantasm_card = detail
            .noble_phantasm_card(reference.noble_phantasm_id)
            .with_context(|| {
                if let Some(id) = reference.noble_phantasm_id {
                    format!(
                        "Atlas servant {} does not contain NP {id}",
                        reference.atlas_id
                    )
                } else {
                    format!(
                        "Atlas servant {} has multiple NP colors; set noble_phantasm_id in the profile",
                        reference.atlas_id
                    )
                }
            })?;
        result.push(PartyServantData {
            slot: reference.slot,
            atlas_id: reference.atlas_id,
            noble_phantasm_card,
            command_card_templates: templates,
        });
    }
    Ok(result)
}

pub fn load_support_servant_data(
    root: &Path,
    region: Region,
    profile: &PartyProfile,
) -> Result<Vec<SupportServantData>> {
    if profile.support_candidates.is_empty() {
        return Ok(Vec::new());
    }
    let (catalog, catalog_path) = ServantCatalog::load_latest(root, region).with_context(|| {
        format!(
            "load Atlas data for party profile {}; run sync-data --profile first",
            profile.id
        )
    })?;
    let version_dir = catalog_path
        .parent()
        .context("catalog path has no parent")?;
    let mut result = Vec::with_capacity(profile.support_candidates.len());
    for reference in &profile.support_candidates {
        let detail_path = servant_detail_path(version_dir, reference.atlas_id);
        let (detail, templates) = ServantDetail::load_bundle(&detail_path).with_context(|| {
            format!(
                "load Atlas support candidate {} for party profile {}; run sync-data --profile first",
                reference.atlas_id, profile.id
            )
        })?;
        ensure!(
            detail.id == reference.atlas_id,
            "support servant detail ID mismatch"
        );
        ensure!(
            detail.source.region == region
                && detail.source.data_hash == catalog.source.data_hash
                && detail.source.data_timestamp == catalog.source.data_timestamp,
            "support servant detail does not match the latest {region} catalog"
        );
        for version in &reference.skill_versions {
            ensure!(
                detail.schema == SERVANT_DETAIL_SCHEMA,
                "Atlas support skill references require detail schema 3; run sync-data --profile first"
            );
            ensure!(
                detail
                    .skills
                    .iter()
                    .any(|skill| skill.id == version.atlas_skill_id
                        && usize::from(skill.slot) == version.skill.index() + 1),
                "Atlas support candidate {} does not contain skill {} in slot {}",
                reference.atlas_id,
                version.atlas_skill_id,
                version.skill.index() + 1
            );
        }
        let noble_phantasm_card = detail
            .noble_phantasm_card(reference.noble_phantasm_id)
            .with_context(|| {
                if let Some(id) = reference.noble_phantasm_id {
                    format!(
                        "Atlas support candidate {} does not contain NP {id}",
                        reference.atlas_id
                    )
                } else {
                    format!(
                        "Atlas support candidate {} has multiple NP colors; set noble_phantasm_id in the profile",
                        reference.atlas_id
                    )
                }
            })?;
        result.push(SupportServantData {
            atlas_id: reference.atlas_id,
            noble_phantasm_card,
            command_card_templates: templates,
        });
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncResult {
    pub changed: bool,
    pub region: Region,
    pub data_hash: String,
    pub data_timestamp: u64,
    pub servant_count: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetailSyncResult {
    pub region: Region,
    pub data_hash: String,
    pub requested: usize,
    pub downloaded: usize,
    pub cached: usize,
    pub templates_written: usize,
    pub paths: Vec<PathBuf>,
}

pub struct AtlasClient {
    agent: ureq::Agent,
    base_url: String,
}

impl Default for AtlasClient {
    fn default() -> Self {
        Self::new(DEFAULT_ATLAS_API)
    }
}

impl AtlasClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .build();
        Self {
            agent: config.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn sync(&self, root: &Path, region: Region) -> Result<SyncResult> {
        sync_with(
            root,
            region,
            |url, limit| self.fetch(url, limit),
            &self.base_url,
        )
    }

    pub fn sync_servant_details(
        &self,
        root: &Path,
        region: Region,
        servant_ids: &[u32],
    ) -> Result<DetailSyncResult> {
        sync_details_with(
            root,
            region,
            servant_ids,
            |url, limit| self.fetch(url, limit),
            &self.base_url,
        )
    }

    fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        let mut response = self
            .agent
            .get(url)
            .header("User-Agent", concat!("fgo-rs/", env!("CARGO_PKG_VERSION")))
            .call()
            .with_context(|| format!("GET {url}"))?;
        response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .with_context(|| format!("read {url}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AtlasVersion {
    hash: String,
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasServant {
    id: u32,
    collection_no: u32,
    name: String,
    original_name: String,
    #[serde(rename = "type")]
    servant_type: String,
    class_id: u32,
    class_name: String,
    attribute: String,
    rarity: u8,
    atk_max: u32,
    hp_max: u32,
    face: Option<String>,
    #[serde(default)]
    traits: Vec<TraitRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasNiceServant {
    id: u32,
    collection_no: u32,
    name: String,
    cards: Vec<String>,
    #[serde(default)]
    noble_phantasms: Vec<AtlasNoblePhantasm>,
    skills: Vec<AtlasSkill>,
    #[serde(default)]
    extra_assets: AtlasExtraAssets,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasSkill {
    id: u32,
    num: u8,
    name: String,
    priority: i32,
    strength_status: u32,
    cond_quest_id: u32,
    cond_quest_phase: u32,
    cond_lv: u32,
    cond_limit_count: u32,
    cool_down: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtlasNoblePhantasm {
    id: u32,
    name: String,
    card: String,
    priority: i32,
    #[serde(default)]
    cond_quest_id: u32,
}

#[derive(Debug, Default, Deserialize)]
struct AtlasExtraAssets {
    #[serde(default)]
    commands: AtlasCommandAssets,
}

#[derive(Debug, Default, Deserialize)]
struct AtlasCommandAssets {
    #[serde(default)]
    ascension: BTreeMap<String, String>,
}

impl From<AtlasServant> for ServantRecord {
    fn from(value: AtlasServant) -> Self {
        Self {
            id: value.id,
            collection_no: value.collection_no,
            name: value.name,
            original_name: value.original_name,
            servant_type: value.servant_type,
            class_id: value.class_id,
            class_name: value.class_name,
            attribute: value.attribute,
            rarity: value.rarity,
            atk_max: value.atk_max,
            hp_max: value.hp_max,
            face_url: value.face,
            traits: value.traits,
        }
    }
}

fn sync_with<F>(root: &Path, region: Region, mut fetch: F, base_url: &str) -> Result<SyncResult>
where
    F: FnMut(&str, u64) -> Result<Vec<u8>>,
{
    let info_url = format!("{}/info", base_url.trim_end_matches('/'));
    let info_bytes = fetch(&info_url, INFO_LIMIT)?;
    let version = parse_version(&info_bytes, region)?;
    validate_hash(&version.hash)?;
    ensure!(
        version.timestamp > 0,
        "invalid Atlas timestamp for {region}"
    );

    let final_dir = root.join(region.code()).join(&version.hash);
    let final_path = final_dir.join("servants.json");
    if final_path.is_file() {
        let catalog = ServantCatalog::load(&final_path)?;
        ensure!(
            catalog.source.region == region,
            "cached catalog region mismatch"
        );
        ensure!(
            catalog.source.data_hash == version.hash,
            "cached catalog hash mismatch"
        );
        return Ok(result_for(&catalog, final_path, false));
    }

    let index_url = format!(
        "{}/export/{}/basic_servant.json",
        base_url.trim_end_matches('/'),
        region.code()
    );
    let index_bytes = fetch(&index_url, INDEX_LIMIT)?;
    let confirmed_version = parse_version(&fetch(&info_url, INFO_LIMIT)?, region)?;
    ensure!(
        confirmed_version == version,
        "Atlas {region} data version changed during synchronization; run sync-data again"
    );
    let upstream: Vec<AtlasServant> = serde_json::from_slice(&index_bytes)
        .with_context(|| format!("parse Atlas {region} basic servant export"))?;
    let mut servants: Vec<ServantRecord> = upstream.into_iter().map(Into::into).collect();
    servants.sort_by_key(|servant| (servant.collection_no, servant.id));
    let catalog = ServantCatalog {
        schema: CATALOG_SCHEMA,
        source: CatalogSource {
            provider: "Atlas Academy".into(),
            region,
            data_hash: version.hash.clone(),
            data_timestamp: version.timestamp,
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system time is before Unix epoch")?
                .as_secs(),
            index_url,
        },
        servants,
    };
    catalog.validate()?;

    let region_dir = root.join(region.code());
    fs::create_dir_all(&region_dir).with_context(|| format!("create {}", region_dir.display()))?;
    let staging_path = region_dir.join(format!(
        ".sync-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time is before Unix epoch")?
            .as_nanos()
    ));
    fs::create_dir(&staging_path).with_context(|| format!("create {}", staging_path.display()))?;
    let mut staging = StagingDir(Some(staging_path));
    let staging_file = staging.path().join("servants.json");
    let bytes = serde_json::to_vec_pretty(&catalog).context("serialize servant catalog")?;
    let mut file = File::create(&staging_file)
        .with_context(|| format!("create {}", staging_file.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("write {}", staging_file.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", staging_file.display()))?;
    file.sync_all()
        .with_context(|| format!("flush {}", staging_file.display()))?;
    drop(file);

    match fs::rename(staging.path(), &final_dir) {
        Ok(()) => staging.disarm(),
        Err(_) if final_path.is_file() => {
            let existing = ServantCatalog::load(&final_path)?;
            ensure!(
                existing.source.data_hash == version.hash,
                "catalog update race"
            );
            return Ok(result_for(&existing, final_path, false));
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "publish catalog {} to {}",
                    staging.path().display(),
                    final_dir.display()
                )
            });
        }
    }

    Ok(result_for(&catalog, final_path, true))
}

fn sync_details_with<F>(
    root: &Path,
    region: Region,
    servant_ids: &[u32],
    mut fetch: F,
    base_url: &str,
) -> Result<DetailSyncResult>
where
    F: FnMut(&str, u64) -> Result<Vec<u8>>,
{
    ensure!(!servant_ids.is_empty(), "profile has no Atlas servant IDs");
    let mut requested = servant_ids.to_vec();
    requested.sort_unstable();
    requested.dedup();
    ensure!(
        requested.iter().all(|id| *id > 0),
        "Atlas servant IDs must be positive"
    );

    let info_url = format!("{}/info", base_url.trim_end_matches('/'));
    let version = parse_version(&fetch(&info_url, INFO_LIMIT)?, region)?;
    validate_hash(&version.hash)?;
    ensure!(
        version.timestamp > 0,
        "invalid Atlas timestamp for {region}"
    );

    let version_dir = root.join(region.code()).join(&version.hash);
    let catalog_path = version_dir.join("servants.json");
    let catalog = ServantCatalog::load(&catalog_path).with_context(|| {
        format!("load current {region} servant index before syncing details; run sync-data first")
    })?;
    ensure!(
        catalog.source.region == region,
        "cached catalog region mismatch"
    );
    ensure!(
        catalog.source.data_hash == version.hash
            && catalog.source.data_timestamp == version.timestamp,
        "cached catalog is not the current Atlas {region} version; run sync-data first"
    );
    let indexed: HashSet<_> = catalog.servants.iter().map(|servant| servant.id).collect();
    for id in &requested {
        ensure!(
            indexed.contains(id),
            "Atlas servant ID {id} is not in the {region} index"
        );
    }

    let details_dir = version_dir.join("details-v3");
    let mut paths = Vec::with_capacity(requested.len());
    let mut cached = 0;
    let mut pending = Vec::new();
    for id in requested {
        let path = details_dir.join(format!("{id}.json"));
        let valid = ServantDetail::load_bundle(&path).is_ok_and(|(detail, _)| {
            detail.schema == SERVANT_DETAIL_SCHEMA
                && detail.id == id
                && detail.source.region == region
                && detail.source.data_hash == version.hash
                && detail.source.data_timestamp == version.timestamp
        });
        if valid {
            cached += 1;
        } else {
            let url = format!(
                "{}/nice/{}/servant/{id}",
                base_url.trim_end_matches('/'),
                region.code()
            );
            let upstream: AtlasNiceServant =
                serde_json::from_slice(&fetch(&url, NICE_SERVANT_LIMIT)?)
                    .with_context(|| format!("parse Atlas {region} nice servant {id}"))?;
            ensure!(
                upstream.id == id,
                "Atlas nice servant ID mismatch: requested {id}, received {}",
                upstream.id
            );
            let detail = detail_from_atlas(upstream, region, &version, url)?;
            let mut templates = Vec::with_capacity(detail.command_cards.len());
            for asset in &detail.command_cards {
                let bytes = fetch(&asset.url, COMMAND_CARD_ASSET_LIMIT)?;
                templates.push(build_command_card_template(&bytes).with_context(|| {
                    format!(
                        "prepare Atlas command-card asset {} for servant {id}",
                        asset.url
                    )
                })?);
            }
            pending.push((detail, templates));
        }
        paths.push(path);
    }

    if !pending.is_empty() {
        let confirmed = parse_version(&fetch(&info_url, INFO_LIMIT)?, region)?;
        ensure!(
            confirmed == version,
            "Atlas {region} data version changed during detail synchronization; run sync-data again"
        );
        fs::create_dir_all(&details_dir)
            .with_context(|| format!("create {}", details_dir.display()))?;
        let staging_path = version_dir.join(format!(
            ".details-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system time is before Unix epoch")?
                .as_nanos()
        ));
        fs::create_dir(&staging_path)
            .with_context(|| format!("create {}", staging_path.display()))?;
        let staging = StagingDir(Some(staging_path));
        for (detail, templates) in &pending {
            for (asset, bytes) in detail.command_cards.iter().zip(templates) {
                write_bytes(&staging.path().join(&asset.template_file), bytes)?;
            }
            let staged_path = staging.path().join(format!("{}.json", detail.id));
            write_json(&staged_path, detail)?;
        }
        for (detail, _) in &pending {
            for asset in &detail.command_cards {
                let staged_path = staging.path().join(&asset.template_file);
                let final_path = details_dir.join(&asset.template_file);
                if let Some(parent) = final_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create {}", parent.display()))?;
                }
                if final_path.exists() {
                    fs::remove_file(&final_path).with_context(|| {
                        format!("replace invalid cache {}", final_path.display())
                    })?;
                }
                fs::rename(&staged_path, &final_path).with_context(|| {
                    format!(
                        "publish command-card template {} to {}",
                        staged_path.display(),
                        final_path.display()
                    )
                })?;
            }
            let staged_path = staging.path().join(format!("{}.json", detail.id));
            let final_path = details_dir.join(format!("{}.json", detail.id));
            if final_path.exists() {
                fs::remove_file(&final_path)
                    .with_context(|| format!("replace invalid cache {}", final_path.display()))?;
            }
            fs::rename(&staged_path, &final_path).with_context(|| {
                format!(
                    "publish servant detail {} to {}",
                    staged_path.display(),
                    final_path.display()
                )
            })?;
        }
    }

    Ok(DetailSyncResult {
        region,
        data_hash: version.hash,
        requested: paths.len(),
        downloaded: pending.len(),
        cached,
        templates_written: pending
            .iter()
            .map(|(detail, _)| detail.command_cards.len())
            .sum(),
        paths,
    })
}

fn detail_from_atlas(
    upstream: AtlasNiceServant,
    region: Region,
    version: &AtlasVersion,
    url: String,
) -> Result<ServantDetail> {
    ensure!(
        upstream.cards.len() == 5,
        "Atlas servant {} has {} face cards instead of 5",
        upstream.id,
        upstream.cards.len()
    );
    let cards: Vec<_> = upstream
        .cards
        .iter()
        .map(|card| CardType::parse_atlas(card))
        .collect::<Result<_>>()?;
    let mut noble_phantasms = upstream
        .noble_phantasms
        .into_iter()
        .map(|noble_phantasm| {
            Ok(NoblePhantasmRecord {
                id: noble_phantasm.id,
                name: noble_phantasm.name,
                card: CardType::parse_atlas(&noble_phantasm.card)?,
                priority: noble_phantasm.priority,
                condition_quest_id: noble_phantasm.cond_quest_id,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    noble_phantasms.sort_by_key(|noble_phantasm| (noble_phantasm.priority, noble_phantasm.id));
    let detail = ServantDetail {
        schema: SERVANT_DETAIL_SCHEMA,
        source: ServantDetailSource {
            provider: "Atlas Academy".into(),
            region,
            data_hash: version.hash.clone(),
            data_timestamp: version.timestamp,
            fetched_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system time is before Unix epoch")?
                .as_secs(),
            url,
        },
        id: upstream.id,
        collection_no: upstream.collection_no,
        name: upstream.name,
        cards: cards.try_into().expect("five cards checked"),
        skills: upstream
            .skills
            .into_iter()
            .map(|skill| SkillRecord {
                id: skill.id,
                slot: skill.num,
                name: skill.name,
                priority: skill.priority,
                strength_status: skill.strength_status,
                condition_quest_id: skill.cond_quest_id,
                condition_quest_phase: skill.cond_quest_phase,
                condition_level: skill.cond_lv,
                condition_limit_count: skill.cond_limit_count,
                cooldown: skill.cool_down,
            })
            .collect(),
        noble_phantasms,
        command_cards: upstream
            .extra_assets
            .commands
            .ascension
            .into_iter()
            .map(|(ascension, url)| CommandCardAsset {
                template_file: PathBuf::from(format!("assets/{}/{ascension}.png", upstream.id)),
                ascension,
                url,
            })
            .collect(),
    };
    detail.validate()?;
    Ok(detail)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).context("serialize JSON cache")?;
    bytes.push(b'\n');
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("flush {}", path.display()))?;
    Ok(())
}

fn build_command_card_template(bytes: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .context("decode command-card PNG")?
        .to_rgba8();
    ensure!(
        image.dimensions() == (COMMAND_CARD_SOURCE_SIZE, COMMAND_CARD_SOURCE_SIZE),
        "Atlas command-card asset must be 256x256"
    );
    let template = imageops::crop_imm(
        &image,
        COMMAND_TEMPLATE_X,
        COMMAND_TEMPLATE_Y,
        COMMAND_TEMPLATE_WIDTH,
        COMMAND_TEMPLATE_HEIGHT,
    )
    .to_image();
    ensure!(
        template.pixels().any(|pixel| pixel[3] > 0),
        "command-card template has an empty mask"
    );
    let mut output = Cursor::new(Vec::new());
    template
        .write_to(&mut output, ImageFormat::Png)
        .context("encode command-card template")?;
    Ok(output.into_inner())
}

fn parse_version(bytes: &[u8], region: Region) -> Result<AtlasVersion> {
    let versions: BTreeMap<String, AtlasVersion> =
        serde_json::from_slice(bytes).context("parse Atlas /info response")?;
    versions
        .get(region.code())
        .cloned()
        .with_context(|| format!("Atlas /info does not contain region {region}"))
}

fn result_for(catalog: &ServantCatalog, path: PathBuf, changed: bool) -> SyncResult {
    SyncResult {
        changed,
        region: catalog.source.region,
        data_hash: catalog.source.data_hash.clone(),
        data_timestamp: catalog.source.data_timestamp,
        servant_count: catalog.servants.len(),
        path,
    }
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.is_empty() || hash.len() > 64 || !hash.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        bail!("invalid Atlas data hash")
    }
    Ok(())
}

struct StagingDir(Option<PathBuf>);

impl StagingDir {
    fn path(&self) -> &Path {
        self.0.as_deref().expect("staging directory is armed")
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for StagingDir {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    const SERVANTS: &str = r#"[
      {
        "id": 400200,
        "collectionNo": 108,
        "name": "伊斯坎达尔",
        "originalName": "伊斯坎达尔",
        "type": "normal",
        "classId": 4,
        "className": "rider",
        "attribute": "human",
        "rarity": 5,
        "atkMax": 11560,
        "hpMax": 13219,
        "face": "https://static.atlasacademy.io/CN/Faces/f_4002000.png",
        "traits": [{"id": 1, "name": "genderMale"}]
      }
    ]"#;

    const NICE_SERVANT: &str = r#"{
      "id": 400200,
      "collectionNo": 108,
      "name": "伊斯坎达尔",
      "cards": ["3", "1", "1", "2", "2"],
      "skills": [
        {"id":5550,"num":1,"name":"领袖气质 A","priority":1,"strengthStatus":1,"condQuestId":0,"condQuestPhase":0,"condLv":0,"condLimitCount":0,"coolDown":[7,7,7,7,7,6,6,6,6,5]},
        {"id":2382550,"num":1,"name":"奔赴彼方的人们 A","priority":2,"strengthStatus":99,"condQuestId":94099428,"condQuestPhase":3,"condLv":0,"condLimitCount":0,"coolDown":[7,7,7,7,7,6,6,6,6,5]}
      ],
      "noblePhantasms": [
        {
          "id": 400201,
          "name": "王之军势",
          "card": "2",
          "priority": 101,
          "condQuestId": 0
        },
        {
          "id": 400202,
          "name": "王之军势",
          "card": "2",
          "priority": 102,
          "condQuestId": 123
        }
      ],
      "extraAssets": {
        "commands": {
          "ascension": {
            "1": "https://static.atlasacademy.io/CN/Servants/Commands/400200/card_servant_1.png",
            "3": "https://static.atlasacademy.io/CN/Servants/Commands/400200/card_servant_3.png"
          }
        }
      }
    }"#;

    fn seed_catalog(root: &Path) {
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
            } else {
                Ok(SERVANTS.as_bytes().to_vec())
            }
        };
        sync_with(root, Region::Cn, fetch, "https://example.test").unwrap();
    }

    fn command_png() -> Vec<u8> {
        let image = RgbaImage::from_pixel(
            COMMAND_CARD_SOURCE_SIZE,
            COMMAND_CARD_SOURCE_SIZE,
            image::Rgba([10, 20, 30, 255]),
        );
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Png).unwrap();
        output.into_inner()
    }

    #[test]
    fn region_is_case_insensitive_and_defaults_to_cn() {
        assert_eq!(Region::default(), Region::Cn);
        assert_eq!("cn".parse::<Region>().unwrap(), Region::Cn);
        assert_eq!("JP".parse::<Region>().unwrap(), Region::Jp);
        assert!("global".parse::<Region>().is_err());
    }

    #[test]
    fn sync_writes_versioned_catalog_and_reuses_it() {
        let directory = tempfile::tempdir().unwrap();
        let responses = HashMap::from([
            (
                "https://example.test/info".to_string(),
                br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec(),
            ),
            (
                "https://example.test/export/CN/basic_servant.json".to_string(),
                SERVANTS.as_bytes().to_vec(),
            ),
        ]);
        let calls = RefCell::new(Vec::new());
        let fetch = |url: &str, limit: u64| {
            calls.borrow_mut().push((url.to_string(), limit));
            responses.get(url).cloned().context("unexpected URL")
        };
        let first = sync_with(directory.path(), Region::Cn, fetch, "https://example.test").unwrap();
        assert!(first.changed);
        assert_eq!(first.servant_count, 1);
        assert_eq!(first.path, directory.path().join("CN/abc123/servants.json"));
        let catalog = ServantCatalog::load(&first.path).unwrap();
        assert_eq!(catalog.servants[0].name, "伊斯坎达尔");
        assert_eq!(catalog.source.region, Region::Cn);
        assert_eq!(calls.borrow().len(), 3, "new version is checked twice");

        calls.borrow_mut().clear();
        let second =
            sync_with(directory.path(), Region::Cn, fetch, "https://example.test").unwrap();
        assert!(!second.changed);
        assert_eq!(calls.borrow().len(), 1, "cached version only fetches /info");
    }

    #[test]
    fn malformed_export_does_not_publish_a_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
            } else {
                Ok(b"[]".to_vec())
            }
        };
        let error =
            sync_with(directory.path(), Region::Cn, fetch, "https://example.test").unwrap_err();
        assert!(error.to_string().contains("servant catalog is empty"));
        assert!(!directory.path().join("CN/abc123").exists());
    }

    #[test]
    fn changing_upstream_version_does_not_publish_mixed_data() {
        let directory = tempfile::tempdir().unwrap();
        let info_calls = RefCell::new(0);
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                let mut calls = info_calls.borrow_mut();
                *calls += 1;
                let hash = if *calls == 1 { "abc123" } else { "def456" };
                Ok(format!(r#"{{"CN":{{"hash":"{hash}","timestamp":123}}}}"#).into_bytes())
            } else {
                Ok(SERVANTS.as_bytes().to_vec())
            }
        };
        let error =
            sync_with(directory.path(), Region::Cn, fetch, "https://example.test").unwrap_err();
        assert!(error.to_string().contains("changed during synchronization"));
        assert!(!directory.path().join("CN/abc123").exists());
    }

    #[test]
    fn load_latest_uses_upstream_timestamp_not_directory_order() {
        let directory = tempfile::tempdir().unwrap();
        for (hash, timestamp) in [("ffff", 1), ("aaaa", 2)] {
            let path = directory.path().join("CN").join(hash);
            fs::create_dir_all(&path).unwrap();
            let catalog = ServantCatalog {
                schema: CATALOG_SCHEMA,
                source: CatalogSource {
                    provider: "Atlas Academy".into(),
                    region: Region::Cn,
                    data_hash: hash.into(),
                    data_timestamp: timestamp,
                    fetched_at: timestamp,
                    index_url: "https://example.test".into(),
                },
                servants: vec![ServantRecord {
                    id: timestamp as u32,
                    collection_no: timestamp as u32,
                    name: hash.into(),
                    original_name: hash.into(),
                    servant_type: "normal".into(),
                    class_id: 4,
                    class_name: "rider".into(),
                    attribute: "human".into(),
                    rarity: 5,
                    atk_max: 1,
                    hp_max: 1,
                    face_url: None,
                    traits: Vec::new(),
                }],
            };
            fs::write(
                path.join("servants.json"),
                serde_json::to_vec(&catalog).unwrap(),
            )
            .unwrap();
        }
        let (latest, path) = ServantCatalog::load_latest(directory.path(), Region::Cn).unwrap();
        assert_eq!(latest.source.data_hash, "aaaa");
        assert!(path.ends_with("CN/aaaa/servants.json"));
    }

    #[test]
    fn syncs_compact_profile_details_and_reuses_the_cache() {
        let directory = tempfile::tempdir().unwrap();
        seed_catalog(directory.path());
        let calls = RefCell::new(Vec::new());
        let fetch = |url: &str, limit: u64| {
            calls.borrow_mut().push((url.to_string(), limit));
            if url.ends_with("/info") {
                Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
            } else if url.ends_with("/nice/CN/servant/400200") {
                Ok(NICE_SERVANT.as_bytes().to_vec())
            } else if url.starts_with("https://static.atlasacademy.io/") {
                Ok(command_png())
            } else {
                bail!("unexpected URL {url}")
            }
        };

        let first = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200, 400200],
            fetch,
            "https://example.test",
        )
        .unwrap();
        assert_eq!(first.requested, 1);
        assert_eq!(first.downloaded, 1);
        assert_eq!(first.cached, 0);
        assert_eq!(calls.borrow().len(), 5, "new details are version-checked");
        assert_eq!(calls.borrow()[1].1, NICE_SERVANT_LIMIT);
        assert_eq!(calls.borrow()[2].1, COMMAND_CARD_ASSET_LIMIT);
        assert_eq!(first.templates_written, 2);

        let (detail, templates) = ServantDetail::load_bundle(&first.paths[0]).unwrap();
        assert_eq!(
            detail.cards,
            [
                CardType::Quick,
                CardType::Arts,
                CardType::Arts,
                CardType::Buster,
                CardType::Buster,
            ]
        );
        assert_eq!(detail.noble_phantasm_card(None), Some(CardType::Buster));
        assert_eq!(
            detail.noble_phantasm_card(Some(400202)),
            Some(CardType::Buster)
        );
        assert_eq!(detail.schema, 3);
        assert_eq!(detail.skills.len(), 2);
        assert_eq!(detail.skills[0].id, 5550);
        assert_eq!(detail.skills[1].condition_quest_id, 94099428);
        assert_eq!(detail.skills[1].condition_quest_phase, 3);
        assert_eq!(detail.skills[1].cooldown[9], 5);
        let mut malformed = detail.clone();
        malformed.skills[0].cooldown.pop();
        assert!(malformed.validate().is_err());
        let mut malformed = detail.clone();
        malformed.skills.push(malformed.skills[0].clone());
        assert!(malformed.validate().is_err());
        let mut malformed = detail.clone();
        malformed.skills[0].slot = 4;
        assert!(malformed.validate().is_err());
        assert_eq!(detail.command_cards.len(), 2);
        assert_eq!(templates.len(), 2);
        assert!(
            templates
                .iter()
                .all(|template| template.dimensions() == (98, 47))
        );

        let mut profile = PartyProfile::default();
        profile.servants.truncate(1);
        let party_data = load_party_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        assert_eq!(party_data.len(), 1);
        assert_eq!(party_data[0].slot, PartySlot::One);
        assert_eq!(party_data[0].atlas_id, 400200);
        assert_eq!(party_data[0].noble_phantasm_card, CardType::Buster);
        assert_eq!(party_data[0].command_card_templates.len(), 2);

        profile.support_candidates = vec![crate::profile::SupportServantReference {
            atlas_id: 400200,
            noble_phantasm_id: None,
            skill_versions: Vec::new(),
        }];
        let support_data =
            load_support_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        assert_eq!(support_data.len(), 1);
        assert_eq!(support_data[0].atlas_id, 400200);
        assert_eq!(support_data[0].noble_phantasm_card, CardType::Buster);
        assert_eq!(support_data[0].command_card_templates.len(), 2);
        profile.support_candidates[0].skill_versions = vec![crate::profile::SupportSkillVersion {
            skill: crate::domain::SkillSlot::One,
            atlas_skill_id: 2382550,
        }];
        profile.validate().unwrap();
        load_support_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        profile.support_candidates[0].skill_versions[0].skill = crate::domain::SkillSlot::Two;
        assert!(
            load_support_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("does not contain skill")
        );
        profile.support_candidates[0].skill_versions[0].skill = crate::domain::SkillSlot::One;
        profile.support_candidates[0].skill_versions[0].atlas_skill_id = 999;
        assert!(load_support_servant_data(directory.path(), Region::Cn, &profile).is_err());
        profile.support_candidates[0].skill_versions[0].atlas_skill_id = 2382550;

        profile.skill_rules = vec![crate::profile::ServantSkillRule {
            servant: PartySlot::One,
            skill: crate::domain::SkillSlot::One,
            target: PartySlot::Three.into(),
            hp_below: None,
            np_below: None,
            enemy_charge_full: false,
            any_of: Vec::new(),
            earliest: None,
            atlas_skill_id: Some(2382550),
        }];
        profile.validate().unwrap();
        load_party_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        profile.servants[0].account_state = Some(Default::default());
        assert!(
            load_party_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("Unknown")
        );
        profile.servants[0]
            .account_state
            .as_mut()
            .unwrap()
            .quest_phases
            .insert(94099428, 2);
        assert!(
            load_party_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("Unmet")
        );
        profile.servants[0]
            .account_state
            .as_mut()
            .unwrap()
            .quest_phases
            .insert(94099428, 3);
        load_party_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        profile.servants[0].account_state = None;
        profile.skill_rules[0].skill = crate::domain::SkillSlot::Two;
        assert!(
            load_party_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("does not contain skill")
        );
        profile.skill_rules[0].skill = crate::domain::SkillSlot::One;
        profile.skill_rules[0].atlas_skill_id = Some(999);
        assert!(load_party_servant_data(directory.path(), Region::Cn, &profile).is_err());
        profile.skill_rules[0].atlas_skill_id = Some(2382550);

        calls.borrow_mut().clear();
        let second = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            fetch,
            "https://example.test",
        )
        .unwrap();
        assert_eq!(second.downloaded, 0);
        assert_eq!(second.cached, 1);
        assert_eq!(second.templates_written, 0);
        assert_eq!(calls.borrow().len(), 1, "valid details only fetch /info");
        // Old snapshots remain byte-for-byte intact while explicit sync writes v3.
        let version_dir = first.paths[0].parent().unwrap().parent().unwrap();
        fs::rename(version_dir.join("details-v3"), version_dir.join("details")).unwrap();
        let old_path = version_dir.join("details/400200.json");
        let mut legacy = serde_json::to_value(&detail).unwrap();
        legacy["schema"] = 2.into();
        legacy.as_object_mut().unwrap().remove("skills");
        let old_bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&old_path, &old_bytes).unwrap();
        assert!(
            load_party_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("sync-data")
        );
        assert!(
            load_support_servant_data(directory.path(), Region::Cn, &profile)
                .err()
                .unwrap()
                .to_string()
                .contains("sync-data")
        );
        profile.support_candidates[0].skill_versions.clear();
        load_support_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        profile.skill_rules.clear();
        load_party_servant_data(directory.path(), Region::Cn, &profile).unwrap();
        let upgrade = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            fetch,
            "https://example.test",
        )
        .unwrap();
        assert_eq!(upgrade.downloaded, 1);
        assert_eq!(fs::read(old_path).unwrap(), old_bytes);
        assert_eq!(
            ServantDetail::load(&upgrade.paths[0]).unwrap().skills.len(),
            2
        );
    }

    #[test]
    fn unlock_conditions_distinguish_partial_unknown_and_unmet_facts() {
        use crate::profile::ServantAccountState;
        let mut skill = SkillRecord {
            id: 1,
            slot: 1,
            name: "test".into(),
            priority: 1,
            strength_status: 1,
            condition_level: 50,
            condition_limit_count: 3,
            condition_quest_id: 123,
            condition_quest_phase: 3,
            cooldown: vec![7; 10],
        };
        let mut state = ServantAccountState::default();
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Unknown);
        state.level = Some(49);
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Unmet);
        state.level = Some(50);
        state.ascension = Some(2);
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Unmet);
        state.ascension = Some(3);
        state.quest_phases.insert(123, 0);
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Unmet);
        state.quest_phases.insert(123, 3);
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Satisfied);
        state.quest_phases.insert(123, 4);
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Satisfied);
        skill.condition_quest_phase = 0;
        assert_eq!(skill.unlock_status(&state), SkillUnlockStatus::Unknown);
        skill.condition_quest_id = 0;
        skill.condition_level = 0;
        skill.condition_limit_count = 0;
        assert_eq!(
            skill.unlock_status(&ServantAccountState::default()),
            SkillUnlockStatus::Satisfied
        );
    }

    #[test]
    fn malformed_skills_are_not_published() {
        let directory = tempfile::tempdir().unwrap();
        seed_catalog(directory.path());
        let mut nice: serde_json::Value = serde_json::from_str(NICE_SERVANT).unwrap();
        nice["skills"][0]["coolDown"] = serde_json::json!([0]);
        let error = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            |url, _| {
                if url.ends_with("/info") {
                    Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
                } else {
                    Ok(serde_json::to_vec(&nice).unwrap())
                }
            },
            "https://example.test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("cooldown"));
        assert!(!directory.path().join("CN/abc123/details-v3").exists());
    }

    #[test]
    fn changing_version_does_not_publish_servant_details() {
        let directory = tempfile::tempdir().unwrap();
        seed_catalog(directory.path());
        let info_calls = RefCell::new(0);
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                let mut calls = info_calls.borrow_mut();
                *calls += 1;
                let hash = if *calls == 1 { "abc123" } else { "def456" };
                Ok(format!(r#"{{"CN":{{"hash":"{hash}","timestamp":123}}}}"#).into_bytes())
            } else if url.ends_with("/nice/CN/servant/400200") {
                Ok(NICE_SERVANT.as_bytes().to_vec())
            } else {
                Ok(command_png())
            }
        };

        let error = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            fetch,
            "https://example.test",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed during detail synchronization")
        );
        assert!(!directory.path().join("CN/abc123/details-v3").exists());
    }

    #[test]
    fn mismatched_servant_detail_is_not_published() {
        let directory = tempfile::tempdir().unwrap();
        seed_catalog(directory.path());
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
            } else {
                Ok(NICE_SERVANT.replace("400200", "400201").into_bytes())
            }
        };

        let error = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            fetch,
            "https://example.test",
        )
        .unwrap_err();
        assert!(error.to_string().contains("ID mismatch"));
        assert!(
            !directory
                .path()
                .join("CN/abc123/details-v3/400200.json")
                .exists()
        );
    }

    #[test]
    fn malformed_command_asset_is_not_published() {
        let directory = tempfile::tempdir().unwrap();
        seed_catalog(directory.path());
        let fetch = |url: &str, _limit: u64| {
            if url.ends_with("/info") {
                Ok(br#"{"CN":{"hash":"abc123","timestamp":123}}"#.to_vec())
            } else if url.ends_with("/nice/CN/servant/400200") {
                Ok(NICE_SERVANT.as_bytes().to_vec())
            } else {
                Ok(b"not a PNG".to_vec())
            }
        };

        let error = sync_details_with(
            directory.path(),
            Region::Cn,
            &[400200],
            fetch,
            "https://example.test",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("prepare Atlas command-card asset")
        );
        assert!(!directory.path().join("CN/abc123/details-v3").exists());
    }
}
