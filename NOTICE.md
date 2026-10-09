# 来源与资源说明

本项目从相邻的 FGO-py 项目开始迁移，沿用 AGPL-3.0-only 代码许可；原始许可文本见 LICENSE。

- 上游项目：https://github.com/hgjazhgj/FGO-py
- 原作者声明：Copyright (C) 2019-2026 hgjazhgj。
- `assets/rider/` 来自本地 FGO-py 的 `FGO-py/fgoImage/riderGrand/`。
- `assets/common/` 来自本地 FGO-py 的 `FGO-py/fgoImage/`。
- `assets/cn/` 来自本地 FGO-py 的 `FGO-py/fgoImage/cn/`。
- `assets/templates.json` 的区域与阈值，以及骑阶好友判定规则，参考 `fgoDetect.py`、`fgoRiderGrandBattle.py`。固定位置模式的首项助战坐标、1–15 编队位置及 `assets/common/teamindex.png` 来自 FGO-py 的 `fgoConst.py`、`fgoKernel.py`、`fgoDetect.py` 和 `fgoImage/teamindex.png`。
- `tests/fixtures/step*.png` 来自本地 `temp/RiderGrandBattleScreenshots/`，用于迁移回归。
- `assets/maa/pipeline/battle.json` 的初始 13 个 recognition 节点及 `assets/maa/image/battle/` 下的 4 张上游模板来自 MaaFgo `cbf7eaac` 的 `assets/resource/base/`，按其 AGPL-3.0 许可复用：https://github.com/xlxyvergil/MaaFgo 。NP 节点调整了 ROI、文字格式及分数门槛；另增 3 个参考 FGO-py HP 区域的诊断节点。中文返回按钮变体裁自本项目已有的 `tests/fixtures/step12.png`。
- 可选 `maa-framework` Rust 绑定与 MaaFramework 原生 SDK 按 LGPL-3.0 许可提供：https://github.com/MaaXYZ/maa-framework-rs 和 https://github.com/MaaXYZ/MaaFramework 。当前动态加载 SDK；其依赖库及 OCR 模型仍受各自来源许可约束。
- 可选桌面界面使用 Slint 1.18.1（`slint`、`slint-build` 及其组件），按其 GPL-3.0-only 选项与本项目 AGPL-3.0-only 组合分发；原许可及其他授权选项见 https://github.com/slint-ui/slint 。保留各依赖自身的版权、许可与对应源码要求。
- `tests/fixtures/live-*.png` 为 2026-09-10/11 经用户授权，从本地 MuMu 模拟器通过 ADB 采集并归一化的游戏画面；`assets/live/` 为这些画面裁出的识别模板。
- `tests/fixtures/live-saber-quests-*.png` 为 2026-10-06 模拟器验证时采集的剑阶关卡列表，用于检查 V／VI 与 VII 的区别。
- `tests/fixtures/live-saber-support-iori120.png` 为同次模拟器验证采集的助战列表，用于检查同一行的宫本伊织、120/120 等级及好友标记。
- `tests/fixtures/live-saber-cards*.png` 和 `live-saber-turn*.png` 为同次单场剑阶 VII 战斗的原始归一化帧，用于检查增益图标遮挡下的助战卡标记，以及 HP／NP 字形。`assets/common/support-card-masked.png` 从上游 `rider/supportCard.png` 派生，仅匹配右侧 45 像素以排除左侧图标遮挡，保留原图尺寸及定位区域。
- `tests/fixtures/live-saber-retest-*.png`、`live-saber-last-vii.png` 和 `live-saber-bond-level-up.png` 来自 2026-10-06 第二场剑阶 VII 验证，保留采集时的归一化画面。`assets/saber/questVII-masked.png` 从既有 `questVII.png` 派生，保留文字及完整罗马数字区域；`assets/live/result-tap-masked.png` 从既有 `result-tap.png` 派生，只匹配明亮文字以排除底部发光背景。两者均保留原尺寸和定位区域。
- `tests/fixtures/saber-*-card.png` 为本地 Atlas CN `13365c` 缓存中红阎魔（103700）、新娘尼禄（100600）和宫本伊织（106000）的 98×47 指令卡裁图，原图来自对应 `https://api.atlasacademy.io/nice/CN/servant/{id}` 返回的 command asset 链接。
- `sync-data` 从 Atlas Academy API 获取 FGO 数据并在本地生成派生索引和按需详情缓存；Atlas Academy 收集的数据按 ODC-BY 1.0 提供，生成文件仍受其来源与游戏权利人的相应条款约束：https://atlasacademy.io/about-us/ 。

游戏截图、角色图像和游戏界面素材的权利属于其各自权利人，不因本项目的代码许可而重新授权。上游也对部分图像与数据另有说明，发布资源包前应核对原始来源。
