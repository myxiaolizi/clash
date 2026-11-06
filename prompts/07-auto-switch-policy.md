# 07 · 自动切换策略（Auto Switch Policy）

> **适用阶段**：P4（紧随 06 章测速引擎）
> **必读前置**：[01-master-prompt.md](01-master-prompt.md)、[appendices/C-conventions.md](appendices/C-conventions.md)（C.5 `SwitchPolicy`/`SwitchRecord`、C.6 `switch_apply_best`/`switch_apply_node`、C.7 `switch:applied`）、[06-speedtest-engine.md](06-speedtest-engine.md)
> **本文件覆盖**：`src-tauri/src/speedtest/selector.rs`、`src-tauri/src/domain/policy.rs`（纯函数）、切换历史落盘与回滚。

---

## 7.1 目标与铁律

| # | 目标 | 依据 | 落地 |
| --- | --- | --- | --- |
| A1 | 每一次自动切换都**可解释** | 01 §3-7 | `SwitchRecord` + 一行结构化日志（`[switch]` 模块） |
| A2 | 三件套防抖，绝不抖动 | C.5 `SwitchPolicy` | `tolerance_ms=30` / `min_dwell_sec=300` / `failure_threshold=2` |
| A3 | 只切 `scope` 允许的组 | C.5 `SwitchScope` | `Global` = 所有 `type: select` 组；`Group(G)` = 只切 G |
| A4 | 切换失败必须回滚，不留半切状态 | 本任务验收 | 逐组「快照 → PUT → GET 校验 → 失败回滚」 |
| A5 | 历史保留最近 200 条 | 本任务验收 | `switch_history.json` 环形裁剪 |
| A6 | 幂等 + 并发锁 | C.8 `E_BUSY` | 单飞（single-flight）+ `runId` 校验 |
| A7 | 判定逻辑纯函数可单测 | C.2 | `rank`/`decide` 不碰网络、不读时钟（`now` 入参） |
| A8 | 用户手动覆盖优先于一切自动策略 | C.5「手动覆盖语义」 | `manual_override_min` 内跳过该组；状态由 `SwitchRuntimeLocks` 持有 |
| A9 | `scope` 组必须为 `select` | C.5「组类型约束」 | 生成侧由 04 章保证；本章 `is_selector()` 兜底 |

> **C.5 组类型约束（v1.2）**：`switch_policy.scope` 指向的策略组**必须**生成为 `type: select`，
> 禁止同名 `url-test`/`fallback`。否则内核自带健康检查会与本章的显式 `PUT` 争夺当前节点。
> 本章只负责「读到的组不是 Selector 就跳过」，生成侧的保证见 [04-multi-subscription-merge.md](04-multi-subscription-merge.md)。
>
> 📌 **完整的错误写法对照表见 [附录 A §A.6.4「禁止生成自动策略组」](appendices/A-mihomo-api-and-config.md)**
> ——那里逐条列出「`select` 里嵌套 `url-test`」「同名 `select` + `url-test` 并存」「`fallback` 组」
> 三种错误写法各自会造成什么现象。**本章不重复展开**，只说明「为什么这件事对本策略是致命的」：
> 本章的全部正确性都建立在「本应用是唯一的决策者」之上——`min_dwell_sec` 的驻留计时、
> `tolerance_ms` 的换手门槛、`SwitchRuntimeLocks` 的手动覆盖期，
> 这三者都假设「除了我们没人会改 `now`」。一旦内核的 `url-test` 也在改，
> 第 7.5 节的判定顺序虽然照常成立，但**它读到的前提已经不成立了**：
> 我们会在自己没发起切换的情况下观察到 `now` 变了，于是 `last_switch_at` 记的不是真实切换时间，
> `min_dwell` 形同虚设。这类 bug 表现为「日志里没有 `[switch] applied` 但节点变了」，
> 极难定位。因此 A9 是本章的**前置条件**，不是可选优化。

---

## 7.2 契约字段速查（不得发明新字段）

> 对齐 **C 章 v1.4**。三处 v1.2 裁决已生效：`to_delay: Option<u32>`、`manual_override_min`、
> `switch_apply_best` 返回 `{primary, all}`。

```rust
// C.5 原文（本章直接使用，不新增）
pub struct SwitchPolicy {
    pub enabled: bool,                  // 总开关
    pub scope: SwitchScope,             // Global | Group(String)
    pub tolerance_ms: u32,              // 默认 30
    pub min_dwell_sec: u32,             // 默认 300
    pub failure_threshold: u8,          // 默认 2
    pub manual_override_min: u32,       // v1.2 新增，默认 60；0 = 不暂停
    pub exclude_keywords: Vec<String>,  // 默认 ["到期","过期","剩余","流量","官网","套餐","重置","客服","域名"]
    pub prefer_region_priority: Vec<String>,
}
pub enum SwitchScope { Global, Group(String) }

pub struct SwitchRecord {
    pub at: i64,
    pub group: String,
    pub from_node: Option<String>,
    pub to_node: String,
    pub from_delay: Option<u32>,
    pub to_delay: Option<u32>,          // v1.2：Manual 时可能无新鲜数据 → None（不得用 0 冒充）
    pub reason: SwitchReason,
}
pub enum SwitchReason { Best, CurrentFailed, Manual, GroupMissing }
```

| 字段 | 语义 | 本章用法 |
| --- | --- | --- |
| `tolerance_ms` | 新候选须**严格快于**当前节点 ≥ 该值才切 | `cur_delay - new_delay >= tolerance_ms` |
| `min_dwell_sec` | 上次切换后最短驻留，期间不切（手动覆盖期除外） | `now - last_switch_at < min_dwell_sec` → 抑制 |
| `failure_threshold` | 当前节点**连续** N 轮失败才降级 | `consecutive_failures >= failure_threshold` |
| `manual_override_min` | 手动选节点后该组暂停自动切换的分钟数（0 = 不暂停） | `now - locked_at < manual_override_min * 60` → 跳过该组 |
| `exclude_keywords` | 信息节点关键词（交给 06 章 `ExcludeRules`） | 引擎侧过滤，本章不重复 |
| `prefer_region_priority` | 延迟并列时的地区偏好 | `rank_candidates_with_priority` 的 tiebreak |
| `scope` | 决定 PUT 哪些组 | `Global` → 所有 `Selector` 组；`Group` → 仅该组 |

> **注意**：`SwitchPolicy` 里**没有** `check_interval`、`cooldown_ms`、`hysteresis` 这类字段。
> 需要的话只能由 `min_dwell_sec` + `tolerance_ms` 组合表达，或提 `CONTRACT-CHANGE-REQUEST:`。
>
> `manual_override_min` 与 `min_dwell_sec` **是两件事，不要混**：
> - `min_dwell_sec` = 系统**自己**切过之后，怕抖，先忍 5 分钟；
> - `manual_override_min` = **用户**手动点了节点，尊重用户意图，暂停 60 分钟。
> 前者由 `GroupHistory.last_switch_at` 驱动，后者由 `SwitchRuntimeLocks.locked_at` 驱动，互不替代。

---

## 7.3 判定伪代码（先读这个再看代码）

```
decide_and_apply(run, group_state, policy, now):

  0. 前置
     if policy.enabled == false            → 不切（reason 不记录）
     if run.finished_at is None            → E_BUSY（runId 未完成）
     if targets(policy.scope) 为空          → 不切（记 GroupMissing）

  1. 排序（06 章 rank_candidates_with_priority 已算好）
     ranked = [c0, c1, c2, ...]   # delay 升序、地区、名称序
     if ranked 为空                         → 全部失败：不切，发 notify

  2. 逐组决策（对 should_sync_group 为真的每个组 G）
     cur      = GET /proxies/G → now（当前选中节点名）
     cur_res  = summary.rank_of(cur)

     # 2.0 手动覆盖期（v1.2，优先级最高）
     if locked_at[G] exists and manual_override_min > 0
        and now - locked_at[G] < manual_override_min * 60
            → 跳过该组（suppress = ManualOverride），**不发任何 PUT**
     # 到期自动解除：now - locked_at >= manual_override_min*60 → 视同不存在
     # manual_override_min == 0 → 覆盖立即失效，正常参与决策

     if cur_res is None                     # 当前节点不在候选里
         if summary.is_excluded(cur)        # 用户手选了 DIRECT / 信息节点
             → 尊重用户，不切（suppress = CurrentExcluded）
         else
             → reason = GroupMissing，切到 ranked[0]
             → 立即执行（不受 tolerance/min_dwell 约束，但受覆盖期约束）

     if summary.is_failed(cur)              # 当前节点本轮失败
         fails[G] += 1
         if fails[G] >= failure_threshold
             → reason = CurrentFailed，切到 ranked[0]，fails[G] = 0
             → 受 min_dwell 约束
         else
             → 抑制（suppress = FailureThreshold，攒够次数再说）
     else
         fails[G] = 0                        # 成功一轮即清零
         cur_delay = cur_res.delay_ms
         if ranked[0].node_key == cur        → 已最优，短路（suppress = BelowTolerance）
         if cur_delay - ranked[0].delay_ms < tolerance_ms
             → 抑制（suppress = BelowTolerance）
         if now - last_switch_at[G] < min_dwell_sec
             → 抑制（suppress = MinDwell）
         else
             → reason = Best，切到 ranked[0]

  3. 执行（逐组）
     for G in pending:
         snapshot = cur
         PUT /proxies/G {"name": target}
         失败 → 回滚到 snapshot，记 WARN，本组记 failed
         成功 → GET /proxies/G 校验 now == target
                校验失败 → 回滚，记 WARN
                校验通过 → last_switch_at[G] = now
                           locked_at[G] 非本组 → 不动；自动切换成功不清除覆盖期
                           persist(SwitchRecord{ to_delay: Some(目标延迟) })
                           emit("switch:applied", record)   # 逐组广播

  4. 返回（v1.2）
     primary = 主组记录（scope=Group 时即该组；scope=Global 时取 groups 排序第一的生效组）
     all     = 全部实际生效的 SwitchRecord
     return { primary, all }
```

> **步骤 2.0 与步骤 3 的一处 v1.2 关键点**：自动切换**成功不再清除**覆盖期。
> 覆盖期只由「到期」或「用户再次手动选择」改变。既然覆盖期内根本不会进入步骤 3，
> 这条在实现上是冗余保险，但必须写明——否则后续维护者会以为「切过就解除」。

---

## 7.4 纯函数实现（`domain/policy.rs`）

```rust
// domain/policy.rs —— 零 IO、零时钟、零 tokio，可 100% 单测
use crate::domain::model::{SwitchPolicy, SwitchReason, SwitchScope};
// C.6.1 契约：GroupInfo / SelectableGroup 定义在 kernel/api.rs（08 章实现）
use crate::kernel::api::{GroupInfo, SelectableGroup};

/// `GroupInfo` 的语义扩展（**只加方法，不加字段**，不违反 C.6.1 冻结面）。
pub trait GroupInfoExt {
    /// 只有 select 语义的组才能被 PUT 切换。`kind` 是 mihomo `type` 原值。
    fn is_selector(&self) -> bool;
    fn group_name(&self) -> &str;
}

impl GroupInfoExt for GroupInfo {
    fn is_selector(&self) -> bool {
        self.kind.eq_ignore_ascii_case("Selector")
    }
    fn group_name(&self) -> &str {
        &self.name
    }
}

/// `SelectableGroup` 的语义扩展，同理只加方法。
pub trait SelectableGroupExt {
    fn member_count(&self) -> usize;
}

impl SelectableGroupExt for SelectableGroup {
    fn member_count(&self) -> usize {
        self.members.len()
    }
}
```

> **为什么用扩展 trait 而不是重新定义类型**：C.6.1 已冻结 `GroupInfo {name, kind, now, all, test_url, hidden}`
> 与 `SelectableGroup {name, members, now}` 的字段，07 章**不得**自建 `GroupView` 之类的影子类型
> （那正是 v1.2→v1.3 之间出现编译级不一致的根因）。字段名以契约为准：`name`（不是 `group`）、
> `kind`（不是 `group_type`）、`members`（不是 `all`）。需要的便利方法一律走扩展 trait。
>
> **两种类型的分工**（C.6.1 原意）：
> - `SelectableGroup`（`proxies_selectable` 返回）：**便宜**——一次 `GET /proxies` 拿到所有可切换组
>   的名字与成员，只用于**筛选**「哪些组该参与决策」。
> - `GroupInfo`（`proxy_group` 返回）：**完整**——单组的 `kind`、`now`、`all`、`test_url`，
>   用于**判定**与执行时的快照/校验。
>
> 代价是 `apply_best` 对每个命中组多一次 `GET /proxies/{group}`（N+1）。实测 N ≤ 6（典型订阅的
> select 组数），单次 <5ms，相对一次测速（~2s）可忽略；换来的是纯函数能拿到完整 `all` 做
> 「当前节点是否真的消失」的准确判定。**不要**为了省这一次请求用 `members` 顶替 `all`——
> `members` 不含 DIRECT/REJECT，会把「用户手选 DIRECT」误判成「节点消失」。

```rust
// domain/policy.rs —— 续：本章自有的判定状态类型

/// 排序后的候选（与 06 章 `speedtest::rank::Candidate` 同构，
/// 此处复制一份以避免 domain 反向依赖 speedtest —— 由 `From` 转换）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedNode {
    pub node_key: String,
    pub delay_ms: u32,
}

/// 单个组的**自动切换**历史状态（内存态，不落盘）
///
/// 与 `SwitchRuntimeLocks` 的分工（C.5 v1.2）：
/// - `GroupHistory`：系统自己的节奏（驻留时间、连续失败数）→ 防抖用
/// - `SwitchRuntimeLocks`：用户手动覆盖期 → 尊重用户用
/// 两者都只在内存，重启清空。
#[derive(Debug, Clone, Default)]
pub struct GroupHistory {
    /// 上次成功切换的 Unix 秒
    pub last_switch_at: Option<i64>,
    /// 当前节点连续失败轮数
    pub consecutive_failures: u8,
}

/// C.5 v1.2 定稿：手动覆盖（manual override）状态。
/// 定义为**独立结构体**而非塞进 `GroupHistory`，因为它是契约里的概念
/// （`SwitchRuntimeLocks { group, locked_at }`），必须与纯函数的内部状态分开命名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRuntimeLocks {
    pub group: String,
    /// 用户最后一次手动选择该组节点的 Unix 秒
    pub locked_at: i64,
}

impl SwitchRuntimeLocks {
    /// 纯函数：覆盖期是否仍然生效。
    /// `manual_override_min == 0` → 恒为 false（立即失效，可被自动切换覆盖）
    /// `now - locked_at >= manual_override_min * 60` → 到期解除
    pub fn is_active(&self, now: i64, manual_override_min: u32) -> bool {
        if manual_override_min == 0 {
            return false;
        }
        let window = (manual_override_min as i64).saturating_mul(60);
        now.saturating_sub(self.locked_at) < window
    }

    /// 同一组再次手动选择 → 以最后一次时间续期（C.5 明文要求）
    pub fn renew(&mut self, now: i64) {
        self.locked_at = now;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub group: String,
    pub to_node: String,
    pub from_node: Option<String>,
    pub from_delay: Option<u32>,
    /// 目标节点的实测延迟。Manual 场景可能没有数据 → None
    /// （与 C.5 v1.2 的 `SwitchRecord.to_delay: Option<u32>` 保持一致）
    pub to_delay: Option<u32>,
    pub reason: SwitchReason,
    /// 抑制原因（None = 应当执行）
    pub suppressed_by: Option<Suppress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppress {
    /// 新节点优势不足 tolerance_ms
    BelowTolerance,
    /// 上次切换距今不足 min_dwell_sec
    MinDwell,
    /// 当前节点失败轮数未达 failure_threshold
    FailureThreshold,
    /// 手动覆盖期内（C.5 v1.2），本组暂停自动切换
    ManualOverride,
    /// 当前节点被排除规则挡掉（信息节点/内置策略），尊重用户不切
    CurrentExcluded,
}

/// 本轮测速结果的三种身份（纯数据，由 06 章引擎汇总后传入）
///
/// **必要性**：`ranked` 只含「成功且未被排除」的节点。仅凭它无法区分
/// 「当前节点本轮失败」与「当前节点被 exclude 规则排除」——两者应对措施完全相反
/// （前者要降级切换，后者要尊重用户）。所以必须三分类。
#[derive(Debug, Clone, Default)]
pub struct ProbeSummary {
    /// 成功候选，已按 06 章规则排序
    pub ranked: Vec<RankedNode>,
    /// 参与了探测但失败（delay=None）的 node_key
    pub failed: Vec<String>,
    /// 被 exclude 规则排除的 node_key（DIRECT/REJECT/信息节点）
    pub excluded: Vec<String>,
}

impl ProbeSummary {
    pub fn rank_of(&self, node: &str) -> Option<&RankedNode> {
        self.ranked.iter().find(|r| r.node_key == node)
    }
    pub fn is_failed(&self, node: &str) -> bool {
        self.failed.iter().any(|n| n == node)
    }
    pub fn is_excluded(&self, node: &str) -> bool {
        self.excluded.iter().any(|n| n == node)
    }
}

/// **核心判定函数**：纯函数，输入 now/policy/组视图/组历史/本轮结果 → 输出决策
///
/// 调用方（selector.rs）负责：
///   1) 构造 `summary`（06 章）
///   2) 取 `view`（GET /proxies）
///   3) 取 `history`（内存态）与 `lock`（手动覆盖态，C.5 v1.2）
///   4) 执行返回的 Decision 并回写 history
pub fn decide(
    now: i64,
    policy: &SwitchPolicy,
    view: &GroupInfo,
    history: &GroupHistory,
    lock: Option<&SwitchRuntimeLocks>,
    summary: &ProbeSummary,
) -> Option<Decision> {
    // 0. 总开关与组类型（C.5 v1.2 组类型约束的兜底）
    if !policy.enabled || !view.is_selector() {
        return None;
    }
    // scope 过滤：Group(G) 只处理 G
    if let SwitchScope::Group(g) = &policy.scope {
        if g != &view.name {
            return None;
        }
    }
    // 没有任何可用候选（全部失败）→ 不做任何事，由调用方发 notify
    let best = summary.ranked.first()?;

    let cur_name = match &view.now {
        Some(n) => n.clone(),
        None => {
            // 组没有任何选中节点（刚创建/内核刚起）→ 直接选最优
            return Some(Decision {
                group: view.name.clone(),
                to_node: best.node_key.clone(),
                from_node: None,
                from_delay: None,
                to_delay: Some(best.delay_ms),
                reason: SwitchReason::GroupMissing,
                suppressed_by: None,
            });
        }
    };

    // 1. 手动覆盖期（最高优先级，C.5 v1.2）
    //    在覆盖期内：跳过该组，不发任何 PUT。到期自动解除，无需显式清理。
    //    注意这里**不看**锁定节点是否存活——覆盖期是「用户说了算」的时间窗，
    //    判断存活会引入「锁定节点失败就偷偷切走」的意外行为。
    if let Some(l) = lock {
        if l.is_active(now, policy.manual_override_min) {
            return Some(Decision {
                group: view.name.clone(),
                to_node: cur_name.clone(),
                from_node: Some(cur_name.clone()),
                // 当前节点不一定是可测候选（可能被排除），延迟如实取 None
                from_delay: summary.rank_of(&cur_name).map(|r| r.delay_ms),
                to_delay: None, // 覆盖期内不产生切换，目标延迟无意义
                reason: SwitchReason::Manual,
                suppressed_by: Some(Suppress::ManualOverride),
            });
        }
    }

    // 2. 当前节点身份判定（三分类，顺序不可换）
    if let Some(cur) = summary.rank_of(&cur_name) {
        // 2a. 成功：走延迟比较 + 防抖
        // 已是最好 → 最常见路径，最先短路
        if best.node_key == cur.node_key {
            return Some(Decision {
                group: view.name.clone(),
                to_node: cur.node_key.clone(),
                from_node: Some(cur.node_key.clone()),
                from_delay: Some(cur.delay_ms),
                to_delay: Some(cur.delay_ms),
                reason: SwitchReason::Best,
                suppressed_by: Some(Suppress::BelowTolerance),
            });
        }
        // tolerance：新节点必须快出至少 tol 毫秒，否则不动
        let gain = cur.delay_ms.saturating_sub(best.delay_ms);
        if gain < policy.tolerance_ms {
            return Some(Decision {
                group: view.name.clone(),
                to_node: best.node_key.clone(),
                from_node: Some(cur.node_key.clone()),
                from_delay: Some(cur.delay_ms),
                to_delay: Some(best.delay_ms),
                reason: SwitchReason::Best,
                suppressed_by: Some(Suppress::BelowTolerance),
            });
        }
        // min_dwell：刚切过不久，忍住
        if let Some(last) = history.last_switch_at {
            if now.saturating_sub(last) < policy.min_dwell_sec as i64 {
                return Some(Decision {
                    group: view.name.clone(),
                    to_node: best.node_key.clone(),
                    from_node: Some(cur.node_key.clone()),
                    from_delay: Some(cur.delay_ms),
                    to_delay: Some(best.delay_ms),
                    reason: SwitchReason::Best,
                    suppressed_by: Some(Suppress::MinDwell),
                });
            }
        }
        // 放行
        return Some(Decision {
            group: view.name.clone(),
            to_node: best.node_key.clone(),
            from_node: Some(cur.node_key.clone()),
            from_delay: Some(cur.delay_ms),
            to_delay: Some(best.delay_ms),
            reason: SwitchReason::Best,
            suppressed_by: None,
        });
    }

    // 2b. 被排除（用户在组里手选了 DIRECT / 信息节点）→ 尊重，不切
    if summary.is_excluded(&cur_name) || !view.all.iter().any(|a| a == &cur_name) {
        if summary.is_excluded(&cur_name) {
            return Some(Decision {
                group: view.name.clone(),
                to_node: cur_name.clone(),
                from_node: Some(cur_name),
                from_delay: None,
                to_delay: Some(best.delay_ms),
                reason: SwitchReason::Best,
                suppressed_by: Some(Suppress::CurrentExcluded),
            });
        }
        // 2c. 不在 all 里 = 当前节点彻底消失（订阅换了节点池）→ 立即切，不受 min_dwell 约束
        return Some(Decision {
            group: view.name.clone(),
            to_node: best.node_key.clone(),
            from_node: Some(cur_name),
            from_delay: None,
            to_delay: Some(best.delay_ms),
            reason: SwitchReason::GroupMissing,
            suppressed_by: None,
        });
    }

    // 2d. 本轮探测失败 → 交给 decide_on_failure（失败计数在 history 里）
    decide_on_failure(now, policy, view, history, lock, summary)
}

/// 当前节点本轮失败时的判定（独立函数，语义更清晰，也便于单测）
///
/// 覆盖期（`lock`）已在 `decide` 步骤 1 拦截过，这里不需要重复判断：
/// 能走到本函数即说明覆盖期不生效。
pub fn decide_on_failure(
    now: i64,
    policy: &SwitchPolicy,
    view: &GroupInfo,
    history: &GroupHistory,
    lock: Option<&SwitchRuntimeLocks>,
    summary: &ProbeSummary,
) -> Option<Decision> {
    if !policy.enabled || !view.is_selector() {
        return None;
    }
    if let SwitchScope::Group(g) = &policy.scope {
        if g != &view.name {
            return None;
        }
    }
    // 防御性重复判断：本函数可能被单独调用（如单测），不能依赖调用方已过滤
    if let Some(l) = lock {
        if l.is_active(now, policy.manual_override_min) {
            return None;
        }
    }
    let best = summary.ranked.first()?;
    let cur_name = view.now.clone()?;

    // 失败轮数未达阈值 → 再观察（`consecutive_failures` 已由调用方 +1）
    if history.consecutive_failures < policy.failure_threshold {
        return Some(Decision {
            group: view.name.clone(),
            to_node: best.node_key.clone(),
            from_node: Some(cur_name),
            from_delay: None,
            to_delay: Some(best.delay_ms),
            reason: SwitchReason::CurrentFailed,
            suppressed_by: Some(Suppress::FailureThreshold),
        });
    }

    // 达到阈值：切，但 min_dwell 仍然生效
    if let Some(last) = history.last_switch_at {
        if now.saturating_sub(last) < policy.min_dwell_sec as i64 {
            return Some(Decision {
                group: view.name.clone(),
                to_node: best.node_key.clone(),
                from_node: Some(cur_name),
                from_delay: None,
                to_delay: Some(best.delay_ms),
                reason: SwitchReason::CurrentFailed,
                suppressed_by: Some(Suppress::MinDwell),
            });
        }
    }

    Some(Decision {
        group: view.name.clone(),
        to_node: best.node_key.clone(),
        from_node: Some(cur_name),
        from_delay: None,
        to_delay: Some(best.delay_ms),
        reason: SwitchReason::CurrentFailed,
        suppressed_by: None,
    })
}
```

> **`Decision` 里为什么带 `suppressed_by`**：让「不切」也可解释。日志可以写
> `[switch] hold group=G cur=HK-01 cur_delay=88ms best=JP-02 best_delay=61ms gain=27ms needed=30ms reason=BelowTolerance`，
> 用户问「为什么不切」时有据可查。**这是本产品「可解释」铁律的关键补充，且不新增契约字段**
> （`Decision` 是本章内部类型，不落盘、不出 IPC）。

### 7.4.1 从 06 章结果构造 `ProbeSummary`

```rust
// speedtest/selector.rs（构造处，紧邻 06 章引擎的 done 回调）
use crate::domain::policy::{ProbeSummary, RankedNode};
use crate::speedtest::rank::{self, ExcludeRules};

/// 把 06 章的 `Vec<ProbeResult>` 转成判定所需的 `ProbeSummary`。
/// 纯函数，可单测（用例见 §7.10 第 9/10/11 行）。
pub fn build_summary(
    results: &[crate::domain::model::ProbeResult],
    rules: &ExcludeRules,
    region_priority: &[String],
) -> ProbeSummary {
    let mut failed = Vec::new();
    let mut excluded = Vec::new();
    for r in results {
        if rank::exclude_reason(&r.node_key, r.kind, rules).is_some() {
            excluded.push(r.node_key.clone());
        } else if r.delay_ms.is_none() {
            failed.push(r.node_key.clone());
        }
    }
    let ranked = rank::rank_candidates_with_priority(results, rules, region_priority)
        .into_iter()
        .map(|c| RankedNode { node_key: c.node_key, delay_ms: c.delay_ms })
        .collect();
    ProbeSummary { ranked, failed, excluded }
}
```

> **一个节点只能进一个桶**：先判 `excluded` 再判 `failed`，顺序不可换。
> 若反过来，一个名字含「剩余流量」且本轮超时的节点会同时进 `failed` 与 `excluded`，
> `decide` 里「是否排除」的判断就会被失败路径抢先，导致用户手选的信息节点被自动切走。

---

## 7.5 三件套防抖的精确公式与判定顺序

### 7.5.1 公式

设：
- `cur` = 当前选中节点，`cur_delay` = 本轮实测延迟（失败为 `None`）
- `best` = 排序后第一名，`best_delay` = 它的延迟
- `tol` = `policy.tolerance_ms`（默认 30）
- `dwell` = `policy.min_dwell_sec`（默认 300）
- `ft` = `policy.failure_threshold`（默认 2）
- `ovr` = `policy.manual_override_min`（默认 60，v1.2 新增）
- `elapsed` = `now - last_switch_at`（无历史则为 `+∞`）
- `lock_age` = `now - locked_at`（无覆盖则为 `+∞`）

| 判定 | 条件 | 动作 |
| --- | --- | --- |
| ⓪ 手动覆盖期 | `ovr > 0` 且 `lock_age < ovr × 60` | 跳过该组（`ManualOverride`），**不 PUT** |
| ① 已是最好 | `cur == best.node_key` | 保持（`BelowTolerance`） |
| ② 优势不足 | `cur_delay - best_delay < tol` | 保持（`BelowTolerance`） |
| ③ 优势足够但驻留不够 | `cur_delay - best_delay >= tol` **且** `elapsed < dwell` | 保持（`MinDwell`） |
| ④ 切换 | `cur_delay - best_delay >= tol` **且** `elapsed >= dwell` | 切换到 `best`（`Best`） |
| ⑤ 失败未达阈值 | `cur_delay is None` 且 `fails < ft` | 保持（`FailureThreshold`） |
| ⑥ 失败达阈值 | `cur_delay is None` 且 `fails >= ft` 且 `elapsed >= dwell` | 切换（`CurrentFailed`） |
| ⑦ 当前节点消失 | `cur ∉ view.all` 且不在 `excluded` | **立即**切换（`GroupMissing`），不受 ②③⑤⑥ 约束 |
| ⑧ 当前节点被排除 | `cur ∈ excluded` | 尊重用户，不切（`CurrentExcluded`） |
| ⑨ 全组无候选 | `ranked.is_empty()` | 不切，发 `notify` |

### 7.5.2 判定顺序（顺序错了行为就错）

```
⓪  手动覆盖期      ← 最高优先级。用户手动选过就是用户说了算，其余定律全部让路
①  已是最好        ← 必须最先，99% 的轮次在这里短路，省掉后续开销
②  当前节点消失    ← 先于延迟比较：延迟数据对不存在的节点无意义
③  当前节点被排除  ← 先于失败判定：被排除的节点根本不该被探测
④  当前节点失败    ← 先于延迟比较，但受 failure_threshold 约束
⑤  tolerance       ← 延迟比较
⑥  min_dwell       ← 最后一道闸，只有「真要切」时才检查
```

> **为什么 `min_dwell` 放在最后**：如果放在最前面，一次切换后的 5 分钟内，
> 「当前节点消失」也会被拦住，导致组里挂着一个不存在的节点 5 分钟。
> 消失类必须能突破 `min_dwell`。反之，`tolerance` 不过关时根本不该消耗判断。

> **为什么 `manual_override_min` 必须排在最前**：覆盖期是**用户意图**的显式表达，
> 不是系统对延迟的判断。如果放在 `tolerance` 之后，就会出现
> 「用户刚手选完 A 节点，30ms 优势触发自动切走」——用户会觉得软件不听话。
>
> **`min_dwell_sec` 与 `manual_override_min` 的独立性**（易混点）：
> - 自动切换成功 → 只刷新 `last_switch_at`，**不动** `locked_at`；
> - 手动切换成功 → **同时**刷新 `last_switch_at` 与 `locked_at`（后者续期）。
> 因此一次手动切换会同时获得「5 分钟驻留」与「60 分钟覆盖」两层保护，前者先到期，
> 但后者仍然拦住后续切换 —— 这正是期望行为。

### 7.5.3 双阈值为什么能防抖（数值推演）

| 轮次 | cur_delay | best_delay | gain | elapsed | 结果 |
| --- | --- | --- | --- | --- | --- |
| 1 | 50 | 48 | 2 | 999 | tol 不过，保持 |
| 2 | 52 | 49 | 3 | 999+ | 保持 |
| 3 | 51 | 48 | 3 | 999+ | 保持 |
| 4 | 50 | 47 | 3 | — | 保持（延迟自然抖动 2~5ms 全被 tol=30 吸收） |
| 5 | 50 | 15 | 35 | 999+ | **切换**（真差异） |
| 6 | 15 | 12 | 3 | 30 | 保持（tol + dwell 双保险） |
| 7 | 15 | 14 | 1 | 400 | 保持（tol） |

无 `tol` 时第 1/2/3/4 轮会切 3 次；无 `dwell` 时第 6 轮可能因瞬时抖动回切。两者缺一都会抖。

---

## 7.6 selector.rs：执行层

```rust
// speedtest/selector.rs
use tokio::sync::Mutex;

use crate::domain::errors::AppError;
use crate::domain::model::{SwitchPolicy, SwitchReason, SwitchRecord};
use crate::domain::policy::{
    self, Decision, GroupHistory, ProbeSummary, SwitchRuntimeLocks,
};
use crate::kernel::api::{ApiError, CoreApi, GroupInfo, SelectableGroup, SelectorApi};

/// 切换历史落盘 + 内存环形缓冲
pub const HISTORY_LIMIT: usize = 200;

/// v1.2：`switch_apply_best` 的返回体（C.6 定稿）
#[derive(Debug, Clone)]
pub struct ApplyBestResult {
    /// 主组记录（scope=Group 时即该组；scope=Global 时取生效组中的第一个）
    pub primary: SwitchRecord,
    /// 全部实际生效的组（scope=Group 时长度为 1）
    pub all: Vec<SwitchRecord>,
}

pub struct Selector {
    /// 防止并发 apply（C.8 E_BUSY）
    /// 注意：`try_lock` 而非 `lock`，抢不到即 E_BUSY，绝不排队等待
    apply_lock: Mutex<()>,
    /// group → 自动切换历史（内存态，不落盘）
    history: Mutex<std::collections::HashMap<String, GroupHistory>>,
    /// C.5 v1.2：group → 手动覆盖状态（内存态，不落盘、不进 RuntimeState）
    locks: Mutex<std::collections::HashMap<String, SwitchRuntimeLocks>>,
    /// 最近 200 条，新记录 push_front
    records: Mutex<std::collections::VecDeque<SwitchRecord>>,
}

impl Selector {
    pub fn new() -> Self {
        Self {
            apply_lock: Mutex::new(()),
            history: Mutex::new(Default::default()),
            locks: Mutex::new(Default::default()),
            records: Mutex::new(std::collections::VecDeque::with_capacity(HISTORY_LIMIT)),
        }
    }

    /// 读某个组的覆盖状态；已到期（或 `manual_override_min == 0`）返回 `None`。
    /// 到期不主动写回删除，由 `is_active` 判定即可——少一次加锁写入，也避免
    /// 「刚读到就过期」的竞态。
    async fn lock_of(&self, group: &str, now: i64, policy_cfg: &SwitchPolicy) -> Option<SwitchRuntimeLocks> {
        let m = self.locks.lock().await;
        m.get(group)
            .filter(|l| l.is_active(now, policy_cfg.manual_override_min))
            .cloned()
    }

    /// IPC `switch_apply_best { runId }` 的实现
    ///
    /// 前置校验（顺序不可换）：
    ///   1. runId 必须存在且 `finished_at.is_some()`，否则 E_BUSY
    ///   2. 抢 apply_lock（try_lock 失败 → E_BUSY）
    ///   3. 逐组决策 + 执行
    ///
    /// 返回 `ApplyBestResult { primary, all }`（C.6 v1.2）；没有任何组生效时
    /// 返回 `E_BUSY` 之外的「空结果」语义由调用方决定——见下方 `Ok(None)` 说明。
    #[allow(clippy::too_many_arguments)]
    pub async fn apply_best(
        &self,
        api: &CoreApi,
        policy_cfg: &SwitchPolicy,
        run_id: &str,
        run: &crate::domain::model::SpeedtestRun,
        summary: &policy::ProbeSummary,
        events: crate::event::EventSink,
        now: i64,
    ) -> Result<Option<ApplyBestResult>, AppError> {
        // ① run 完整性
        if run.id != run_id {
            return Err(AppError::new("E_BUSY", "测速任务不匹配，请重新测速").retryable(true));
        }
        if run.finished_at.is_none() {
            return Err(AppError::new("E_BUSY", "测速尚未完成，请稍候").retryable(true));
        }
        // ② 单飞
        let _guard = match self.apply_lock.try_lock() {
            Ok(g) => g,
            Err(_) => return Err(AppError::new("E_BUSY", "正在执行切换，请稍候").retryable(true)),
        };
        if !policy_cfg.enabled {
            tracing::info!("[switch] policy disabled, skip run={}", run_id);
            return Ok(None);
        }

        // ③ 取可切换组清单（轻量：一次 GET /proxies）
        let groups = api.proxies_selectable().await.map_err(map_api_err)?;
        // ④ 逐组决策：外层用 SelectableGroup 过滤（省请求），命中后再取 GroupInfo（要完整 all）
        let mut out: Vec<SwitchRecord> = Vec::new();
        for sel in groups.iter().filter(|g| policy::should_sync_group(policy_cfg, g)) {
            // N+1：只对通过过滤的组打一次 GET /proxies/{group}。典型 N ≤ 6，单次 <5ms。
            // 必须拿 GroupInfo 而不是复用 sel.members：members 不含 DIRECT/REJECT，
            // 用它会把手选 DIRECT 误判成「节点消失」。
            let view = api.proxy_group(&sel.name).await.map_err(map_api_err)?;
            let hist = {
                let h = self.history.lock().await;
                h.get(&view.name).cloned().unwrap_or_default()
            };
            let lock = self.lock_of(&view.name, now, policy_cfg).await;
            let decision = policy::decide(now, policy_cfg, &view, &hist, lock.as_ref(), summary);
            let Some(d) = decision else { continue };
            if let Some(sup) = d.suppressed_by {
                // `to_delay` 现在是 Option，gain 计算要两边都解包
                let gain = match (d.from_delay, d.to_delay) {
                    (Some(c), Some(b)) => c.saturating_sub(b),
                    _ => 0,
                };
                tracing::info!(
                    "[switch] hold group={} cur={:?} cur_delay={:?} best={} best_delay={:?} gain={}ms need={}ms suppress={:?}",
                    d.group, d.from_node, d.from_delay, d.to_node, d.to_delay,
                    gain, policy_cfg.tolerance_ms, sup
                );
                continue;
            }
            match self.execute_one(api, &d, events.clone(), now).await {
                Ok(rec) => out.push(rec),
                Err(e) => {
                    tracing::warn!("[switch] apply failed group={} target={} code={}", d.group, d.to_node, e.code);
                    // 单组失败不影响其它组（继续遍历）
                }
            }
        }

        // ⑤ 组装 v1.2 返回体
        if out.is_empty() {
            return Ok(None);
        }
        let primary = out[0].clone();
        Ok(Some(ApplyBestResult { primary, all: out }))
    }

    /// 06 章测速完成后的**唯一入口**（由引擎在 emit `speedtest:done` 前后调用）。
    /// 顺序固定：先累加失败计数，再判定切换 —— 反了则 failure_threshold 永远差 1 轮。
    pub async fn after_speedtest(
        &self,
        api: &CoreApi,
        policy_cfg: &SwitchPolicy,
        run: &crate::domain::model::SpeedtestRun,
        summary: &policy::ProbeSummary,
        events: crate::event::EventSink,
        now: i64,
    ) -> Result<Option<ApplyBestResult>, AppError> {
        self.update_failure_counters(api, policy_cfg, summary).await?;
        self.apply_best(api, policy_cfg, &run.id, run, summary, events, now)
            .await
    }
}

fn map_api_err(e: ApiError) -> AppError {
    use ApiError as A;
    match e {
        // C.6.1 v1.3 五变体，逐条对齐 08 §08.17.1
        A::Unauthorized => {
            AppError::new("E_CORE_API_UNAUTH", "内核 API 鉴权失败（secret 不一致）").retryable(false)
        }
        A::NotFound(_) => {
            AppError::new("E_CORE_START_FAILED", "内核 API 未找到目标（组名或节点名不存在）")
                .retryable(false)
        }
        // 刻意不归 E_CORE_START_FAILED：否则 UI 误报「内核启动失败」并暗示用户重启内核
        A::Timeout => AppError::new("E_NET_TIMEOUT", "内核响应超时").retryable(true),
        A::Status(code, _body) => {
            // 5xx 可重试，4xx 不可（组名写错这类错误重试一万次也没用）
            let retryable = (500..600).contains(&code);
            AppError::new("E_CORE_START_FAILED", format!("内核 API 返回异常状态 HTTP {code}"))
                .retryable(retryable)
        }
        A::Transport(_) => {
            AppError::new("E_CORE_START_FAILED", "无法连接内核，可能已退出").retryable(true)
        }
    }
}
```

> **`Decode` 去哪了？** C.6.1 的 `ApiError` **没有** `Decode` 变体。07 章的选择：
> 一律归入 **`Status(200, _)`** 分支（响应 200 但 body 无法解析），由 `map_api_err` 的
> `A::Status(200, _)` → `E_CORE_START_FAILED`（`retryable=false`，因为再试一次内核仍会返回同样的坏 body）。
> **不**归 `Transport`——`Transport` 的语义是「连接没建立」，而解析失败是「连接成功但内容不对」，
> 两者排障方向完全不同（后者要查内核版本/配置，不是查端口）。
> 08 章 `map_reqwest` 在 `json()` 失败时即构造 `ApiError::Status(200, <原始 body 片段>)`，
> 07 章无需为此新增变体。

### 7.6.1 内核 API 依赖面（C.6.1 契约冻结面，08 章实现）

本章只依赖 **1 个 trait + 3 个方法 + 3 个类型**，签名与 **C.6.1 逐字一致**，
定义在 `kernel/api.rs`（08 章）。测试用 §7.10 的 `FakeApi` 实现同一 trait。

```rust
// kernel/api.rs —— C.6.1 v1.4 契约原文，08 章实现、07 章消费。不得改字段名。
use std::future::Future;
use std::pin::Pin;

/// 可被切换的 select 组（**仅 `kind == "Selector"`**）。
/// `members` 为该组当前可选节点名（不含 DIRECT/REJECT，含则由 rank.rs 自行排除）。
pub struct SelectableGroup {
    pub name: String,
    pub members: Vec<String>,
    pub now: Option<String>,
}

/// 单组当前状态。对应 mihomo `GET /proxies/{group}`：{ name, type, now, all }
pub struct GroupInfo {
    pub name: String,
    pub kind: String,            // mihomo `type` 原值："Selector" / "URLTest" / "Fallback" …
    pub now: Option<String>,
    pub all: Vec<String>,
    pub test_url: Option<String>,
    pub hidden: bool,
}

/// 内核 API 的统一错误。**不携带 reqwest 类型**，否则 trait 就泄漏了实现细节。
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,                 // 401 → E_CORE_API_UNAUTH
    #[error("not found: {0}")]
    NotFound(String),             // 404：组/节点不存在
    #[error("timeout")]
    Timeout,                      // 读超时 → E_NET_TIMEOUT
    #[error("status {0}: {1}")]
    Status(u16, String),          // 其它非 2xx；也是 Decode 失败的落点（见上方说明）
    #[error("transport: {0}")]
    Transport(String),            // 连接失败：内核可能已退出
}

/// boxed-future 形式，使 trait **dyn 兼容**（Rust 1.85 下 `async fn in trait` 默认不 dyn 兼容）。
pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

pub trait SelectorApi: Send + Sync {
    /// GET /proxies，只返回 kind == "Selector" 的组
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>;
    /// GET /proxies/{group}
    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>;
    /// PUT /proxies/{group}  body {"name": name}  → 期望 204
    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>;
}
```

> **为什么是 boxed-future 而不是 `async fn`**（C.6.1 v1.4 定稿）：
> Rust 1.85 下 `async fn in trait` **默认不 dyn 兼容**，而本章需要用 `Arc<dyn SelectorApi>`
> 注入 §7.10 的 `FakeApi` 做纯逻辑单测。若写裸 `async fn`，`Arc<dyn SelectorApi>` 直接编译不过。
>
> - **不用 `async-trait` 宏**：它等价于 boxed future，但多一层宏依赖；本项目不扩大依赖白名单（C.2）。
> - **实现侧**：`fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> { Box::pin(async move { … }) }`
> - **调用侧**：写法与 `async fn` **完全一致**，直接 `.await`，无额外成本（每请求一次装箱，可忽略）。
>
> 因此本章所有 `api.xxx(...).await` 的写法**不需要任何改动**，改动只发生在 trait 定义与
> `impl` 的签名上。这正是选 boxed-future 而非让调用方感知差异的原因。

> **`proxy_group` 与 `set_group_selected` 必须共用同一个 secret 与 base URL**，
> 由 08 章 `CoreApi` 内部持有，不要在 selector 里自己拼 URL（否则 secret 管理会散落多处，
> 违反 C.12 第 2 条「每次启动或每次 config 重建随机 secret」）。
>
> **`test_url` / `hidden` 本章不读**：它们是 C.6.1 为 08/09 章准备的字段，07 章保留不用，
> 这样将来 UI 要显示「该组测速用哪个 URL」时不必改契约。**不要**因为「不用」就删掉它们。

### 7.6.2 单组执行 + 回滚

```rust
impl Selector {
    /// 一个组的三步事务：快照 → PUT → 校验（失败回滚）
    ///
    /// **顺序铁律**：必须先 GET 拿快照，再 PUT。反过来写进历史的 `from_node` 会变成新节点，
    /// 历史与日志全部错乱，且回滚时无「原节点」可回。
    async fn execute_one(
        &self,
        api: &CoreApi,
        d: &Decision,
        events: crate::event::EventSink,
        now: i64,
    ) -> Result<SwitchRecord, AppError> {
        // ① 快照（用于回滚）
        let before = api
            .proxy_group(&d.group)
            .await
            .map_err(map_api_err)?
            .now
            .clone();

        // ② PUT /proxies/{group}  body {"name": "<to_node>"}
        api.set_group_selected(&d.group, &d.to_node)
            .await
            .map_err(map_api_err)?;

        // ③ 校验：内核可能异步回写 / 被 profile.store-selected 干预
        let after = match api.proxy_group(&d.group).await {
            Ok(g) => g.now.clone(),
            Err(e) => {
                tracing::warn!("[switch] verify failed group={} rollback=1", d.group);
                self.rollback(api, &d.group, before.clone()).await;
                return Err(map_api_err(e));
            }
        };
        if after.as_deref() != Some(d.to_node.as_str()) {
            tracing::warn!(
                "[switch] verify mismatch group={} want={} got={:?} rollback=1",
                d.group, d.to_node, after
            );
            self.rollback(api, &d.group, before).await;
            return Err(AppError::new("E_CORE_START_FAILED", "切换未生效，已回滚").retryable(true));
        }

        // ④ 回写历史（自动切换**不动**手动覆盖期，见 C.5 v1.2）
        {
            let mut h = self.history.lock().await;
            let st = h.entry(d.group.clone()).or_default();
            st.last_switch_at = Some(now);
            st.consecutive_failures = 0;
            // 覆盖期只由「到期」或「用户再次手动选择」改变，自动切换成功不解除。
            // 实际上覆盖期内根本进不到本函数，这里是显式声明契约、防止后续误改。
        }

        // ⑤ 落盘 + 事件（顺序：先落盘再发事件，保证前端看到的一定已持久化）
        let record = SwitchRecord {
            at: now,
            group: d.group.clone(),
            from_node: d.from_node.clone(),
            to_node: d.to_node.clone(),
            from_delay: d.from_delay,
            to_delay: d.to_delay,
            reason: d.reason.clone(),
        };
        self.push_record(record.clone()).await?;

        tracing::info!(
            "[switch] applied group={} {}->{} {:?}ms->{:?}ms reason={:?}",
            record.group,
            record.from_node.as_deref().unwrap_or("<none>"),
            record.to_node,
            record.from_delay,
            record.to_delay,
            record.reason
        );
        events.emit("switch:applied", &record);
        Ok(record)
    }

    /// 回滚：PUT 回原节点。回滚失败只记 Error，不再递归重试（避免死循环）。
    async fn rollback(&self, api: &CoreApi, group: &str, before: Option<String>) {
        let Some(prev) = before else { return };
        match api.set_group_selected(group, &prev).await {
            Ok(()) => tracing::warn!("[switch] rolled back group={} to={}", group, prev),
            Err(e) => tracing::error!(
                "[switch] rollback FAILED group={} to={} err={:?} manual_intervention=1",
                group, prev, e
            ),
        }
    }
}
```

### 7.6.3 失败计数维护（`decide_on_failure` 的调用点）

```rust
impl Selector {
    /// 每轮测速完成后调用一次：对每个参与同步的组，
    /// 若当前节点本轮失败则累加计数，否则清零。
    ///
    /// **必须在 `apply_best` 之前执行**，否则 failure_threshold 永远差一轮攒不满。
    /// 数据源用 `ProbeSummary` 而不是 `SpeedtestRun.results`：
    /// 后者含被排除项，会把「用户手选的 DIRECT」误判成失败并疯狂累加。
    pub async fn update_failure_counters(
        &self,
        api: &CoreApi,
        policy_cfg: &SwitchPolicy,
        summary: &ProbeSummary,
    ) -> Result<(), AppError> {
        let groups = api.proxies_selectable().await.map_err(map_api_err)?;
        let mut h = self.history.lock().await;
        for view in groups.iter().filter(|g| policy::should_sync_group(policy_cfg, g)) {
            let Some(cur) = view.now.as_deref() else { continue };
            // 用户在组里手选了内置策略 / 信息节点 → 不是「失败」，不累加
            if summary.is_excluded(cur) {
                continue;
            }
            let st = h.entry(view.name.clone()).or_default();
            let ok = summary.rank_of(cur).is_some();
            if ok {
                if st.consecutive_failures > 0 {
                    tracing::info!("[switch] recovered group={} node={} fails_reset=1", view.name, cur);
                }
                st.consecutive_failures = 0;
            } else {
                st.consecutive_failures = st.consecutive_failures.saturating_add(1);
                tracing::info!(
                    "[switch] failure_count group={} node={} fails={}/{}",
                    view.name, cur, st.consecutive_failures, policy_cfg.failure_threshold
                );
            }
        }
        Ok(())
    }
}
```

### 7.6.4 手动切换（`switch_apply_node`）

```rust
/// IPC `switch_apply_node { group, node }` 的实现。
/// **不再做策略判定**：用户说什么就是什么（唯一约束是节点必须存在于该组 all 里）。
/// 成功后写入 `SwitchRuntimeLocks`，进入 `manual_override_min` 分钟的覆盖期（C.5 v1.2）。
pub async fn apply_node_manual(
    &self,
    api: &CoreApi,
    policy_cfg: &SwitchPolicy,
    group: &str,
    node: &str,
    summary: &ProbeSummary,
    events: crate::event::EventSink,
    now: i64,
) -> Result<SwitchRecord, AppError> {
    let _guard = match self.apply_lock.try_lock() {
        Ok(g) => g,
        Err(_) => return Err(AppError::new("E_BUSY", "正在执行切换，请稍候").retryable(true)),
    };

    let view = api.proxy_group(group).await.map_err(map_api_err)?;
    if !view.all.iter().any(|n| n == node) {
        return Err(AppError::new(
            "E_UNSUPPORTED",
            format!("组 {group} 中不存在节点 {node}"),
        ).retryable(false));
    }
    let before = view.now.clone();
    // 延迟只用本轮实测值（summary 里现成的），取不到就是 None（C.5 v1.2 允许）
    let before_delay = before.as_deref().and_then(|n| summary.rank_of(n)).map(|r| r.delay_ms);
    let to_delay = summary.rank_of(node).map(|r| r.delay_ms);

    api.set_group_selected(group, node).await.map_err(map_api_err)?;

    // 校验
    let after = api.proxy_group(group).await.map_err(map_api_err)?.now;
    if after.as_deref() != Some(node) {
        self.rollback(api, group, before.clone()).await;
        return Err(AppError::new("E_CORE_START_FAILED", "手动切换未生效，已回滚").retryable(true));
    }

    // ① 写手动覆盖状态（C.5 v1.2）：仅内存、不落盘、不进 RuntimeState
    //    同一组再次手动选择 → 续期（覆盖 locked_at 为最新时刻）
    {
        let mut locks = self.locks.lock().await;
        let entry = locks
            .entry(group.to_string())
            .or_insert_with(|| SwitchRuntimeLocks { group: group.to_string(), locked_at: now });
        entry.renew(now);
    }
    // ② 手动切换同时刷新自动切换历史（min_dwell 也从此刻起算）
    {
        let mut h = self.history.lock().await;
        let st = h.entry(group.to_string()).or_default();
        st.last_switch_at = Some(now);
        st.consecutive_failures = 0;
    }

    let record = SwitchRecord {
        at: now,
        group: group.to_string(),
        from_node: before,
        to_node: node.to_string(),
        from_delay: before_delay,
        // 手动切换没有新探测：有本轮延迟就带上，没有就 None（**不得用 0 冒充**）
        to_delay,
        reason: SwitchReason::Manual,
    };
    self.push_record(record.clone()).await?;
    tracing::info!(
        "[switch] manual group={} ->{} hold_min={}",
        group, node, policy_cfg.manual_override_min
    );
    events.emit("switch:applied", &record);
    Ok(record)
}
```

> **`to_delay` 在手动切换时取什么？** C.5 v1.2 已将其改为 `Option<u32>`。
> 手动切换没有新探测：目标节点在本轮结果里有延迟就填 `Some(ms)`，没有就填 `None`。
> **绝不用 `0` 冒充** —— `0ms` 在 UI 上会被读成「极快」，而 `None` 正确表达「无新鲜数据」，
> 前端显示 `—`。

---

## 7.7 多 selector 组同步策略

实际订阅里常见这样的结构：

```yaml
proxy-groups:
  - name: "🚀 节点选择"     # type: select（主组，用户可见）
  - name: "♻️ 自动选择"     # 生成时改为 select
  - name: "🌍 国外媒体"     # type: select
  - name: "📲 电报消息"     # type: select
```

「切到最快节点」在语义上意味着**所有语义为「选择节点」的组都应该一致**，
否则用户会发现「节点选择」切了但「媒体」没切。

| 策略 | 行为 | 适用 |
| --- | --- | --- |
| `SwitchScope::Group(G)` | **只 PUT 名字等于 G 的组** | 用户想精确控制某一个组 |
| `SwitchScope::Global` | PUT 所有 `type == "Selector"` 的组 | 默认推荐 |

**不参与同步的组（即使 type 是 Selector）**

| 组名/特征 | 原因 |
| --- | --- |
| 含「直连」「DIRECT」「拒绝」「REJECT」「拦截」 | 语义是分流策略而非节点选择，切了会改路由 |
| `all` 长度 ≤ 1 | 无可选项 |
| 组名命中 `exclude_keywords` | 与信息节点同源，通常是机场的展示组 |
| `type` 非 `Selector`（URLTest/Fallback/LoadBalance） | 内核自有调度，PUT 会被忽略或引发冲突 |

```rust
/// 纯函数：给定「可切换组」与策略 → 是否应参与自动切换（可单测）
///
/// 注意入参是 `SelectableGroup`（`proxies_selectable` 的轻量结果），**不需要** `GroupInfo`：
/// 外层过滤只看「组名 + 成员数 + 组类型」，这些 `SelectableGroup` 里都有
/// （`proxies_selectable` 按 C.6.1 只返回 `kind == "Selector"` 的组，故类型判断天然成立）。
/// 需要完整 `all` / `test_url` 的判定留给内层 `decide`，避免每个组都先打一次 `GET /proxies/{group}`。
pub fn should_sync_group(policy: &SwitchPolicy, view: &SelectableGroup) -> bool {
    if view.members.len() <= 1 {
        return false;
    }
    let lower = view.name.to_lowercase();
    // 语义排除：分流/拦截类组
    const DENY: [&str; 6] = ["direct", "reject", "直连", "拒绝", "拦截", "广告"];
    if DENY.iter().any(|k| lower.contains(k)) {
        return false;
    }
    match &policy.scope {
        SwitchScope::Global => true,
        SwitchScope::Group(g) => g == &view.name,
    }
}
```

> **`should_sync_group` 不再调 `is_selector()`**：因为 C.6.1 规定 `proxies_selectable` 只返回
> `kind == "Selector"` 的组，类型判断在 08 章已做过一次。本章的 `decide` 里仍保留 `is_selector()`
> 作为**兜底**（万一 08 章实现漏过滤，或未来 `proxy_group` 返回的是 URLTest 组），
> 这属于「不信任上游但也不重复开销」的取舍。

> 两者关系：`should_sync_group` 是**外层过滤**（哪些组进入决策），`policy::decide` 是**内层判定**
> （该组要不要切）。**不要**在 `policy::decide` 里重复写 scope 比较后就用它当外层过滤器——
> `decide` 只管 scope 与 `is_selector`，不处理「分流组语义排除」「单选项组」。
> 正确用法：外层 `filter(should_sync_group)` → 内层 `decide(...)`。这是易错点，写在这里防止后续改错。

---

## 7.8 历史落盘与保留

| 项 | 约定 |
| --- | --- |
| 文件 | `%APPDATA%\ClashZhengl\switch_history.json`（Roaming，与 `profiles/` 同级） |
| 格式 | `{ "schema_version": 1, "records": [SwitchRecord, ...] }` |
| 保留 | 最近 **200** 条，超出丢弃最旧 |
| 写入 | 原子写（`.tmp` → fsync → rename，C.3） |
| 频率 | 每次成功切换写一次；`min_dwell_sec=300` 保证最坏 12 次/小时，磁盘压力可忽略 |
| 损坏 | 重命名 `*.corrupt-<ts>`，重置为空数组，UI 不报错（C.3） |

```rust
impl Selector {
    async fn push_record(&self, rec: SwitchRecord) -> Result<(), AppError> {
        let mut buf = self.records.lock().await;
        buf.push_front(rec);                 // 新在前
        while buf.len() > HISTORY_LIMIT {
            buf.pop_back();                  // 丢最旧
        }
        let snapshot: Vec<SwitchRecord> = buf.iter().cloned().collect();
        drop(buf);
        // 落盘失败不得阻断切换：切换已生效，日志告警即可
        if let Err(e) = crate::store::json_store::write_switch_history(&snapshot) {
            tracing::warn!("[switch] persist history failed code={} (switch still applied)", e.code);
        }
        Ok(())
    }

    pub async fn recent(&self, limit: usize) -> Vec<SwitchRecord> {
        let buf = self.records.lock().await;
        buf.iter().take(limit.min(HISTORY_LIMIT)).cloned().collect()
    }
}
```

> **为什么落盘失败不报错给用户**：切换已经生效（`PUT` 成功 + 校验通过），
> 历史只是诊断信息。把 `E_STORE_IO` 弹给用户会造成「切成功了却说失败」的困惑。

---

## 7.9 可解释日志样例

正常切换（自动）：

```
2026-11-06T16:02:04.881 INFO  [speedtest] done total=51 ok=48 best=HK-01 delay=42ms elapsed=1776ms
2026-11-06T16:02:04.902 INFO  [switch] failure_count group=🚀 节点选择 node=JP-02 fails=1/2
2026-11-06T16:02:04.903 INFO  [switch] hold group=🚀 节点选择 cur="JP-02" cur_delay=Some(96) best=HK-01 best_delay=Some(42) gain=54ms need=30ms suppress=MinDwell
2026-11-06T16:07:05.010 INFO  [switch] applied group=🚀 节点选择 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
2026-11-06T16:07:05.011 INFO  [switch] applied group=🌍 国外媒体 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
2026-11-06T16:07:05.012 INFO  [switch] applied group=📲 电报消息 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
```

无操作（防抖抑制，最重要的一类日志——证明「没瞎切」）：

```
2026-11-06T16:12:04.900 INFO  [switch] hold group=🚀 节点选择 cur="HK-01" cur_delay=Some(42) best=HK-02 best_delay=Some(39) gain=3ms need=30ms suppress=BelowTolerance
```

当前节点失效降级：

```
2026-11-06T16:17:04.900 INFO  [switch] failure_count group=🚀 节点选择 node=HK-01 fails=1/2
2026-11-06T16:22:04.900 INFO  [switch] failure_count group=🚀 节点选择 node=HK-01 fails=2/2
2026-11-06T16:22:04.901 INFO  [switch] applied group=🚀 节点选择 HK-01->HK-02 None->Some(39)ms reason=CurrentFailed
```

当前节点消失（订阅换了节点池）：

```
2026-11-06T16:27:04.900 INFO  [switch] applied group=🚀 节点选择 HK-03->SG-01 Some(55)ms->Some(61)ms reason=GroupMissing
```

切换失败并回滚：

```
2026-11-06T16:32:04.900 WARN  [switch] verify mismatch group=🚀 节点选择 want=HK-01 got=Some("JP-02") rollback=1
2026-11-06T16:32:04.905 WARN  [switch] rolled back group=🚀 节点选择 to=JP-02
2026-11-06T16:32:04.906 WARN  [switch] apply failed group=🚀 节点选择 target=HK-01 code=E_CORE_START_FAILED
```

手动覆盖期内（C.5 v1.2 `manual_override_min`，用户刚手选过，自动策略让路）：

```
2026-11-06T16:37:04.900 INFO  [switch] manual group=🚀 节点选择 ->US-09 hold_min=60
2026-11-06T16:42:04.900 INFO  [switch] hold group=🚀 节点选择 cur="US-09" cur_delay=Some(210) best=HK-01 best_delay=Some(42) gain=168ms need=30ms suppress=ManualOverride
```

覆盖期到期后恢复参与自动切换（同一组，60 分钟后）：

```
2026-11-06T17:37:04.900 INFO  [switch] applied group=🚀 节点选择 US-09->HK-01 Some(210)ms->Some(42)ms reason=Best
```

v1.2 返回体（`switch_apply_best` 多组生效）：

```
2026-11-06T16:07:05.010 INFO  [switch] applied group=🚀 节点选择 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
2026-11-06T16:07:05.011 INFO  [switch] applied group=🌍 国外媒体 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
2026-11-06T16:07:05.012 INFO  [switch] applied group=📲 电报消息 JP-02->HK-01 Some(96)ms->Some(42)ms reason=Best
2026-11-06T16:07:05.020 INFO  [ipc] switch_apply_best primary=🚀 节点选择 all=3
```

---

## 7.10 边界用例表（≥15 行，全部可单测）

> `decide` / `decide_on_failure` / `should_sync_group` 是纯函数，测试文件
> `src-tauri/tests/policy_decide.rs`，**不需要 tokio、不需要网络**。
> `now` 全部显式传入，无隐藏时钟依赖。

| # | 场景 | 输入（`ProbeSummary` + `GroupInfo` + `GroupHistory` + `SwitchRuntimeLocks`） | 期望 | 断言 |
| --- | --- | --- | --- | --- |
| 1 | 已是最优 | cur=HK-01(42)，ranked[0]=HK-01(42) | `suppressed_by=BelowTolerance` | 不等 |
| 2 | 优势不足 | cur=HK-01(50)，best=JP-02(30)，tol=30 | `BelowTolerance`（gain=20<30） | 不等 |
| 3 | 优势恰好等于 tolerance | cur=HK-01(72)，best=JP-02(42)，tol=30 | 切（gain=30 ≥ 30） | `suppressed_by=None` |
| 4 | 优势够但驻留不足 | 同上，`last_switch_at=now-100`，dwell=300 | `MinDwell` | 不等 |
| 5 | 优势够且驻留够 | 同上，`last_switch_at=now-301` | 切，`reason=Best` | `Decision.to_node=="JP-02"` |
| 6 | 并发测速 | 同一 runId 同时调两次 `apply_best` | 第二次 `E_BUSY` | `err.code=="E_BUSY"` |
| 7 | 测速未完成 | `run.finished_at=None` | `E_BUSY` | `put_count==0` |
| 8 | runId 不匹配 | 传旧 runId | `E_BUSY` | `put_count==0` |
| 9 | 当前节点消失 | cur=HK-03，`view.all` 无 HK-03，ranked 非空 | 立即切，`reason=GroupMissing` | 不受 `min_dwell` 影响 |
| 10 | 全部失败 | `summary.failed` 含全部，`ranked=[]` | `decide` 返回 `None` | `apply_best` 返回 `Ok(None)` |
| 11 | 全被排除 | 所有节点名含「剩余流量」，`excluded` 含全部 | `ranked=[]` → `None` | 不切、不报错 |
| 12 | 延迟并列 | HK-01=42, HK-02=42, HK-03=42 | best=HK-01（名称字节序） | 两次调用结果一致 |
| 13 | 延迟并列 + 地区偏好 | JP=42, HK=42, priority=["HK"] | best=HK | 地区后置 tiebreak |
| 14 | **覆盖期内跳过** | `lock=Some{locked_at=now-60}`，ovr=60 | `suppressed_by=ManualOverride` | `put_count==0` |
| 15 | 覆盖期到期 | `lock=Some{locked_at=now-3600}`，ovr=60 | 覆盖失效，正常参与决策 | `suppressed_by != ManualOverride` |
| 16 | `ovr=0` 立即失效 | `lock=Some{locked_at=now}`，ovr=0 | 不等，正常切换 | `is_active()==false` |
| 17 | 覆盖期 + 节点失败 | 覆盖期内 cur 在 `failed` 里 | 仍跳过（覆盖期不判断存活） | `ManualOverride`，`put_count==0` |
| 18 | 再次手动选择续期 | `locked_at=now-3000`（已过期），手动选一次 | `locked_at` 更新为 `now`，重新进入覆盖 | `is_active(now,60)==true` |
| 19 | 失败未达阈值 | `history.consecutive_failures=1`，ft=2 | `decide_on_failure` → `FailureThreshold` | 不等 |
| 20 | 失败达阈值 | fails=2/2，elapsed≥dwell | 切，`reason=CurrentFailed` | `suppressed_by=None` |
| 21 | 失败达阈值但驻留不足 | fails=2/2，elapsed=10 | `MinDwell` | 不等 |
| 22 | 失败后恢复 | fails=1 后本轮成功 | `consecutive_failures` 归零 | 计数器=0 |
| 23 | 非 Selector 组 | `GroupInfo.kind="URLTest"`（违反 C.5 组类型约束） | `decide` 返回 `None` | `put_count==0` |
| 24 | 单选项组 | `SelectableGroup.members.len()==1` | `should_sync_group==false` | 不参与 |
| 25 | 分流组语义排除 | 组名「广告拦截」 | `should_sync_group==false` | 不参与 |
| 26 | scope=Group 命中 | scope=Group("🚀 节点选择")，组名相同 | 参与 | `should_sync_group==true` |
| 27 | scope=Group 不命中 | scope=Group("A")，组名 "B" | 不参与 | `false` |
| 28 | 无当前节点 | `view.now=None` | `reason=GroupMissing`，切 best | `from_node=None` |
| 29 | policy 关闭 | `enabled=false` | `decide` 返回 `None` | `apply_best` 返回 `Ok(None)` |
| 30 | 回滚路径 | FakeApi：PUT 成功但 GET 返回旧值 | `E_CORE_START_FAILED` | `put_count==2`（正向 + 回滚） |
| 31 | 历史裁剪 | 连写 250 条 | `recent(1000).len()==200` | 最旧的 50 条被丢 |
| 32 | 历史落盘失败 | Store 注入 IO 错误 | 切换仍成功，仅 Warn | 返回 `Ok` |
| 33 | min_dwell 边界 | elapsed 恰为 300 | 允许切（`< dwell` 才拦） | `suppressed_by=None` |
| 34 | 覆盖期边界 | `lock_age` 恰为 `ovr × 60` | 覆盖失效（`< window` 才拦） | `is_active()==false` |
| 35 | 当前节点是信息节点 | cur 在 `excluded`，ranked 非空 | `CurrentExcluded`，**不切** | `put_count==0` |
| 36 | 当前节点手选 DIRECT | cur="DIRECT"，在 `excluded` 且不在 `all` | `CurrentExcluded`，不切 | `put_count==0` |
| 37 | 排除优先于失败 | 某节点名含「流量」且本轮超时 | 只进 `excluded`，不进 `failed` | `build_summary` 单测：两桶不相交 |
| 38 | 单组失败不影响它组 | 3 组，第 2 组 PUT 返回 500 | 第 1/3 组仍切换 | `all.len()==2` |
| 39 | 手动切换节点不存在 | `node="不存在"` | `E_UNSUPPORTED` | `put_count==0` |
| 40 | **v1.2 返回体** | 3 组全部生效 | `primary` 是首条，`all.len()==3` | `primary==all[0]` |
| 41 | **v1.2 全无生效** | 全部被抑制 | `apply_best` 返回 `Ok(None)` | 不构造空 `primary` |
| 42 | **Manual 的 to_delay** | 手动选未测速节点 | `SwitchRecord.to_delay == None` | 不等于 `Some(0)` |

**FakeCoreApi（selector 测试用）**

```rust
// 测试代码允许 unwrap（C.13 只约束生产路径）
struct FakeApi {
    /// 单组完整状态（GroupInfo 是契约类型，08 章定义，测试直接用）
    groups: std::sync::Mutex<std::collections::HashMap<String, GroupInfo>>,
    /// 注入：PUT 后 GET 返回什么（None = 正常回写，Some(x) = 强制返回 x，用于测回滚）
    verify_override: std::sync::Mutex<Option<String>>,
    put_count: std::sync::atomic::AtomicUsize,
}

/// §7.10 全部用例的构造夹具。
/// **必须存在**：`GroupInfo` 有 6 个字段（`test_url` / `hidden` 是 v1.4 按 C.6.1 补的），
/// 每条用例都手写结构体字面量会啰嗦且容易漏字段（漏了就是 `E0063: missing fields`）。
/// `test_url` / `hidden` 在 07 章的判定里**不参与任何决策**，故固定给 `None` / `false`；
/// 需要测「隐藏组」语义时才用 `info_hidden()` 变体。
fn info(name: &str, kind: &str, now: Option<&str>, all: &[&str]) -> GroupInfo {
    GroupInfo {
        name: name.to_string(),
        kind: kind.to_string(),
        now: now.map(|s| s.to_string()),
        all: all.iter().map(|s| s.to_string()).collect(),
        test_url: None,
        hidden: false,
    }
}

/// 隐藏组变体（08 章 `proxies_selectable` 已按 `!hidden` 过滤，这个夹具用于
/// 验证「若 08 章实现漏过滤，07 章是否仍正确」这类防御性用例）
fn info_hidden(name: &str) -> GroupInfo {
    GroupInfo { hidden: true, ..info(name, "Selector", None, &[]) }
}

// boxed-future 形式：每个方法返回 ApiFuture，函数体用 Box::pin(async move { … })
impl SelectorApi for FakeApi {
    /// 只有 kind == "Selector" 的组才会被返回（复刻 C.6.1 的过滤语义）
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> {
        Box::pin(async move {
            Ok(self
                .groups
                .lock()
                .unwrap()
                .values()
                .filter(|g| g.kind.eq_ignore_ascii_case("Selector"))
                .map(|g| SelectableGroup {
                    name: g.name.clone(),
                    members: g.all.clone(),
                    now: g.now.clone(),
                })
                .collect())
        })
    }
    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()> {
        Box::pin(async move {
            self.put_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut g = self.groups.lock().unwrap();
            let v = g
                .get_mut(group)
                .ok_or_else(|| ApiError::NotFound(group.to_string()))?;
            v.now = Some(node.to_string());
            Ok(())
        })
    }
    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo> {
        Box::pin(async move {
            let mut g = self.groups.lock().unwrap();
            let v = g
                .get_mut(group)
                .ok_or_else(|| ApiError::NotFound(group.to_string()))?;
            if let Some(forced) = self.verify_override.lock().unwrap().clone() {
                v.now = Some(forced);
            }
            Ok(v.clone())
        })
    }
}
```

> **`FakeApi` 的 `NotFound` 用法**：C.6.1 的 `NotFound(String)` 携带的是**资源标识**。
> 测试里传组名（`NotFound(group.to_string())`），生产代码里 08 章传 URL path
> （见 08 §08.6.3 的 `map_reqwest`）。两者都是「哪个东西没找到」的字符串，
> 07 章只用 `map_api_err` 把它统一映射成 `E_CORE_START_FAILED`，不解析内容。
>
> **`ApiFuture` 的 lifetime 细节**：`Box::pin(async move { … })` 里若有 `&self` 的借用
> （如上面三个方法都读 `self.groups`），编译器会要求返回的 future 活不过 `&self` 的借期。
> `ApiFuture<'_, T>` 的 `'_` 由**方法签名里的 `&self`/`&str` 参数共同决定**，
> 因此 `fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>` 能正确捕获两个借期，
> 不需要额外标注。这也是为什么类型别名必须写 `ApiFuture<'_, T>` 而非 `ApiFuture<'static, T>`。

---

## 7.11 与其它章节的边界

| 对端 | 职责划分 |
| --- | --- |
| 06 章 测速引擎 | 只负责「测出延迟」与 `rank_candidates`；**不**做切换决策。测速完成后（`trigger ≠ Manual`）直接回调本章 `after_speedtest()` |
| 04 章 合并 | 负责把 `url-test` 组改写为 `select`（C.5「组类型约束」；否则内核自带健康检查与本应用双引擎打架，见 06 §6.9） |
| 05 章 调度器 | 只决定「什么时候测速」；切换由测速引擎回调本章，不经过调度器 |
| 08 章 内核 | 提供 `CoreApi`，实现 `SelectorApi` 三方法（`proxies_selectable` / `proxy_group` / `set_group_selected`）与 `ApiError` 五变体（C.6.1）。**签名见 §7.16 冻结表 #1–#6，不得单方面修改** |
| 09 章 UI | 消费 `switch:applied` 事件与 `recent(200)` 历史，展示「切换记录」列表；`to_delay == None` 显示 `—` |
| C.6 IPC | `switch_apply_best{runId}` → `{primary: SwitchRecord, all: Vec<SwitchRecord>}`（v1.2 定稿） |

> **`switch_apply_best` 的返回与事件分工**（C.6 v1.2）：
> - **invoke 返回** `{primary, all}`：给「点一下就切」这种一次性调用的调用方，
>   它拿到的是**已持久化**的结果，可直接渲染结果 Toast（「已切换到 HK-01（3 个组）」）。
> - **事件 `switch:applied`**：仍然**逐组**广播，供 UI 做增量更新（组卡片高亮、切换记录列表插入）。
>
> 两者不冲突：事件是「流」，返回是「这一次调用的账」。前端**不要**用事件去拼 invoke 的返回值
> （事件可能先于 invoke 返回到达，也可能因为窗口刚打开而漏掉前几条）。

---

## 7.12 契约对齐记录（v1.2 + v1.3 + v1.4）

> C 章 v1.2 裁决了本章提出的 3 条缺口；v1.3 新增 §C.6.1 内部接口冻结面，
> 本章据此完成 `ApiError` 与组结构的两处编译级收敛。以下逐条记录「改了什么 → 对应 C 章哪节」。
> **无遗留未决项**；本章不再有 `CONTRACT-CHANGE-REQUEST:`。

| # | 变更点 | 本章改动 | C 章依据 |
| --- | --- | --- | --- |
| 1 | `switch_apply_best` 返回值 | 新增 `ApplyBestResult { primary, all }`；`apply_best`/`after_speedtest` 返回 `Result<Option<ApplyBestResult>>`（全无生效时 `Ok(None)`，不构造空 `primary`）；§7.11 重写返回与事件分工 | C.6 `switch_apply_best` 行 |
| 2 | `SwitchRecord.to_delay` 可空 | `SwitchRecord.to_delay`、`Decision.to_delay` 均改 `Option<u32>`；Manual 时取本轮实测（`Some`）或无数据（`None`），**删除「填 0」方案**；日志格式串改 `{:?}`；§7.10 新增用例 42 | C.5 `SwitchRecord.to_delay` 注释 |
| 3 | 手动覆盖语义 | `GroupHistory` 拆出 `SwitchRuntimeLocks { group, locked_at }` 独立结构；`Selector` 新增 `locks: Mutex<HashMap<String, SwitchRuntimeLocks>>`（**仅内存、不落盘、不进 `RuntimeState`**）；新增 `is_active(now, manual_override_min)` / `renew(now)` 纯函数；`Suppress::UserLocked` → `Suppress::ManualOverride`；`decide`/`decide_on_failure` 增加 `lock: Option<&SwitchRuntimeLocks>` 入参；判定顺序新增 ⓪ 覆盖期最高优先；§7.10 新增用例 14–18、34 | C.5 `SwitchPolicy.manual_override_min` + 「手动覆盖语义」段 |
| 4 | 组类型约束 | §7.1 新增 A9 与 C.5 组类型约束引用；`is_selector()` 作为兜底 | C.5「合并配置的组类型约束」 |
| 5 | **`ApiError` 收敛到契约五变体** | `Status(401)`→`Unauthorized`；`Status(404)`→`NotFound(String)`；`Connect`→`Transport(String)`；`Decode`→**统一落在 `Status(200, _)`**（理由见 §7.6 末段：解析失败是「连接成功但内容不对」，排障方向与 `Transport` 相反）；`Other(String)`→`Transport(String)`。`map_api_err` 全量重写，`Timeout` 归 `E_NET_TIMEOUT`（照抄 08 §08.17.1 的裁决） | C.6.1 `enum ApiError` |
| 6 | **`GroupView` 拆为两类型** | 删除自建 `GroupView`；`proxies_selectable`→`Vec<SelectableGroup>`、`proxy_group`→`GroupInfo`；`decide`/`decide_on_failure` 入参改 `&GroupInfo`，`should_sync_group` 入参改 `&SelectableGroup`；字段名 `group`→`name`、`group_type`→`kind`、`all`→`members`（按类型而定）；便利方法走 `GroupInfoExt` / `SelectableGroupExt` 扩展 trait，**不碰字段**；`FakeApi` 同步改写 | C.6.1 `struct GroupInfo` / `struct SelectableGroup` |
| 7 | **跨章接口冻结表** | 新增 §7.16：26 条签名 + `SchedulerCmd` 完整定义 + 冻结规则 + 与 08 章对齐确认；冻结版本 `v1.3-freeze-1`（v1.4 起 `SelectorApi` 相关三条升至 `v1.4-freeze-1`） | C.6.1「明细表见 07 章 §7.16」 |
| 8 | **`SelectorApi` 改 boxed-future（v1.4）** | 三方法由 `async fn → Result<T, ApiError>` 改为 `fn → ApiFuture<'_, T>`；新增 `type ApiFuture<'a, T> = Pin<Box<dyn Future<Output=Result<T, ApiError>> + Send + 'a>>` 并作为冻结表 **#26** 单列；§7.6.1 trait 定义、§7.10 `FakeApi` 三方法（改 `Box::pin(async move { … })`）同步改写；**移除 `async_trait` 宏** | C.6.1「`SelectorApi` 的 dyn 兼容形式」 |
| 9 | **组类型约束依据指向附录 A.6.4（v1.4）** | §7.1 A9 下补引 [附录 A §A.6.4](appendices/A-mihomo-api-and-config.md) 的错误写法对照表，**不重复展开**三种错误写法；只补「为什么这对本章致命」——本章三件套（`min_dwell` / `tolerance` / 覆盖期）全部假设「只有我们在改 `now`」 | C.6.1 + 附录 A.6.4 |

### 7.12.1 已撤销的 CHANGE-REQUEST（历史存档）

<details>
<summary>点击展开：v1.1 提出、v1.2 已采纳的 3 条原始请求</summary>

```text
CONTRACT-CHANGE-REQUEST: switch_apply_best 返回值
- 现状：C.6 定义返回单个 SwitchRecord；但 scope=Global 时可能切换多个组。
- 影响：前端无法从 invoke 返回值得知全部切换结果，只能依赖事件（存在竞态，
        事件可能在 invoke 返回前到达）。
- 建议：改为返回 { primary: SwitchRecord, all: SwitchRecord[] }，或 Vec<SwitchRecord>。
- 裁决（v1.2）：采纳建议 → C.6 定为 { primary: SwitchRecord, all: Vec<SwitchRecord> }。
```

```text
CONTRACT-CHANGE-REQUEST: SwitchRecord.to_delay 的可选性
- 现状：to_delay: u32（必填）；reason=Manual 时没有新测速数据。
- 影响：只能填 0 或「最近一次已知延迟」，0ms 有歧义。
- 建议：改为 Option<u32>（与 from_delay 对称）。
- 裁决（v1.2）：采纳 → C.5 改为 Option<u32>，注释明确「不得用 0 冒充」。
```

```text
CONTRACT-CHANGE-REQUEST: 用户手动锁定的表达
- 现状：C.5 无 "locked" 字段；C.6 switch_apply_node 只是「手动指定节点」。
- 影响：无法表达「用户手动指定后，自动切换是否还能覆盖」。
- 建议：在 SwitchPolicy 增加 lock_until_manual_change: bool（默认 true）。
- 裁决（v1.2）：未采纳本建议字段名，改为更精确的方案：
        新增 SwitchPolicy.manual_override_min: u32（默认 60，0 = 不暂停），
        并定义 SwitchRuntimeLocks { group, locked_at } 内存态结构。
        ——比 bool 更好：bool 只能表达「永久锁 / 不锁」，无法表达「锁多久」，
        而 60 分钟覆盖期既尊重用户意图，又不会让一个已劣化的节点被永久钉死。
```

</details>

### 7.12.2 与 v1.2 的一处主动收紧（非契约变更，仅实现决策）

`apply_best` 在「所有组都被抑制」时返回 `Ok(None)` 而非 `Ok(Some(空 primary))`。
理由：`primary` 在类型上非可选，构造一个假记录（例如 `to_node` 等于 `from_node`）
会污染 `switch_history.json` 并让 UI 显示一条「切换了但没切」的记录。
用 `Option` 表达「本次没有任何切换」是唯一诚实的做法。

> 若后续认为 IPC 层不便处理 `Option`，可在 `commands/speedtest.rs` 的薄封装里
> 保持 `Option` → 返回 `null`（前端已有 `null` 处理路径），仍不构造假记录。

---

## 7.13 验收清单

| 项 | 方法 | 通过标准 |
| --- | --- | --- |
| 纯函数单测 | `cargo test --test policy_decide` | 42 条全绿 |
| Clippy | `cargo clippy --all-targets -- -D warnings` | 无警告 |
| 无 `unwrap` | `rg "unwrap\(\)\|expect\(" src/speedtest/selector.rs src/domain/policy.rs` | 仅测试代码中允许 |
| 防抖真机 | 30 分钟测速周期，观察 2 小时 | 日志中 `hold ... BelowTolerance` 占多数，`applied` ≤ 4 次 |
| 回滚真机 | 手工把内核 API 指向错误端口后触发切换 | 有 `rolled back` 日志，组内节点不变 |
| 历史保留 | 连续切 210 次（脚本） | `switch_history.json` 中 `records.len()==200` |
| 可解释性 | 随机抽 10 条 `SwitchRecord` | 每条都能在日志里找到对应 `[switch] applied` 行 |
| 多组同步 | 4 个 selector 组的订阅，触发一次切换 | 4 条 `switch:applied`，`all.len()==4`，节点一致 |
| 手动覆盖 | 手动选 US-09，跑 3 轮测速（间隔 < 60min） | 3 条 `suppress=ManualOverride`，`put_count==0` |
| 覆盖期到期 | 把 `manual_override_min` 临时设为 1，等 90s | 覆盖失效后下一轮可被自动切换 |
| `ovr=0` | 设 `manual_override_min=0` 后手动选节点再测速 | 立刻可被自动切换覆盖（无 `ManualOverride` 抑制） |
| 返回体 | 3 组全部切换 | invoke 返回 `primary` + `all.len()==3`，`primary.group == all[0].group` |
| 并发保护 | 前端连点两次「应用最优」 | 第二次返回 `E_BUSY`，无重复 PUT |

### 7.13.1 常见坑

| 坑 | 现象 | 对策 |
| --- | --- | --- |
| 先 PUT 再查当前节点 | 拿到的 `from_node` 已经是新节点，记录错乱 | 先快照 `proxies_selectable` 再 PUT（已按此实现） |
| 只比 `best` 不比 `cur` | 切到另一个同样慢的节点 | 用 `cur_delay - best_delay >= tol` 而非绝对值 |
| `failure_threshold` 用「累计」而非「连续」 | 一天里零星失败攒够就切，用户莫名其妙 | 用 `consecutive_failures`，成功即清零 |
| 覆盖期判断放在 `tolerance` 之后 | 用户刚手选完 A，30ms 优势就被自动切走 | 覆盖期判定必须是 ⓪ 最高优先级 |
| 覆盖期内判断「锁定节点是否存活」 | 锁定节点一失败就「聪明地」切走，违背用户意图 | 覆盖期只管时间窗，不看存活；到期自然恢复 |
| 把覆盖期落盘 | 用户手动选一次，重启后 60 分钟都不自动切，用户以为坏了 | `SwitchRuntimeLocks` 仅内存，见 C.5 |
| 把 `manual_override_min` 当 `min_dwell_sec` 用 | 语义错：一个尊重用户、一个防系统抖动 | 两个字段、两个状态、两个判定，互不替代 |
| 忘记清除覆盖期 | — | **不需要清除**：到期由 `is_active` 自然失效，避免清理逻辑漏写 |
| `to_delay` 用 `0` 冒充未知 | UI 显示「0ms」，用户以为极快 | 用 `None`，前端显示 `—`（C.5 v1.2） |
| 每轮都 PUT 同一个节点 | 内核日志刷屏，`profile.store-selected` 反复写盘 | `cur == best` 时短路，绝不 PUT |
| 用 `/group/{g}/delay` 后再比延迟 | 内核清空了固定选择，与本节 PUT 抢 | 只用 `/proxies/{name}/delay`（06 章） |
| 回滚递归 | 回滚失败又触发回滚 → 死循环 | 回滚只做一次，失败即 Error 日志 |
| 历史落盘阻塞切换 | 磁盘慢时用户点切换卡 2 秒 | 落盘在 `push_record` 内做，失败只 Warn；必要时改 `spawn_blocking` |
| 用事件拼 invoke 返回值 | 事件可能先到或漏收，UI 显示不一致 | 一次性操作读 `{primary, all}`，事件只做增量更新 |

---

## 7.14 本章交付物清单

| 文件 | 内容 | 行数量级 |
| --- | --- | --- |
| `src-tauri/src/domain/policy.rs` | `decide` / `decide_on_failure` / `should_sync_group` / `GroupInfoExt` / `SelectableGroupExt` / `GroupHistory` / `SwitchRuntimeLocks` / `ProbeSummary` / `Decision` / `Suppress` | ~380 |
| `src-tauri/src/speedtest/selector.rs` | `Selector`、`ApplyBestResult`、`apply_best`、`after_speedtest`、`apply_node_manual`、`execute_one`、`rollback`、`update_failure_counters`、`lock_of`、`push_record`、`recent` | ~460 |
| `src-tauri/src/commands/speedtest.rs`（增量） | `switch_apply_best` / `switch_apply_node` 两个命令的薄封装 | ~60 |
| `src-tauri/src/store/json_store.rs`（增量） | `write_switch_history` / `read_switch_history` | ~40 |
| `src-tauri/tests/policy_decide.rs` | §7.10 的 42 条用例 | ~450 |

> 所有 IPC 命令严格对应 C.6 表中 `switch_apply_best` / `switch_apply_node` 两行，
> 不新增命令、不新增事件（只用 `switch:applied` 与 `notify`）。

---

## 7.15 本章自检清单（Agent 必须逐条确认）

> 交付任何代码前逐条回答「是/否」，任一「否」不得进入下一章。

- [ ] 1. `SwitchPolicy` / `SwitchRecord` / `SwitchReason` / `SwitchScope` 的**字段名与类型**是否与 C.5 逐字一致？特别是 `to_delay: Option<u32>`、`manual_override_min: u32` 是否已按 v1.2 使用？
- [ ] 2. `switch_apply_best` 是否返回 `{primary, all}`、`switch_apply_node` 是否返回单条 `SwitchRecord`？（C.6 v1.2）
- [ ] 3. 事件是否只用 `switch:applied`（逐组）与 `notify`？有无自造 `switch:hold`、`switch:failed` 等未登记事件？（C.7）
- [ ] 4. 错误码是否只取自 C.8 清单（`E_BUSY` / `E_UNSUPPORTED` / `E_CORE_START_FAILED` / `E_CORE_API_UNAUTH` / `E_NET_TIMEOUT`）？有无自造错误码？
- [ ] 5. `domain/policy.rs` 是否**零 IO、零时钟、零 tokio**？`now` 是否全部由参数传入？能否在无 tokio 的测试中直接跑完 §7.10 的 42 条用例？
- [ ] 6. 三件套（`tolerance_ms` / `min_dwell_sec` / `failure_threshold`）的默认值与 C.5 是否一致（30 / 300 / 2）？
- [ ] 7. 判定顺序是否为 ⓪ 手动覆盖 → ① 已是最好 → ② 节点消失 → ③ 被排除 → ④ 失败 → ⑤ tolerance → ⑥ min_dwell？`min_dwell` 是否**没有**挡住「节点消失」与「失败达阈值」？
- [ ] 8. 是否**先 GET 快照再 PUT**？切换失败是否**回滚**到快照？回滚失败是否只记 Error 不递归？
- [ ] 9. `SwitchRuntimeLocks` 是否**仅内存**（不在 `switch_history.json`、不在 `RuntimeState`）？`manual_override_min == 0` 时是否立即失效？
- [ ] 10. 同一组再次手动选择是否**续期**（`locked_at` 更新为最新）而非叠加两条记录？
- [ ] 11. 自动切换成功后是否**不解除**覆盖期？（契约：覆盖期只由到期或再次手动改变）
- [ ] 12. `apply_best` 是否用 `try_lock` 而非 `lock`（并发时返回 `E_BUSY` 不排队）？`runId` 与 `finished_at` 是否在加锁**之前**校验？
- [ ] 13. `reason=Manual` 时 `to_delay` 是否用 `None`（而非 `0`）？`switch_history.json` 里是否可能出现 `to_delay: 0` 的伪记录？
- [ ] 14. 排序 tiebreak 是否按 `node_key` **字节序**兜底？同延迟节点是否**不会**每轮轮流被选？
- [ ] 15. `should_sync_group` 是否排除了分流/拦截组与单选项组（只看 `SelectableGroup.members.len()`，不依赖 `GroupInfo`）？`decide` 内是否**没有**重复实现这层过滤？
- [ ] 16. 历史是否裁剪到 200 条？落盘失败是否只 Warn 而**不阻断**切换成功？
- [ ] 17. 生产路径是否有 `unwrap()` / `expect()` / `panic!()`？（`cargo clippy -D warnings` + `rg` 双重检查）
- [ ] 18. 一次切换是否**逐组发出** `switch:applied`？`all.len()` 是否与实际生效组数一致？全无生效时是否返回 `Ok(None)` 而非伪造 `primary`？
- [ ] 19. 日志是否可解释：每条 `hold` 是否带 `cur/best/gain/need/suppress` 四要素？每条 `applied` 是否带 `from→to` 与延迟？
- [ ] 20. **`ApiError` 是否严格用 C.6.1 五变体**（`Unauthorized` / `NotFound(String)` / `Timeout` / `Status(u16, String)` / `Transport(String)`）？全文有无残留 `Status(401)` / `Connect` / `Decode` / `Other`？
- [ ] 21. **是否彻底没有自建 `GroupView`**？所有组数据是否用 `GroupInfo`（单组，`kind`/`all`/`test_url`/`hidden`）与 `SelectableGroup`（可切换组，`members`）？字段名是否为 `name`/`kind`/`members`（而非 `group`/`group_type`/`all`）？
- [ ] 22. `apply_best` 是否**只对通过 `should_sync_group` 的组**才调 `proxy_group`（避免 N+1 打满）？是否**没有**用 `members` 顶替 `all`（会误判「手选 DIRECT」为节点消失）？
- [ ] 23. `map_api_err` 是否让 `Timeout` → `E_NET_TIMEOUT`（**不是** `E_CORE_START_FAILED`）？`Decode` 失败是否统一落在 `Status(200, _)` 分支并归 `E_CORE_START_FAILED`？
- [ ] 24. 本章所有跨章签名是否与 **§7.16 冻结表**逐字一致？发现不一致时是否以冻结表为准回头改文档（而非迁就代码）？
- [ ] 25. **`SelectorApi` 三方法是否返回 `ApiFuture<'_, T>`（boxed-future）**？有无改回裸 `async fn`（会失去 `Arc<dyn SelectorApi>`）或引入 `async-trait` 宏（C.2 白名单外）？§7.10 `FakeApi` 是否用 `Box::pin(async move { … })` 实现、且能被 `Arc<dyn SelectorApi>` 注入？
- [ ] 26. 本章是否**没有**修改 `01-master-prompt.md`、`appendices/C-conventions.md` 或其它章节文件？v1.2/v1.3/v1.4 的契约变更是否已**回填**而非另提新请求？

---

## 7.16 跨章接口冻结表（Frozen Interfaces）

> **为什么需要这张表**：07 章固定了 `SelectorApi` 三方法，08 章同时在独立实现 `kernel/api.rs`。
> 两章各自演进时，下游 Agent 会照着一个**能读、但编译不过**的组合去实现
> （如 `ApiError::Status(401)` vs `ApiError::Unauthorized`、`GroupView` vs `GroupInfo`）。
> 本节把这些签名集中冻结，**签名以本节 + C.6.1 为准，不以任何单章的叙述为准**。

| # | 接口名 | 所属模块/文件 | 完整签名 | 调用方 | 被调方 | 冻结版本 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `SelectableGroup` | `kernel/api.rs` | `struct SelectableGroup { name: String, members: Vec<String>, now: Option<String> }` | 07 | 08 | `v1.3-freeze-1` |
| 2 | `GroupInfo` | `kernel/api.rs` | `struct GroupInfo { name: String, kind: String, now: Option<String>, all: Vec<String>, test_url: Option<String>, hidden: bool }` | 07 | 08 | `v1.3-freeze-1` |
| 3 | `ApiError` | `kernel/api.rs` | `enum ApiError { Unauthorized, NotFound(String), Timeout, Status(u16, String), Transport(String) }` | 07 | 08 | `v1.3-freeze-1` |
| 4 | `SelectorApi::proxies_selectable` | `kernel/api.rs` | `fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>` | 07 | 08 | `v1.4-freeze-1` |
| 5 | `SelectorApi::proxy_group` | `kernel/api.rs` | `fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>` | 07 | 08 | `v1.4-freeze-1` |
| 6 | `SelectorApi::set_group_selected` | `kernel/api.rs` | `fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>` | 07 | 08 | `v1.4-freeze-1` |
| 7 | `SwitchRuntimeLocks` | `speedtest/selector.rs` | `struct SwitchRuntimeLocks { group: String, locked_at: i64 }` | 07 | 07 | `v1.2-freeze-1` |
| 8 | `Selector::after_speedtest` | `speedtest/selector.rs` | `async fn after_speedtest(&self, api: &CoreApi, policy_cfg: &SwitchPolicy, run: &SpeedtestRun, summary: &ProbeSummary, events: EventSink, now: i64) -> Result<Option<ApplyBestResult>, AppError>` | 06 | 07 | `v1.2-freeze-1` |
| 9 | `Selector::update_failure_counters` | `speedtest/selector.rs` | `async fn update_failure_counters(&self, api: &CoreApi, policy_cfg: &SwitchPolicy, summary: &ProbeSummary) -> Result<(), AppError>` | 06（经 `after_speedtest`） | 07 | `v1.2-freeze-1` |
| 10 | `Selector::apply_best` | `speedtest/selector.rs` | `async fn apply_best(&self, api: &CoreApi, policy_cfg: &SwitchPolicy, run_id: &str, run: &SpeedtestRun, summary: &ProbeSummary, events: EventSink, now: i64) -> Result<Option<ApplyBestResult>, AppError>` | 04 章 IPC 封装 / 06 | 07 | `v1.2-freeze-1` |
| 11 | `Selector::apply_node_manual` | `speedtest/selector.rs` | `async fn apply_node_manual(&self, api: &CoreApi, policy_cfg: &SwitchPolicy, group: &str, node: &str, summary: &ProbeSummary, events: EventSink, now: i64) -> Result<SwitchRecord, AppError>` | IPC `switch_apply_node` | 07 | `v1.2-freeze-1` |
| 12 | `Selector::recent` | `speedtest/selector.rs` | `async fn recent(&self, limit: usize) -> Vec<SwitchRecord>` | IPC `switch_history` | 07 | `v1.2-freeze-1` |
| 13 | `ApplyBestResult` | `speedtest/selector.rs` | `struct ApplyBestResult { primary: SwitchRecord, all: Vec<SwitchRecord> }` | 04 章 IPC 封装 | 07 | `v1.2-freeze-1` |
| 14 | `SpeedtestEngine::start` | `speedtest/engine.rs` | `async fn start(self: &Arc<Self>, api: Arc<CoreApi>, inventory: Vec<ProbeTarget>, rules: ExcludeRules, region_priority: Vec<String>, policy: SwitchPolicy, selector: Arc<Selector>, trigger: RunTrigger, events: EventSink) -> Result<StartOutcome, AppError>` | IPC `speedtest_start` / 05 | 06 | `v1.3-freeze-1` |
| 15 | `SpeedtestEngine::status` | `speedtest/engine.rs` | `async fn status(&self, run_id: &str) -> Result<SpeedtestRun, AppError>` | IPC `speedtest_status` | 06 | `v1.3-freeze-1` |
| 16 | `SpeedtestEngine::cancel` | `speedtest/engine.rs` | `async fn cancel(&self, run_id: &str)` | IPC `speedtest_cancel` | 06 | `v1.3-freeze-1` |
| 17 | `StartOutcome` | `speedtest/engine.rs` | `enum StartOutcome { Started(String), AlreadyRunning(String) }` | 06 | 06 | `v1.3-freeze-1` |
| 18 | `TaskExecutor::run_speedtest` | `scheduler/runner.rs` | `async fn run_speedtest(&self) -> Result<(), AppError>` | 05 | 06 | `v1.3-freeze-1` |
| 19 | `TaskExecutor::run_sub_refresh` | `scheduler/runner.rs` | `async fn run_sub_refresh(&self, uid: &str) -> Result<(), AppError>` | 05 | 03 | `v1.3-freeze-1` |
| 20 | `TaskExecutor::run_expire_check` | `scheduler/runner.rs` | `async fn run_expire_check(&self) -> Result<(), AppError>` | 05 | 03 | `v1.3-freeze-1` |
| 21 | `SchedulerCmd` | `scheduler/runner.rs` | 见下方完整定义 | 全章 IPC 层 | 05 | `v1.3-freeze-1` |
| 22 | `Scheduler::run` | `scheduler/runner.rs` | `async fn run(mut self, rx: mpsc::Receiver<SchedulerCmd>, shutdown: watch::Receiver<bool>, events: EventSink)` | `lib.rs` 启动 | 05 | `v1.3-freeze-1` |
| 23 | `rank_candidates_with_priority` | `speedtest/rank.rs` | `fn rank_candidates_with_priority(results: &[ProbeResult], rules: &ExcludeRules, region_priority: &[String]) -> Vec<Candidate>` | 06 / 07 | 06 | `v1.3-freeze-1` |
| 24 | `ExcludeRules::from_policy` | `speedtest/rank.rs` | `fn from_policy(p: &SwitchPolicy) -> Self` | 06 | 06 | `v1.3-freeze-1` |
| 25 | `CoreApi::proxy_delay` | `kernel/api.rs` | `async fn proxy_delay(&self, name: &str, url: &str, timeout_ms: u32, expected: u16) -> Result<u32, ApiError>` | 06 | 08 | `v1.3-freeze-1` |
| 26 | `ApiFuture` | `kernel/api.rs` | `type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>` | 07 | 08 | `v1.4-freeze-1` |

**`SchedulerCmd` 完整定义（第 21 行，5 章对外暴露的唯一入口）**

```rust
// scheduler/runner.rs —— 外部 → 调度器的命令（有界 channel 容量 16，满则回 E_BUSY）
#[derive(Debug)]
pub enum SchedulerCmd {
    RefreshNow { uid: String },              // subscription_refresh
    RefreshAllNow,                           // subscription_refresh_all
    SpeedtestNow,                            // speedtest_start(trigger=Manual)
    Rebuild { slots: Vec<Slot> },            // 订阅增删改后重建槽位
    SetInterval { uid: String, interval_sec: i64 }, // subscription_set_interval
    CoreStateChanged { running: bool },      // core:state 的镜像
    NetworkBack,                             // 网络恢复提示
    Shutdown,                                // 应用退出
}
```

> **`SchedulerCmd` 是 5 章唯一对外入口**：IPC 层、`lib.rs`、订阅管理器都只能通过它
> 与调度器通信，**不得**直接调 `Scheduler` 的内部方法或读 `slots` 字段（那会绕过单 tick 循环，
> 造成「两个定时源」）。容量 16 是有意的：满了说明调用方在死循环里狂发命令，回 `E_BUSY` 比无限排队安全。

### 7.16.1 冻结规则

1. **签名以本节为准**。任何章节（含 05/06/07/08 章正文）的单方面修改**无效**，
   以本表 + C.6.1 的记录为准。
2. **变更流程**：先在 C 章提 `CONTRACT-CHANGE-REQUEST:` → 获裁决后
   **同一次提交内**同时更新 ① C.6.1 ② 本节表格 ③ 受影响章节的代码块，三者缺一不可。
3. **冻结版本号规则**：`v<契约版本>-freeze-<序号>`。契约版本随 C 章升级（v1.2 → v1.3 → v1.4），
   同版本内的签名修补递增 freeze 序号（`v1.4-freeze-1` → `v1.4-freeze-2`）。
   只改实现细节（函数体、日志、常量）**不升版本**；改签名/字段名/变体**必升版本**。
   **本表当前分三档**（多版本并存是**有意**的，没有签名变化的条目不该被"顺手"升版，
   否则版本号就失去「哪天空改了签名」的信息量）：

   | 版本 | 收录条目 | 为什么是这一档 |
   | --- | --- | --- |
   | `v1.2-freeze-1` | #7–#13 | 07 章内部接口（`Selector` 方法、`SwitchRuntimeLocks`、`ApplyBestResult`），v1.2 定稿后未再变 |
   | `v1.3-freeze-1` | #1–#3、#14–#25 | C.6.1 首次引入契约形态时的签名（`GroupInfo`/`SelectableGroup`/`ApiError`、引擎与调度器入口、rank/delay） |
   | `v1.4-freeze-1` | #4–#6、#26 | `SelectorApi` 改 boxed-future（`ApiFuture` 形式） |
4. **字段名不得别名**：`GroupInfo.name`（不是 `group`）、`GroupInfo.kind`（不是 `group_type`）、
   `SelectableGroup.members`（不是 `all`）。任何章节若需要别名，只能加扩展 trait（见 §7.4 `GroupInfoExt`），
   **不得**定义影子结构体（`GroupView` 已因此废弃）。
5. **dyn 兼容是硬约束**：`SelectorApi` 的三个方法必须返回 `ApiFuture<'_, T>`，
   **不得**改成裸 `async fn`（会失去 `Arc<dyn SelectorApi>` 能力，§7.10 的 `FakeApi` 立刻编译不过），
   也**不得**引入 `async-trait` 宏（C.2 依赖白名单不含它）。这条比"签名逐字一致"更强：
   它规定的是**形式**，不是名字。
6. **下游校验**：实现阶段若发现某章实现与本表不符，**以实现本表为准**，
   并回过头改那一章的文档——不要「迁就已有代码」。

### 7.16.2 与 08 章的对齐确认

| 项 | 状态 |
| --- | --- |
| 08 章 `SelectorApi` 三方法 | 已实现于 **08 §08.6.3**，**boxed-future 形式**，与本节 #4–#6 一致 |
| 08 章 `ApiFuture` 类型别名 | 已实现于 **08 §08.6.3**，与本节 #26 一致 |
| 08 章 `ApiError` 五变体 | 已实现于 **08 §08.6.3**，与本节 #3 一致 |
| 08 章 `ApiError → AppError` 映射 | 已实现于 **08 §08.17.1**，07 章 `map_api_err` 逐条对齐 |
| 08 章 `GroupInfo` / `SelectableGroup` | 已实现于 **08 §08.6.3**，字段与本节 #1–#2 一致 |
| 08 章 `proxy_delay` | 已实现于 **08 §08.6.3 附录 A**，与本节 #25 一致 |

> **双方签名以 C.6.1 + 本节为准**。07 章（`core-writer`）与 08 章（`app-writer`）已确认
> 使用同一版本 **`v1.4-freeze-1`**（`SelectorApi` 的 boxed-future 形式）。任何一方变更需
> **同步 C.6.1 并升冻结版本号**，且必须在同一轮内通知对方，否则会重现 v1.2 期间
> `error[E0599]: no variant named 'Status'` 那类编译级不一致。
>
> **为什么 `ApiFuture` 必须进冻结表**：它不只是一个类型别名，而是 `SelectorApi` 的**调用形式**。
> 若 08 章实现成裸 `async fn`、07 章仍声明 `ApiFuture`，报错不是"找不到方法"，而是
> `error[E0195]: lifetime parameters do not match the trait declaration` 或
> `the trait bound ... is not satisfied`——这类错误指向的是 trait 签名不匹配，
> 排查时很难一眼看出根因是"没装箱"。把它单列为第 26 条，就是为了让这种不一致在核对表格时直接暴露。
>
> **`Timeout` 映射的裁决**（沿用 08 §08.17.1，07 章照抄不另立）：
> `ApiError::Timeout` → `AppError{ code: E_NET_TIMEOUT, retryable: true }`，
> **不**归 `E_CORE_START_FAILED`。理由：后者语义是「启动失败/端口占用」，
> 把读超时归进去会让 UI 误报「内核启动失败」并暗示用户重启内核。

---
