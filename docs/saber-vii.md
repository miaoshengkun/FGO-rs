# 剑阶冠位战 VII

单场入口为 `scripts/saber_battle.rhai`，周回入口为 `scripts/saber_farm.rhai`，使用 `profiles/saber-v1.json`。CN 实机模板已接入；GUI 已完成两回合胜利、结算和完整事件回放，覆盖边界见 [验证](validation.md)。

队伍：1 阿蒂拉、2 红阎魔、3 好友宫本伊织（120/120）、4 剑阶美杜莎、5 尼禄·克劳狄乌斯〔新娘〕、6 兰陵王。御主礼装需提供配置的强化与换人技能。运行区服默认 CN，本地 Atlas 详情需与运行区服一致。

```sh
cargo run --locked -- sync-data --profile profiles/saber-v1.json
cargo run --locked -- run scripts/saber_farm.rhai \
  --profile profiles/saber-v1.json --serial 127.0.0.1:5559 \
  --execute --battles 1 --apples 0 --max-seconds 900
```

## 战斗顺序

首回合：阿蒂拉 1、3 技能 → 御主 3 技能交换队伍 1/5 → 新娘尼禄全部技能（可选目标均指向助战）→ 红阎魔全部技能 → 伊织全部技能 → 御主 1 技能 → 伊织宝具，余下两张优先助战普通卡。

后续每回合：

1. 伊织宝具可用时释放并结束该回合。
2. 否则检查其他前排宝具，可用时按 Profile 顺序释放并结束该回合。
3. 没有可用宝具时，按前排 3、1、2，技能 1、2、3 的顺序检查；冷却/禁用跳过，未知停止。每次成功施放后重新检查伊织宝具，可用即攻击。
4. 技能用完仍未充满时按 Profile 选卡推进，超过 100 回合停止。

`attack_np(slot, required)` 在选卡界面检查指定前排宝具，只允许该宝具参与本次选卡；不可用时退回并验证技能界面，`required=true` 则报错。`attack(true)` 的三宝具要求保留。

## 助战与识别要求

助战列表要求同一行宫本伊织名称、完整 120/120 等级及好友标记，编队第 3 位再次确认身份与等级。`support_candidates` 只提供卡面候选，不能代替列表身份验证。

关卡、职阶筛选、编队及战斗前排标记均在 `assets/templates.json` 中提供，包含新旧标题变体。缺少模板时启动校验拒绝运行；错误身份或等级不会退化为任选 120 级好友。

单场脚本从尚未操作的首回合开始。周回脚本支持关卡入口、助战、结算和连续出击，并受 `--battles`、`--apples` 与时间预算约束。开场非关键技能被关卡禁用时跳过，必要事实未知仍停止。
