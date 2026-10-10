# 验证与适用范围

本文记录当前检查入口与有效实机证据。默认识别适配 CN 横屏界面；离线与模拟设备检查不等同于真实设备验收。

## 常规检查

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo test --locked --all-targets --features gui,maa
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features gui,maa -- -D warnings
cargo build --locked --release --features gui,maa --bins
```

2026-10-10 OCR 迁移后的检查结果：默认测试 292 项通过、2 项跳过；GUI / Maa 测试 300 项通过、14 项跳过。实际加载 SDK 与模型的串行识别回归 10 项、稀疏六敌数值识别 1 项、编队识别 1 项全部通过；格式、两种构建的 Clippy 和发布构建通过。GUI 仅保留 Maa 后端后，使用 `--features gui` 重新通过常规测试、Clippy 和 GUI 发布构建，包含库与模型缺失时阻止启动的检查。

原生 Maa 和 OCR 回归需要 SDK 与模型，命令见 [识别后端](recognition.md#原生回归)。常规测试中这部分明确标记 ignored，不能将跳过视为原生通过。ADB 子进程的两个专用测试也默认跳过。

回归保留技能条件与冷却、确认弹窗、换人映射、前排稳定性、HP / NP 完整性、空位宝具拒绝、卡牌归属、编队序号、预算、取消、资源定位和 JSONL 回放。四张 `owner-*.png` 是仍被这些回归引用的真实截图。

## 已有实机证据

| 场景 | 已验证行为 | 本机证据 |
| --- | --- | --- |
| Rider VII Profile | 技能顺序、换人、助战目标、结算；NP / HP 严格阈值和技能确认分别完成实机验证 | `logs/` 中对应录制帧与事件；语义回放检查 |
| GUI Saber VII | Maa 后端两回合胜利并结算，0 苹果；586/586 事件回放一致 | `logs/gui-saber-vii-20261007/` |
| GUI 固定位置 | 点选当前活动关卡、编队 0、首项助战，三波八回合完成结算；0 苹果，728/728 事件回放一致 | `logs/gui-fixed-live-20261007/` |
| 默认从者技能 | 26 次技能均验证冷却；该次运行到 600 秒预算后停止，尚未结束战斗 | `logs/default-skills-fix-20261008/repair-audit.json` |
| 空位宝具修复后续打 | NP 85 / 19 / Unknown 时选择三张普通卡，击败剩余敌人并结算；136/136 事件回放一致 | 同上 |
| Maa 数值迁移后默认战斗 | 用户手动运行，三波六回合胜利并结算，1 场、0 苹果；原始日志包含 815 个事件和 236 张帧，NP 证据来自 Maa OCR | `~/Library/Application Support/FGO-rs/logs/run-1791610670751670000-74425/` |

固定位置完整战斗是在自动技能修复前完成的。自动技能与空位宝具修复的证据来自同一原局的多次续打，其中超时和修复前失败均保留，不能合并成一次从进本到结算的完整成功运行，也不能据此声称伤害或速度改善。

真实截图、标注清单和可移植回归位于 `tests/fixtures/`。上表 `logs/` 为被 Git 忽略的本机录制资料，不随仓库分发。

## 尚未验收或存在限制

- 实际消费苹果后的自动续战尚未完成实机验收；已验证无苹果连续出击、种类/数量识别与额度不足取消。
- 非零编队切换已用模拟 ADB 验证完整执行链，真实标签经 Rust / Maa 检查；固定位置实机使用编队 0。
- 自然退场补位、严重遮挡、任意助战和其他区服不具备完整实机覆盖；过渡帧回归不能视为所有退场情况通过。
- 满充能条件、治疗后重新判断及从者敌方目标效果主要由离线和模拟执行覆盖，尚不能视为完整实机效果验证。
- 配置 Maa OCR 后，敌方 HP / NP 运行识别已使用原生 OCR，并保留格式、NP 空间完整性和敌方占位检查；友方 HP 的原生 OCR 仍只用于诊断。迁移已通过真实截图回归及上表的一次实机默认战斗；其他队伍、关卡、模型或字体需独立检查。
- 只读模式仍等待画面推进，完整进本可能超时；离线回放始终禁止输入。
- GUI 仅在 macOS ARM64 完成运行验收，Windows / Intel macOS、配置持久化与可分发安装包仍未完成。

## 回放比较

```sh
cargo run --locked -- run scripts/saber_farm.rhai \
  --profile profiles/saber-v1.json \
  --replay logs/farm-frames --replay-trace logs/farm.jsonl \
  --replay-compare events --trace logs/replay.jsonl \
  --replay-report logs/replay-report.json
```

帧数必须与 capture 事件一致，输出不能覆盖基线。`events` 比较场景、事实、决策、命令、输入与验证状态，忽略 run ID、墙钟绝对值、路径及设备原始坐标，任何语义差异返回非零。单张参考截图用于识别检查，不能代表完整交互回放。
