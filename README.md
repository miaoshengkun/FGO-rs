# FGO-rs

Rust 核心 + Rhai 策略脚本的 FGO 自动化项目，目标为 Windows 和 macOS。目前迁移范围是中文界面的骑阶冠位战 VII，尚未完整替代 FGO-py。

## 当前功能

- ADB 连接、截图、点击和滑动；横屏归一化到 1280×720 并映射输入坐标。
- 纯 Rust 模板识别、PNG 回放、JSONL 动作日志、逐帧录制及失败截图。
- 骑阶 120/120 级好友筛选、翻页、最多 10 次刷新、编队第 3 位等级确认。
- 从关卡列表、助战列表、编队或连续出击提示进入战斗。
- 技能冷却确认、我方目标选择、1/5 换人、御主技能、首轮三宝具及后续回合选卡。
- 牵绊、经验、战利品结算及连续出击；战斗次数、苹果额度和总运行时间限制。
- 暂停、继续、停止、Ctrl-C 和 ADB 子进程超时处理。

运行不依赖 Python、Qt、OpenCV 或 ONNX Runtime。实机输入需要单独提供 ADB。
当前沿用游戏已选中的敌人，尚未迁移最高 HP 选敌、通用战斗、OCR 和 GUI。macOS 的 CI 已配置，但尚未实际运行验证。

## 构建

```sh
cargo build --locked --release
cargo test --locked
```

Windows MSVC 需要 Visual Studio C++ Build Tools；macOS 需要 Xcode Command Line Tools。本机使用 Windows GNU 工具链，操作见 [本机构建说明](docs/local-windows.md)。
编译结果位于 `target/release/fgo-rs`，Windows 为 `.exe`。以下命令从项目根目录运行。

## 连接与检查

```sh
cargo run --locked -- devices
cargo run --locked -- connect 127.0.0.1:5559
cargo run --locked -- screenshot --serial 127.0.0.1:5559 --output logs/screen.png
cargo run --locked -- inspect tests/fixtures/live-formation.png --farm
cargo run --locked -- inspect tests/fixtures/step12.png --battle
cargo run --locked -- run scripts/inspect_rider.rhai --replay tests/fixtures/step2.png
```

ADB 不在 PATH 时传入 `--adb /path/to/adb`。设备序列号必须显式指定。macOS 上的游戏载体同样需要是支持 ADB 的 Android 设备或模拟器。
`--farm` 输出关卡、助战、AP 和结算状态；`--battle` 输出技能、换人、目标、指令卡等状态。

## 骑阶冠位战连续出击

准备队伍：前排第 1 位伊斯坎达尔、第 2 位太公望、第 3 位好友助战，后排第 5 位莱妮丝；御主礼装提供原方案的强化、眩晕和换人技能。关闭游戏的技能使用确认。
从骑阶冠位战 VII 关卡列表、助战列表、编队、该关卡连续出击提示，或尚未使用技能的首回合启动：

```sh
cargo run --locked -- run scripts/rider_farm.rhai --serial 127.0.0.1:5559 --execute --battles 3 --apples 0 --max-seconds 1800 --trace logs/farm.jsonl --frames logs/farm-frames
```

`--battles` 默认为 1；设为 0 时持续运行，直到 AP 与苹果额度不足、超时或发生错误。`--apples` 默认为 0，AP 不足时关闭恢复窗口并停止。苹果额度按确认页的实际消耗个数计算，确认消费前扣减运行内额度；不确定的操作结果会报错，不重试消费。额度不跨进程保存。
`--apple-kind` 接受 `gold`（黄金）、`silver`（白银）、`bronze`（赤铜）。白银和赤铜先将数量滑块设为游戏允许的最小值，识别确认页数量；最低批次超出额度时取消并停止。当前识别 1～4 个的批次，未知数量报错。蓝色青铜果实的确认页尚无实机样本，暂不开放。
本次实际验证了不吃苹果连续出击，以及苹果种类、数量识别和额度不足时取消；未实际消费苹果验证恢复后的自动续战，详见 [验证记录](docs/validation.md)。

每场完成结算后才计入次数，到达上限时保留在连续出击提示或关卡列表。助战每轮最多翻 25 页、最多刷新 10 次，刷新间隔至少 10 秒；找不到符合条件的好友会停止。
游戏自身设置的道具自动使用仍由游戏控制，`--apples` 只限制 AP 恢复。

单独执行战斗用 `scripts/rider_battle.rhai`，到结果页结束。单独选助战用 `scripts/select_rider_support.rhai`，确认编队后结束；游戏的连续出击模式可能在选完助战后直接进入战斗。

没有 `--execute` 时输入只写日志，依赖画面变化的流程可能超时。`--replay` 与 `--execute` 互斥。
执行期间输入 `pause`、`resume`、`stop` 后回车，或按 Ctrl-C。总时限包含暂停时间，已发出的设备输入不能撤销。

## 日志与回放

默认日志为 `logs/actions.jsonl`，追加写入。`--frames` 要求一个尚不存在的目录，按 `000001.png` 等名称保存每次识别所用的归一化画面，日志记录相应路径。
失败时另存最后观察到的画面，可能早于报错时刻。回放按文件名末尾数字排序，每次截图消耗一帧，耗尽即报错；独立参考截图不等于完整交互录像。

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
| `attack(first)` | 首轮要求三宝具，后续按宝具与助战卡优先选三张 |
| `wait_next_turn()` | 下一回合返回 true，胜利结果返回 false，失败或超时报错 |

低层识别和输入前需要 `capture()`；高层战斗及连续出击 API 自行截图和等待。宝具就绪仅从指令卡画面间隔至少 150ms 的两帧判断。
`check` 只检查 Rhai 语法，不验证宿主函数。脚本模块导入关闭，未注册任意 shell 或文件访问 API。

## 识别与迁移范围

默认从横屏中央裁成 16:9；游戏区域不居中时，用原始坐标传入 `--viewport x,y,width,height`。不自动处理旋转、窗口边框或导航栏。
模板在 `assets/templates.json` 中配置，坐标为 1280×720，路径相对该清单。资源目前适配中文界面，Catmull-Rom 缩放与旧版 OpenCV 不逐像素等价。
实机运行和回归结果见 [验证记录](docs/validation.md)。下一阶段可继续迁移最高 HP 选敌、通用战斗及 GUI，并在 macOS 实机验证。

代码主要分为 `device`/`adb` 设备层、`vision`/`battle_vision`/`farm_vision` 识别层、`battle`/`farming` 流程层，以及 `runtime` Rhai 宿主。
许可和素材来源见 [NOTICE.md](NOTICE.md) 与 [LICENSE](LICENSE)。
