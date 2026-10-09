# GUI 使用

桌面入口为 `fgo-rs-gui`，使用 Slint 软件渲染器，CLI 与 GUI 共用 Runner、识别和执行器。

```sh
# Rust 后端
cargo run --locked --features gui --bin fgo-rs-gui

# 同时启用 Maa
cargo run --locked --features gui,maa --bin fgo-rs-gui -- --adb /path/to/adb

# 发布构建
cargo build --locked --release --features gui,maa --bin fgo-rs-gui
```

本机开发 Bundle 可通过 `open .tools/FGO-rs-dev.app` 启动，其资源目录指向源码仓库；它不是可分发安装包。ADB 需单独提供，不在 PATH 时传入 `--adb` 或在路径设置中选择。

## 固定位置刷图

默认固定位置模式、只读演练、1 场、0 苹果。实际发送点击或滑动前选择“执行输入”。

1. 在游戏中打开关卡列表，在 GUI 点击“在截图上点选关卡”，再点预览中的关卡；也可填写归一化 X / Y，默认 `(845, 203)`。
2. 编队序号从左至右 1～15，0 保持当前队伍。非零编队需要新鲜画面确认所选标签；受限编队没有可识别标签时使用 0。
3. 设置场次、苹果额度和最长运行时间，选择设备后开始。助战取当前列表第一项，不限制身份、职阶或等级；没有可用项时有界刷新。

点选只设置坐标。预览和运行使用同一套 1280×720 归一化与设备坐标映射，窗口缩放、留边不改变位置。

此模式适配 CN，无需 Profile、Rhai 或 Atlas。每回合按当前前排 1～3 位、各技能 1～3 的顺序，逐个重新识别并施放可用技能；冷却、禁用和未知技能跳过，未知记入日志。友方目标默认选择施法者；完整识别本次施放产生的确认弹窗后只确认一次，最终验证冷却。

攻击前读取 NP，只有确认 100～300% 且选卡页可用的宝具才参与选卡。空位、未知或低 NP 不作为宝具，其余卡沿用默认评分与连携。默认确认倍速，御主技能和换人关闭。该策略不推断充能、治疗或增伤效果，无法保证任意队伍的伤害或回合数。

支持从关卡、助战、编队、连续出击提示或战斗回合开始。结算后按预算继续；未知场景、卡色、编队或无法验证的技能结果会停止。场次只在完整结算后计入，达到上限停留在连续出击提示或列表。

## Profile 与脚本

切换到“Profile / Rhai 脚本”使用冠位战策略。显式传入 `--profile` 或 `--script` 时默认脚本模式，外部文件会加入选项。

```sh
cargo run --locked -- sync-data --profile profiles/saber-v1.json
cargo run --locked --features gui,maa --bin fgo-rs-gui -- \
  --resources . --profile profiles/saber-v1.json --script scripts/saber_farm.rhai

# 离线输入始终禁用
cargo run --locked --features gui --bin fgo-rs-gui -- \
  --resources . --replay tests/fixtures/step2.png \
  --profile profiles/rider-v1.json --script scripts/inspect_rider.rhai
```

脚本模式使用对应的离线 Atlas 快照，路径设置可指定 `data/atlas` 根目录；更新仍通过 CLI 的 `sync-data` 完成。SDK 和 OCR 路径见 [识别后端](recognition.md)。

## 控制、资源与日志

空闲时约每秒 2 次预览设备，运行后显示核心识别所用帧。暂停计入预算，停止和关闭窗口取消后台任务。界面保留最新 300 条日志，完整 JSONL、录制帧及失败截图写入每次运行的独立目录。

资源根目录包含 `assets/`、`profiles/`、`scripts/`，可用 `--resources` 指定。自动定位检查可执行文件旁的 `resources/`、Bundle 的 `Contents/Resources/` 和当前目录；开发构建可回退源码目录。资源目录无需可写。

`--output` 指定日志根目录。默认 macOS 为 `~/Library/Application Support/FGO-rs/logs`，Windows 为 `%LOCALAPPDATA%\FGO-rs\logs`。

只读演练记录计划输入并跳过点击，仍等待真实画面变化，因此完整进本可能超时。它适合检查配置和识别；完整离线执行使用录制帧回放。

macOS ARM64 已完成窗口控制与实机战斗验收；Windows、Intel macOS 尚未完成 GUI 运行验收。设置持久化、文件选择对话框、GUI 数据同步及可分发安装包尚未实现。已有证据见 [验证](validation.md)。
