# FGO-rs

Rust 核心 + Rhai 策略的 FGO 自动化项目，提供 Windows / macOS CLI 与可选 Slint GUI。当前适配 CN 界面，支持骑阶、剑阶冠位战 VII 专用策略及 GUI 固定位置刷图。

剑阶冠位战 VII 已加入策略、Profile 和实机识别模板；流程及验收说明见 [剑阶 VII](docs/saber-vii.md)。

## 当前功能

- ADB 连接、截图、点击和滑动；横屏归一化到 1280×720 并映射输入坐标。
- 纯 Rust 模板识别、显式未知状态与原始识别证据、互斥场景冲突检查、带模拟时间的 PNG/JSONL 回放、语义差异报告、逐帧录制及失败截图。
- 骑阶 120/120 级好友筛选、翻页、最多 10 次刷新、编队第 3 位等级确认。
- 从关卡列表、助战列表、编队或连续出击提示进入战斗。
- 技能冷却确认、我方目标选择、1/5 换人、御主技能、首轮三宝具及后续回合选卡。
- 牵绊、经验、战利品结算及连续出击；战斗次数、苹果额度和总运行时间限制。
- 暂停、继续、停止、Ctrl-C 和 ADB 子进程超时处理。
- 可选 Slint 桌面界面：设备发现与预览、截图点选固定位置刷图、按序号选编队、Profile/脚本和预算设置、MaaFramework 识别、开始/暂停/继续/停止及运行日志。
- 从 Atlas Academy 按区服同步版本化的本地从者索引，并按 Profile 缓存所需详情；默认国服。

默认构建不依赖 Python、Qt、OpenCV 或 ONNX Runtime。实机输入需要单独提供 ADB。

可选 MaaFramework 后端提供原生模板匹配、卡色 OCR 补充；配置 OCR 模型后，敌方 HP 和 NP 数字也由 Maa 识别。NP 仍检查文字完整性，友方 HP 沿用 Rust；身份、充能和动作校验由 Rust 核心判断。构建时启用 `--features maa`，运行时提供 `--maa-library` 和可选 `--maa-ocr-model`，详见 [识别后端](docs/recognition.md)。

当前已支持 CN 普通 1～3 敌与六敌布局（含部分敌人退场后的稀疏占位）的 HP、充能和最高 HP 选敌，以及指令卡同从者分组、Atlas 身份识别、原始队伍槽位感知的宝具排序和 Profile 化选卡评分；已配置从者及候选助战的宝具与普通卡可以共同形成同色 Chain 或 Brave Chain。普通布局已有单敌、二敌和三敌实机语料；其他区服、完整 OCR、GUI 配置持久化和安装包尚待补齐。桌面窗口已在 macOS ARM64 本机验收，Windows 和 Intel macOS 尚待运行验收。

## 构建

```sh
cargo build --locked --release
cargo test --locked
```

Windows MSVC 需要 Visual Studio C++ Build Tools；macOS 需要 Xcode Command Line Tools。本机使用 Windows GNU 工具链，操作见 [本机构建说明](docs/local-windows.md)。
编译结果位于 `target/release/fgo-rs`，Windows 为 `.exe`。以下命令从项目根目录运行。

## 桌面界面

```sh
# GUI 固定使用 MaaFramework，gui feature 自动启用 maa
cargo run --locked --features gui --bin fgo-rs-gui
# 指定 ADB
cargo run --locked --features gui --bin fgo-rs-gui -- --adb /path/to/adb
# 发布构建
cargo build --locked --release --features gui --bin fgo-rs-gui
```

默认 **固定位置刷图、只读演练、1 场、0 苹果**；发送点击/滑动需在界面选择“执行输入”。固定位置模式目前支持 CN 界面：

1. 在游戏中打开目标关卡列表，在 GUI 点击“在截图上点选关卡”，再点击预览中的关卡位置；也可直接填写 X / Y。点选只设置坐标，默认 `(845, 203)` 对应列表第一项。空闲与运行画面使用相同的 1280×720 归一化和设备坐标映射，窗口缩放、预览留边不影响坐标。
2. 设置编队序号：从左至右 1–15；**0 保持当前队伍**。选择非零编队后会确认所选标签再开始战斗；受限编队没有可识别标签时停止，请设为 0。
3. 设置场次、苹果预算和运行时长，选择“执行输入”后开始。助战直接选择当前列表第一项，不限制从者、职阶或等级；无可用助战时仍按现有次数限制刷新。

固定模式不需要 Profile、Rhai 脚本或 Atlas 数据。默认战斗参考 MaaFgo 的无计划策略：每回合按当前前排 1–3 位、各技能 1–3 的顺序，重新识别并施放可用从者技能；冷却、封印和未知技能分别跳过，未知会写入日志。需要选择友方目标时默认选择施法者本人；完整识别本次施放后的技能确认弹窗才确认一次，并等待技能进入冷却。每回合攻击前选择当前血量最高的敌人并确认目标；已选中时不重复点击，血量布局或当前目标不完整时停止。随后自动使用可用宝具：攻击前必须确认该槽位 NP 为 100–300%，再结合选卡页的可用/封印状态，避免把退场后的空位亮度当作宝具；NP 未知或不足时使用普通卡。剩余指令卡沿用现有默认评分与连携策略；御主技能和换人关闭，选卡前沿用默认倍速确认。这个通用策略不识别技能效果，不具备 FGO-py 基于从者数据的充能、治疗和增伤时机/目标决策。支持从关卡列表、助战、编队、连续出击提示或战斗回合开始；结算后按场次与苹果预算继续。未知场景、卡色、无法确认编队、特殊技能弹窗或施放后无法确认冷却时保留核心的停止行为。可切换“Profile / Rhai 脚本”使用原有冠位战策略；显式传入 `--script` / `--profile` 时默认进入脚本模式。

空闲时自动预览所选设备；开始后停止独立预览，改为显示运行核心的归一化截图。暂停时间计入最长运行时长，停止和关闭窗口都会取消后台任务。日志区保留最新 300 条，完整 JSONL、逐帧截图及失败截图写入独立运行目录。

资源目录包含 `assets/`、`profiles/` 和 `scripts/`。程序依次检查可执行文件旁的 `resources/`、macOS Bundle 的 `Contents/Resources/` 和当前目录；开发构建还可回退到源码目录。在其他目录启动时可显式传入 `--resources /path/to/FGO-rs`。`--profile`、`--script` 可加入外部文件并默认选中，`--output` 可指定可写日志目录。默认输出位于 macOS 的 `~/Library/Application Support/FGO-rs/logs`、Windows 的 `%LOCALAPPDATA%\FGO-rs\logs`。Atlas 数据仍通过 CLI 的 `sync-data --profile ...` 预先同步，界面“路径设置”可指定对应 `data/atlas` 根目录。

```sh
# 离线回放，强制禁止设备输入
cargo run --locked --features gui --bin fgo-rs-gui -- \
  --resources . --replay tests/fixtures/step2.png \
  --profile profiles/rider-v1.json --script scripts/inspect_rider.rhai
```

GUI 需提供匹配版本的 SDK 动态库与完整 OCR 模型，路径可在界面设置；缺失时会提示并阻止开始任务。CLI 仍支持不依赖 SDK 的 Rust 后端。资源定位、只读模式限制及平台范围见 [GUI 使用](docs/gui.md)。

## 连接与检查

```sh
cargo run --locked -- devices
cargo run --locked -- connect 127.0.0.1:5559
cargo run --locked -- screenshot --serial 127.0.0.1:5559 --output logs/screen.png
cargo run --locked -- inspect tests/fixtures/live-formation.png --farm
cargo run --locked -- inspect tests/fixtures/step12.png --battle
cargo run --locked -- inspect tests/fixtures/step12.png --battle --profile profiles/rider-v1.json
cargo run --locked -- run scripts/inspect_rider.rhai --replay tests/fixtures/step2.png
```

ADB 不在 PATH 时传入 `--adb /path/to/adb`。设备序列号必须显式指定。macOS 上的游戏载体同样需要是支持 ADB 的 Android 设备或模拟器。
`--farm` 输出关卡、助战、AP 和结算状态；`--battle` 输出技能、换人、目标、指令卡等状态。

## 本地从者数据

联网更新与自动化运行相互独立。以下命令先读取 Atlas Academy 的数据版本，再同步轻量从者索引；区服支持 `JP`、`NA`、`CN`、`KR`、`TW`，不传 `--region` 时默认为国服 `CN`：

```sh
cargo run --locked -- sync-data
cargo run --locked -- sync-data --region JP
cargo run --locked -- sync-data --profile profiles/rider-v1.json
```

快照写入 `data/atlas/<区服>/<Atlas hash>/servants.json`，其中记录 schema、数据源、区服、上游 hash/时间戳、同步时间和来源 URL。相同 hash 再次更新只检查版本，不重复下载索引；新版本先完整校验，再以独立目录原子发布，因此失败不会覆盖上一版。运行端可通过 `ServantCatalog::load_latest` 读取指定区服的最新有效快照。

默认命令只同步约 0.5 MB 的 `basic_servant` 索引，包含名称、职阶、稀有度、基础属性、满级 ATK/HP、traits 和头像 URL。传入 `--profile` 后，会读取 Profile 的 `servants[].atlas_id` 和 `support_candidates[].atlas_id`，只请求这些从者的 nice 详情，并将卡组、宝具版本/颜色、主动技能各版本及指令卡素材 URL 压缩保存到同一版本目录的 `details-v3/<Atlas ID>.json`。同步器还会下载所引用灵基阶段的 256×256 指令卡立绘，在内存中校验并裁成 98×47 透明识别模板；只保留小模板，不保存原图，也不拉取完整 `nice_servant` 导出。详情、模板与索引使用同一 Atlas hash，同步中版本发生变化时不发布混合数据，再次执行会复用已经校验的完整缓存。 schema 3 保存技能 ID、槽位、名称、优先级、强化状态、解锁任务/阶段/等级/灵基条件及 10 个技能等级的冷却值，保留同槽位的多个版本。运行时优先读取 `details-v3`，缺少时兼容旧 `details` 下的 schema 2；显式同步写入新目录，不改写旧快照。损坏的新缓存会报错，不静默回退到旧版本。

`sync-data` 是显式联网更新命令，`run` 不会隐式更新或访问 Atlas。生成的 `data/atlas/` 已排除在 Git 之外。运行端按 Profile 的固定成员及全部助战候选，选择同时包含这些从者详情的最新本地快照；整队共用同一个区服、hash 和时间戳，再校验详情和模板。只同步其他队伍或索引不会使已有完整队伍缓存失效；没有整队快照时提示先执行 `sync-data --profile`，不会跨版本拼接。选中快照中的损坏数据仍会报错，不静默回退。非 CN 数据可同时向 `sync-data`、`run` 或带 Profile 的 `inspect --battle` 传入相同的 `--region` 与 `--data-dir`。

## 骑阶冠位战连续出击

准备队伍：前排第 1 位伊斯坎达尔、第 2 位太公望、第 3 位好友助战，后排第 5 位莱妮丝；御主礼装提供原方案的强化、眩晕和换人技能。默认关闭游戏的技能使用确认；若需启用自动确认，见下文 `confirm_skill_use` 配置。
从骑阶冠位战 VII 关卡列表、助战列表、编队、该关卡连续出击提示，或尚未使用技能的首回合启动：

```sh
cargo run --locked -- run scripts/rider_farm.rhai --profile profiles/rider-v1.json --serial 127.0.0.1:5559 --execute --battles 3 --apples 0 --max-seconds 1800 --trace logs/farm.jsonl --frames logs/farm-frames
```

`--profile` 默认为 `profiles/rider-v1.json`。Profile 负责队伍标记、助战前排位置、开局必须未使用的技能、换人位置和敌方目标策略；`enemy_target` 可设为 `current` 或 `highest_hp`，默认 Rider Profile 使用后者。Rhai 继续负责技能与攻击顺序。启动时会拒绝未知 schema、越界 slot 或引用了清单中不存在模板的 Profile，因此自定义阵容还需同时提供对应角色模板。

Profile schema 1 支持可选 `skill_rules`，用于配置从者技能调度。每项指定原始队伍身份、技能和友方目标，数组顺序即施放优先级，例如：

```json
"skill_rules": [
  { "servant": "five", "skill": "one", "target": "two" },
  { "servant": "two", "skill": "three", "target": "three",
    "earliest": { "wave": 2, "turn": 1 } }
]
```

在 Rhai 每回合攻击前调用 `cast_profile_skills()`，返回本次成功施放数。成员按 BattleTracker 映射到当前前排；不在前排的施法者跳过。`earliest` 是最早波次/波内回合，按二元组比较，之后的回合仍可施放；和 FGO-py 一样，配置波次高于本关总波次时折算到最后一波。省略条件表示每回合检查，冷却或禁用技能跳过；条件所需波次未知、符合条件的友方目标不在前排或技能状态未知时停止。每次调用按顺序检查每项一次，实际施放仍要求确认进入冷却，不重复点击来重试。

旧 Profile 缺少 `skill_rules` 时该调用返回 0，原 `cast_skill` API 及 Rider 脚本行为保持兼容。规则可以用于主动换人后的成员，但中途附着战斗不能恢复历史队伍身份，波内回合也是从本次运行观察到的回合开始计数。Atlas 技能版本引用已接入；NP、HP 上限条件见下文，账号状态自动采集尚未接入。

从者和御主规则都可添加可选的 `np_below`，例如为原始队伍第三位充能：

```json
{ "servant": "five", "skill": "three", "target": "three",
  "np_below": { "servant": "three", "percent": 100 } }
```

`np_below.servant` 指定检查 NP 的原始队伍成员，可以与施法者或技能目标不同。`percent` 为 1～300，使用严格小于：上例仅在该成员当前 NP 为 0～99% 时施放，100% 起跳过。每个可用技能在施放前读取当前画面，前一个充能技能完成后不会复用旧 NP；敌方技能切换目标后也再次检查。换人后按当前前排身份映射取值，所需成员不在前排、身份不明或 NP 不可读均停止该技能执行。冷却/禁用技能先跳过，不要求额外的 NP 判断。

NP 数字识别覆盖 CN 1280×720 技能界面固定字体，支持 0～300%，要求数字与百分号完整识别，并记录 ROI、分数和未知状态。Profile 省略 `np_below` 时不为该条件额外请求 NP，脚本选卡沿用宝具可用性检测；GUI 固定模式还要求攻击前确认 NP 为 100～300%。适用边界见 [验证](docs/validation.md)。

从者和御主规则还支持绝对 HP 条件 `hp_below`，例如：

```json
{ "servant": "five", "skill": "one", "target": "three",
  "hp_below": { "servant": "three", "hp": 6600 } }
```

这里的 6600 是当前 HP 数值：低于 6600 才施放，等于或高于时跳过，不是最大 HP 的百分比。阈值允许 1～999999；规则不推测技能是否治疗，也不从 Atlas 基础属性推测账号最大 HP。`servant` 同样按原始队伍身份解析，每次可用技能施放前及选敌后重新读取，因此前一个治疗技能结束后会根据新的 HP 判断。必要 HP 未知或成员无法定位时停止。

从者和御主规则还可设置 `"enemy_charge_full": true`，仅在至少一名可见敌人的充能槽已确认全满时施放。例如防御技能可以写成：

```json
{ "servant": "one", "skill": "three", "target": "one",
  "enemy_charge_full": true }
```

该条件复用现有普通三敌和六敌布局的充能 observation；已确认所有敌人均未满时跳过，敌人布局为空或无法确认必要充能槽时停止，不把未知当作未满。它只判断 UI 充能槽，不识别宝具封印、眩晕、break 后行动规则或技能实际效果。
友方目标还支持按当前前排动态选择：`"target": { "lowest_np": { "below": 100 } }` 或 `"target": { "lowest_hp": { "below": 6600 } }`。每次技能施放前读取所有当前在场成员，选择 NP/HP 最低者（并列时取前排靠前者）；最低值未低于严格阈值时跳过。任一在场成员的必要数值未知，或没有可确认的前排成员时停止，不会随意选择目标。普通 `"target": "one"`、`"target": {"enemy":"three"}` 及旧 Profile 行为不变。

可同时配置 `earliest`、`np_below`、`hp_below` 和 `enemy_charge_full`，只有所有条件满足才施放；已知不满足的条件会直接跳过，不强求其余数值。HP 目前支持 CN 技能界面固定字体，独立匹配字形并检查千位逗号和数字布局；未配置时不新增 HP 请求。HP 条件已有实机阈值验证，治疗后跳过仍由离线测试覆盖。敌方满充能条件有标准/六敌实图与模拟执行回归，尚未进行实机满充能施放验收。

复杂分支可用 `any_of` 声明多个候选条件；每个候选内部为 AND，候选之间为 OR。例如 `"any_of": [{"enemy_charge_full": true}, {"hp_below": {"servant": "one", "hp": 3300}}]` 对应“敌方任一满充能或施法者 HP 低于 3300”。所有候选所需的 NP/HP/敌方事实都会按需读取；任一必要事实未知时安全停止，不把未知当成满足。

战斗默认使用 2 倍速（Profile 的 `"battle_speed": "double"`，省略时同样生效）。选卡前读取右上角速度图标：已是双箭头时保持；连续两帧确认等速才点击一次，并等待连续两帧确认倍速后选卡。当前图标未知、等速未稳定或切换超时会停止，避免盲目切换。可设 `"battle_speed": "unchanged"` 保留游戏当前速度并跳过速度识别。目前支持 CN 选卡界面；宝具演出的独立等速设置仍由游戏配置决定，详见[官方说明](https://faq.fate-go.jp/faq/show/418?site_domain=default)。

Profile 可设置 `"confirm_skill_use": true`，允许从者、普通御主及换人技能处理 CN“技能使用”确认弹窗。默认 `false`；此选项只授权执行器确认已发起的技能，不修改游戏设置。

只有“技能使用”标题、“取消”“决定”按钮及对应关闭叉号同时匹配，且弹窗出现在本次技能点击之后，才点击一次“决定”。弹窗短暂未消失时等待，不连点；退出后再次出现、已进入友方选择后出现、超时或不完整弹窗均停止。启动时已有的确认窗也不会自动通过。点击确认后仍要完成原有目标选择、冷却或换人结果验证。自定义模板清单启用此选项时必须包含确认锚点，缺少会在启动前报错。

Profile 的 `confirm_skill_use` 默认为关闭；游戏中开启技能确认时，需要同步启用该选项。GUI 固定模式自动处理完整识别的本次技能确认窗。

从者规则可额外指定 `atlas_skill_id`，例如已在 `servants` 将 `one` 绑定为伊斯坎达尔（400200）时，可使用 `{"servant":"one","skill":"one","target":"three","atlas_skill_id":5550}`。启动前会从本地 schema 3 详情校验该 ID 属于此从者且位于第一技能槽；缺少新缓存时提示执行 `sync-data --profile`，不会在战斗时联网。此字段目前只支持绑定固定 Atlas 从者 ID 的队伍槽位，动态助战仍可使用不带此字段的规则。

默认 Rider Profile 的助战候选为骑阶阿尔托莉雅〔Alter〕（402700，Quick 宝具）；运行前执行 `sync-data --profile profiles/rider-v1.json` 补齐缓存。

动态助战可在每个 `support_candidates` 候选中声明 `skill_versions`，例如本地 CN Atlas 缓存中的摩根：

```json
{ "atlas_id": 704000,
  "skill_versions": [
    { "skill": "one", "atlas_skill_id": 879450 },
    { "skill": "two", "atlas_skill_id": 762350 },
    { "skill": "three", "atlas_skill_id": 880550 }
  ] }
```

候选可以使用不同的技能 ID，但必须声明相同的槽位集合；重复槽位、零 ID 或候选遗漏会在 Profile 校验时拒绝。启动时逐候选校验本地 schema 3 详情中的从者 ID、技能槽位和版本 ID，缺缓存时先执行 `sync-data --profile`。省略 `skill_versions` 保持原行为，也不要求升级旧 schema 2 缓存。声明不会改变规则顺序、选择技能版本或证明选中的好友身份及解锁状态；动态助战身份仍由后续卡面识别产生，实际技能继续按画面可用性和冷却验收。

版本引用仅检查元数据一致性，不证明账号已经完成强化任务，也不会自动选择最高优先级版本或利用冷却表推算可用状态。实际施放继续以截图识别和冷却确认作为执行依据；省略 `atlas_skill_id` 的旧规则保持原行为。

如需校验所引用技能的解锁条件，可在对应 `servants` 条目中提供可选的 `account_state`。以下只是配置示例，不代表当前游戏账号的实际状态：

```json
{
  "slot": "one", "atlas_id": 400200,
  "account_state": {
    "level": 90, "ascension": 4,
    "quest_phases": { "94099428": 3 }
  }
}
```

同时在该成员的技能规则中指定 `atlas_skill_id`。`level` 是从者等级，`ascension` 为 0～4，`quest_phases` 记录已完成的任务阶段，显式的 0 表示未完成；未填写的等级、灵基或任务均视为未知。启动时对照本地 Atlas 的等级、灵基、任务阶段条件，只有满足才通过，未满足或必要事实未知都会拒绝启动。Atlas 指定任务但阶段为 0 时仍保持未知，不猜测任务已完成。无对应前置条件的字段不必填写。

这是对用户声明的静态条件校验，不会读取账号、自动完成强化任务、选择技能版本或判断复杂技能脚本条件。省略整个 `account_state` 时继续只校验技能 ID/槽位；提供它却没有对应的 Atlas 技能引用时会报错，避免配置被静默忽略。

普通御主技能使用独立的可选 `master_skill_rules`，目标仍按原始队伍身份解析，时机和数组优先顺序与从者规则相同：

```json
"master_skill_rules": [
  { "skill": "two", "target": "three", "earliest": { "wave": 2, "turn": 1 } },
  { "skill": "one", "target": "two" }
]
```

Rhai 调用 `cast_profile_master_skills()` 返回成功施放数；缺少规则时返回 0。例如每回合先 `cast_profile_skills();`，再 `cast_profile_master_skills();`，两组之间的先后顺序由脚本决定。御主技能施放后必须重新打开面板确认冷却，并关闭面板；不以估算的冷却回合数代替画面验证。目标缺失或所需波次未知时在施放前停止。schema 校验拒绝重复御主技能以及被 `order_change.master_skill` 占用的技能槽位。旧 `cast_master_skill` API 保持默认助战目标。

不使用换人的御主礼装可省略 `order_change` 或设为 `null`，此时 `master_skill_rules` 可配置 `"skill": "three"`，直接调用 `cast_master_skill(3, strict)` 也可用。普通技能仍须确认冷却，并按需处理友方目标弹窗。存在 `order_change` 时，其指定槽位只能由 `order_change()` 调用；缺少配置时调用换人会在输入前报错，启动校验也不会要求换人前后的角色标记。

这一配置不会自动识别或切换御主礼装，须与实机装备一致。若普通技能意外打开换人弹窗，程序会停止，不选择成员。第三技能已覆盖模拟状态和既有截图编排的执行链路；其他礼装的实机画面及特殊多选弹窗尚未验收。

对施加于当前选中敌人的普通御主技能，可在规则中使用 `"target": {"enemy": "three"}`，或调用 `cast_master_skill_on_enemy(2, 3, strict)`。数字 3 是敌方布局槽位，不是从者队伍身份；普通布局只支持 1～3，六敌布局支持 1～6。

执行器先检查技能可用性，再关闭御主面板，确认目标在场并选中它；重新打开面板后再次确认选中标记、目标在场及技能可用，才提交施放。目标切换失败或证据不足时停止。敌方技能若弹出友方选择窗口会报错，不点击友方目标。该选敌会保留为游戏当前目标，后续攻击仍按 Profile 的 `enemy_target` 策略处理。旧 `"target": "three"` 仍表示原始第三队伍成员。

从者技能现支持相同的 `"target": {"enemy":"three"}` 对象，或直接调用 `cast_skill_on_enemy(servant_slot, skill, enemy_slot, strict)`；直接 API 的 `servant_slot` 是当前前排 1～3，Profile 的 `servant` 仍按原始队伍身份解析。敌方槽位使用已有普通/六敌布局，施放前核对选中、目标在场、助战标记和技能可用，之后要求进入冷却；未知状态及意外友方弹窗会停止，冷却中的技能不会触发选敌。旧友方字符串目标和 `cast_skill` 行为保持兼容。

这项配置不识别技能效果类型；须由使用者保证与所装备技能一致。支持的是施加于当前选中敌人的技能，特殊敌方选择弹窗、多选及效果命中识别尚未支持，实际技能效果也未由冷却变化证明。

从者敌方技能当前主要由模拟执行和截图编排覆盖，尚未验收真实敌方效果；具体实机范围见 [验证](docs/validation.md)。

技能调度、换人、条件与目标的回归和实机覆盖见 [验证](docs/validation.md)。

`--battles` 默认为 1；设为 0 时持续运行，直到 AP 与苹果额度不足、超时或发生错误。`--apples` 默认为 0，AP 不足时关闭恢复窗口并停止。苹果额度按确认页的实际消耗个数计算，确认消费前扣减运行内额度；不确定的操作结果会报错，不重试消费。额度不跨进程保存。
`--apple-kind` 接受 `gold`（黄金）、`silver`（白银）、`bronze`（赤铜）。白银和赤铜先将数量滑块设为游戏允许的最小值，识别确认页数量；最低批次超出额度时取消并停止。当前识别 1～4 个的批次，未知数量报错。蓝色青铜果实的确认页尚无实机样本，暂不开放。
实际消费苹果后的自动续战尚未完成实机验收，详见 [验证](docs/validation.md)。

每场完成结算后才计入次数，到达上限时保留在连续出击提示或关卡列表。助战每轮最多翻 25 页、最多刷新 10 次，刷新间隔至少 10 秒；找不到符合条件的好友会停止。
游戏自身设置的道具自动使用仍由游戏控制，`--apples` 只限制 AP 恢复。

单独执行战斗用 `scripts/rider_battle.rhai`，到结果页结束。单独选助战用 `scripts/select_rider_support.rhai`，确认编队后结束；游戏的连续出击模式可能在选完助战后直接进入战斗。

没有 `--execute` 时输入只写日志，依赖画面变化的流程可能超时。`--replay` 与 `--execute` 互斥。
执行期间输入 `pause`、`resume`、`stop` 后回车，或按 Ctrl-C。总时限包含暂停时间，已发出的设备输入不能撤销。

## 日志与回放

默认日志为 `logs/actions.jsonl`，追加写入。`--frames` 要求一个尚不存在的目录，按 `000001.png` 等名称保存每次识别所用的归一化画面，日志记录相应路径。
每条日志都包含 run ID、事件序号、运行耗时和关联 frame ID；observation schema 2 保存当前完整 facts、未知字段及识别证据；已验证的战斗/周回意图及场景变化分别记录为 `command` 和 `scene`，脚本或识别失败记录为 `error`。核心通过可替换的 `EventSink` 输出结构化事件，默认实现同时写 stderr 和 JSONL，GUI 直接订阅事件。
失败时另存最后观察到的画面，可能早于报错时刻。回放按文件名末尾数字排序，每次截图消耗一帧，耗尽即报错；独立参考截图不等于完整交互录像。

只有 PNG 的 `--replay` 使用模拟时钟运行，脚本等待会推进逻辑时间而不实际阻塞，但缺少原始 capture 时间，不能复现设备帧间隔。录制时同时保存 trace 和 frames 后，可用原 trace 最后一个 run 的 capture 时间戳重放，并比较 observation 或完整语义事件流：

```sh
cargo run --locked -- run scripts/rider_farm.rhai \
  --replay logs/farm-frames \
  --replay-trace logs/farm.jsonl \
  --replay-compare events \
  --trace logs/farm-replay.jsonl \
  --replay-report logs/farm-replay-report.json
```

`--replay-trace` 要求 PNG 数量与基线 capture 数量一致，并且输出 `--trace` 不能覆盖基线。`observations` 模式只比较逐帧识别结果；`events` 还比较场景、决策、类型化命令、归一化输入位置、验证状态和重试次数。比较会忽略 run ID、墙钟绝对值、录制文件路径、原始设备坐标及 replay 固定为不执行输入的差异。任何语义差异都会返回非零结果，报告最多展开前 20 项，同时保留总差异数。

## 脚本 API

| 函数 | 行为 |
|---|---|
| `capture()` | 更新缓存画面 |
| `find(name)` / `matches(name)` | 从缓存识别模板；find 返回含 found 的 map |
| `find("support120")` | 联合识别 120/120 等级和好友标志 |
| `tap(x,y)` / `swipe(x1,y1,x2,y2,ms)` | 归一化坐标输入 |
| `sleep(ms)` / `elapsed_ms()` | 可中断等待 / 本次运行时间 |
| `log(message)` / `fail(message)` | 记录日志 / 抛出脚本错误 |
| `enter_battle()` | 选关、处理 AP、选助战并等待战斗；达到限额返回 false |
| `select_support()` | 筛选、刷新并验证好友选择结果 |
| `finish_battle()` | 推进已识别的结算页面并计数 |
| `farm_progress()` | 返回 completed、apples_remaining |
| `verify_opening()` / `verify_support()` | 确认完整开局 / 第 3 位好友仍在场 |
| `cast_skill(slot,skill,strict)` | 1 起始索引；选目标并验证冷却；strict=false 可跳过冷却技能 |
| `cast_master_skill(skill,strict)` | 使用御主技能 1/2 并确认冷却 |
| `order_change()` | 1/5 换人并确认莱妮丝上场 |
| `attack(first)` | 首轮要求三宝具；按 Profile 的宝具身份顺序及普通卡评分选择三张 |
| `wait_next_turn()` | 下一回合返回 true，胜利结果返回 false，失败或超时报错 |
| `battle_progress()` | 返回本次运行观察到的回合数、波次/波次回合、原始队伍槽位对应的当前前排及战斗结果 |

低层识别和输入前需要 `capture()`；高层战斗及连续出击 API 自行截图和等待。宝具就绪仅从指令卡画面间隔至少 150ms 的两帧判断。
`battle_progress()` 的 `frontline` 使用 1～6 表示成员最初所在的队伍槽位，0 表示空位；换人成功后才更新。CN 战斗 UI 的 1～3 波数字已经接入，当前值与总数必须同时识别且满足 `current <= total` 才会更新；现有真实截图覆盖 `1/1`、`1/3` 和 `3/3`，其余组合由合成测试覆盖。前排头像指纹和助战位置会在攻击前后的可操作画面间比较，从而更新自动补位；主动换人会刷新比较基线。该路径已经覆盖合成退场状态和真实换人截图差异，但尚无真实从者退场截图，不能视为实机退场验证。
Party Profile 的 `servants` 使用原始队伍槽位与 Atlas ID 标识已知成员；`support_candidates` 列出本阵容允许选择的动态助战，当前 Rider Profile 包含骑阶阿尔托莉雅〔Alter〕。普通助战卡先通过画面上的助战标记归属到原始槽位，再在候选模板间识别具体 Atlas ID；候选结果相互冲突或过于接近时保持未知。`cards.noble_phantasm_order` 按成员最初的 1～6 号队伍槽位配置，因此主动换人和自动补位不会改变宝具身份优先级；旧 Profile 缺少 `cards`、`servants` 或 `support_candidates` 时仍可加载，并继续使用安全的兼容行为。`face_card_priority` 可调整助战卡数量、Brave Chain、同色 Chain 和伤害的字典序，颜色分值、出卡位置倍率及首红卡加成也在同一配置中。已确认候选助战的宝具颜色会参与完整三卡 Chain；当前指令卡页没有出现可识别的助战普通卡时仍保持未知，不臆测同色 Chain。若助战角色固定，也可以直接为其原始槽位配置 Atlas ID。
`check` 会在未连接设备前检查 Rhai 语法，以及直接函数调用的名称和参数个数；宿主 API、脚本自定义函数和 Rhai 标准函数均纳入校验。动态 `eval`/`Fn`/`call` 不开放，以免绕过启动前检查；参数类型仍由 Rhai 在运行时检查。脚本模块导入关闭，未注册任意 shell 或文件访问 API。

## 识别与迁移范围

默认从横屏中央裁成 16:9；游戏区域不居中时，用原始坐标传入 `--viewport x,y,width,height`。不自动处理旋转、窗口边框或导航栏。
模板在 `assets/templates.json` 中配置，坐标为 1280×720，路径相对该清单。资源目前适配中文界面，Catmull-Rom 缩放与旧版 OpenCV 不逐像素等价。
当前架构见 [架构说明](docs/architecture.md)，识别职责见 [识别后端](docs/recognition.md)，实机证据与尚未覆盖的范围见 [验证](docs/validation.md)。

代码主要分为 `device`/`adb` 设备层、`catalog` 本地数据层、`vision`/`battle_vision`/`farm_vision` 识别层、`observation` 场景归一层、`executor` 执行边界、`battle`/`farming` 流程层，以及 `runtime` Rhai 宿主。
许可和素材来源见 [NOTICE.md](NOTICE.md) 与 [LICENSE](LICENSE)。
