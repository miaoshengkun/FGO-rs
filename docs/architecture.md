# 当前架构

FGO-rs 使用 Rust 核心、Rhai 策略和可选 Slint GUI。默认识别适配 CN 横屏界面；骑阶、剑阶冠位战 VII 使用专用 Profile，GUI 另提供无需脚本的固定位置刷图。实机边界见 [验证](validation.md)。

## 运行链路

```text
CLI / Slint GUI
       │ RunConfig、RunHooks
       ▼
    Runner ── 资源、Profile、离线 Atlas 缓存校验
       │
       ▼
    Runtime / Rhai / 默认战斗
       ├── BattleDriver / FarmDriver ── 类型化命令与结果验证
       ├── Vision ── 场景、事实、识别证据
       └── Device ── FrameSource / InputSink ── ADB / 回放
```

`src/runner.rs` 统一 CLI 和 GUI 的启动校验与运行对象。`gui`、`maa` 是独立 Cargo feature，默认 CLI 构建不加载 Slint 或原生 Maa 库。

| 层 | 文件 | 职责 |
| --- | --- | --- |
| 设备与控制 | `device.rs`、`adb.rs`、`control.rs` | 截图、输入、子进程超时、暂停与取消 |
| 识别 | `vision.rs`、`battle_vision.rs`、`farm_vision.rs`、各 OCR 模块 | 图像归一化、模板与数字识别、原始证据 |
| 可选原生识别 | `maa.rs` | 原生模板匹配、识别资源及卡色 OCR 补充 |
| 数据与配置 | `catalog.rs`、`profile.rs` | 本地 Atlas 快照、队伍与技能规则校验 |
| 决策与执行 | `observation.rs`、`domain.rs`、`executor.rs`、`verification.rs` | 当前事实、类型化命令、输入前置条件与结果验证 |
| 流程 | `battle.rs`、`farming.rs`、`runtime.rs` | 战斗、周回、Rhai API 与默认技能策略 |
| 回放 | `replay.rs` | 模拟时钟、录制帧与语义事件比较 |
| 界面 | `src/bin/gui.rs`、`src/gui/`、`ui/app.slint` | 设备预览、运行配置、控制及日志展示 |

## 识别与执行约束

- 截图默认从横屏中央裁为 16:9，归一化到 1280×720；输入映射回设备坐标。CLI 可指定 `--viewport`。
- `assets/templates.json` 声明 ROI、阈值及渲染变体。相同路径共享解码结果，同次 observation 内复用识别结果；新截图不复用旧事实。
- 场景检查互斥冲突。必要身份或数值未知时重拍或停止，未知数值不转换为零。
- 技能提交后验证冷却，换人后验证队伍，编队切换后验证所选标签。已提交技能或道具消费不通过重复点击重试。
- BattleTracker 将原始队伍 1～6 号身份映射到当前前排；主动换人及稳定头像证据更新映射，过渡帧不能直接视为退场。
- Maa 只承担识别，不执行资源节点的 `action`、`next`，后端失败向上报错。

固定位置模式每回合逐个读取并使用可用从者技能，友方目标默认为施法者；宝具要求攻击前 NP 数值与选卡页状态共同确认。它不推断技能效果，也不调用御主技能或换人。Profile / Rhai 模式保留配置的技能条件与顺序，详见 [README](../README.md) 和 [GUI](gui.md)。

## 数据、日志与控制

Atlas 仅由显式 `sync-data` 联网更新，运行时读取完整的同区服、同 hash 队伍快照。新数据先校验再发布；运行不跨版本拼接，也不忽略损坏缓存。固定位置模式不需要 Atlas。

核心通过 `EventSink` 写 JSONL，通过 `FrameSink` 发布归一化像素。事件包含 run ID、序号、运行耗时和 frame ID；录制帧结合 capture 时间戳可复现决策与验证，`serde_json/float_roundtrip` 保留比较所需数值精度。

Slint 主线程处理展示与消息，ADB、识别及运行放在后台。空闲预览与运行截图互斥，界面保存最新帧和 300 条日志，完整事件仍落盘。暂停计入预算；停止、Ctrl-C 和关闭窗口经 `Control` 取消运行。

配置持久化、通用地图导航、完整账号状态采集和安装包尚未实现。资源与许可来源见 [NOTICE](../NOTICE.md)，识别边界见 [识别后端](recognition.md)。
