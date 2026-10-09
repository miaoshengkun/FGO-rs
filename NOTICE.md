# 来源与资源说明

本项目从相邻的 FGO-py 项目开始迁移，沿用 AGPL-3.0-only 代码许可；原始许可文本见 LICENSE。

- 上游项目：https://github.com/hgjazhgj/FGO-py
- 原作者声明：Copyright (C) 2019-2026 hgjazhgj。
- `assets/rider/` 来自本地 FGO-py 的 `FGO-py/fgoImage/riderGrand/`。
- `assets/common/` 来自本地 FGO-py 的 `FGO-py/fgoImage/`。
- `assets/cn/` 来自本地 FGO-py 的 `FGO-py/fgoImage/cn/`。
- `assets/templates.json` 的区域与阈值，以及骑阶好友判定规则，参考 `fgoDetect.py`、`fgoRiderGrandBattle.py`。
- `tests/fixtures/step*.png` 来自本地 `temp/RiderGrandBattleScreenshots/`，用于迁移回归。
- `tests/fixtures/live-*.png` 为 2026-09-10/11 经用户授权，从本地 MuMu 模拟器通过 ADB 采集并归一化的游戏画面；`assets/live/` 为这些画面裁出的识别模板。
- `sync-data` 从 Atlas Academy API 获取 FGO 数据并在本地生成派生索引和按需详情缓存；Atlas Academy 收集的数据按 ODC-BY 1.0 提供，生成文件仍受其来源与游戏权利人的相应条款约束：https://atlasacademy.io/about-us/ 。

游戏截图、角色图像和游戏界面素材的权利属于其各自权利人，不因本项目的代码许可而重新授权。上游也对部分图像与数据另有说明，发布资源包前应核对原始来源。
