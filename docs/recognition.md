# 识别后端

默认后端使用 Rust 模板匹配和固定字体数字识别。启用 `maa` feature 并提供原生库后，模板匹配由 MaaFramework 执行；提供 OCR 模型时还能补充指令卡颜色。HP、NP、身份与充能计数仍使用 Rust 逻辑。

## 当前职责

| 事实 | Rust 后端 | Maa 后端 |
| --- | --- | --- |
| 场景及 manifest 模板 | 带掩码 SQDIFF 匹配 | 原生匹配，保留相同 ROI、阈值和变体 |
| 普通卡颜色 | 模板 | 模板优先，未命中时 OCR 补充 |
| HP / NP 数值 | 完整字形及空间格式校验 | 运行沿用 Rust，原生 OCR 节点仅用于诊断 |
| 卡牌身份 | Atlas 候选、最佳/次优分差 | 沿用 Rust 身份匹配 |
| 敌方充能 | 连通组件计数 | 沿用 Rust 计数 |
| 场景冲突、命令与验证 | 核心 | 核心 |

manifest 有 203 个模板条目、97 张共享 PNG；Maa 资源包含 16 个识别节点和 5 张 PNG。`alternatives` 表示相同语义、相同尺寸的渲染变体，两个后端都取最低分并合并重复计数区域。

透明像素转换为 Maa 绿色掩码，不兼容的实体纯绿色像素报错。原生单次识别有 15 秒上限，支持取消，提取结果后清理缓存。正常未命中保留未知；后端加载或执行失败报错。

Maa 资源仅提交 recognition，不绑定设备 controller，不执行 `action` 或 `next`。`--maa-resource` 可重复指定，按顺序叠加额外资源。

## 构建与使用

仓库固定官方 Rust 绑定 `maa-framework = 1.25.2`，当前验收使用 MaaFramework SDK `v5.14.2`。从 [对应版本发布页](https://github.com/MaaXYZ/MaaFramework/releases/tag/v5.14.2) 获取匹配平台的完整 SDK，保留动态库同目录依赖。

```sh
cargo build --locked --release --features maa

# 原生模板匹配
target/release/fgo-rs \
  --maa-library .tools/maa/sdk/bin/libMaaFramework.dylib \
  inspect tests/fixtures/step12.png --battle

# 本机复用相邻 MaaFgo 的 OCR 模型
target/release/fgo-rs \
  --maa-library .tools/maa/sdk/bin/libMaaFramework.dylib \
  --maa-ocr-model ../MaaFgo/assets/resource/base/model/OCR \
  inspect tests/fixtures/step12.png --battle

# 独立数值诊断
target/release/fgo-rs \
  --maa-library .tools/maa/sdk/bin/libMaaFramework.dylib \
  --maa-ocr-model ../MaaFgo/assets/resource/base/model/OCR \
  inspect tests/fixtures/step10.png --maa-node 战斗_NP卡1
```

`run` 支持相同参数；GUI 在路径设置中选择 SDK 和 OCR 模型。未指定原生库时使用 Rust 后端。Windows 对应 `bin/MaaFramework.dll`；Intel macOS 需要 x86_64 SDK，两者的原生运行尚未在本机验收。

SDK 与 OCR 模型不提交到仓库。MaaFgo 资源来源和许可保留在 [NOTICE](../NOTICE.md)。

## 数值与身份边界

卡色 OCR 只接受一个候选、已知颜色文字和至少 `0.90` 的分数。低分、漏字、多候选保持 Unknown，不降低阈值来覆盖漏识别。

HP / NP 不能只凭 OCR 分数或数字范围判断完整性：遮住 `300%` 中间的 `0` 后，原生 OCR 可返回高分 `30%`。该反例已纳入原生测试；运行时 NP 字符间距校验仍拒绝它。HP 同样检查完整数字与千位逗号布局，异常标点不删除后拼成数字。

Atlas 身份候选需要通过阈值与次优分差。助战标记、角色候选和当前队伍映射共同参与归属判断；过渡帧或遮挡证据不足时保留未知。原生多目标 NMS 与充能槽连通组件计数含义不同，不能替代。

当前语料覆盖 CN 固定界面和部分渲染变体，其他区服、字体、遮挡及自然退场需独立验收。启用 Maa 不代表完整替代 FGO-py 的技能效果决策或全部 MaaFgo 流程。

## 原生回归

```sh
FGO_MAA_LIBRARY="$PWD/.tools/maa/sdk/bin/libMaaFramework.dylib" \
FGO_MAA_OCR_MODEL="$PWD/../MaaFgo/assets/resource/base/model/OCR" \
  cargo test --locked --features maa --test maa_recognition -- \
  --ignored --nocapture --test-threads=1

FGO_MAA_LIBRARY="$PWD/.tools/maa/sdk/bin/libMaaFramework.dylib" \
  cargo test --locked --features maa --lib \
  farm_vision::tests::fixed_team_tabs_with_native_recognition -- --ignored
```

测试实际加载 SDK，覆盖模板坐标、真实卡色与数值、缺字反例、场景与队伍稳定性、取消及错误。该 SDK 的 OCR 初始化并行执行会触发 Paddle domain 冲突，原生集成测试需串行运行。更换模型需要重新验证。常规检查与实机范围见 [验证](validation.md)。
