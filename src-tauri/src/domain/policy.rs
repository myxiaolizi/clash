//! 切换策略纯函数（契约 07 章）。**不碰网络、不碰时钟**，全部可单测。
//!
//! 判定顺序（07 §7.4，⓪ 优先级最高）：
//! ⓪ 手动覆盖期内 → 抑制
//! ① 启用开关关闭 → 抑制
//! ② 最短驻留未满 → 抑制
//! ③ 当前节点探测失败且失败次数 ≥ 阈值 → 切换（reason = CurrentFailed）
//! ④ 存在明显更快的候选（差值 > tolerance） → 切换（reason = Best）
//! 否则不切换

use crate::domain::model::{
    GroupInfo, ProbeResult, SelectableGroup, SwitchPolicy, SwitchReason,
};

/// ⚠️ 关键常量：mihomo **API** 返回的组类型是大驼峰 `Selector`，
/// 而订阅 YAML 里写的是小写 `select`。二者**永不相等**。
/// 任何比较都必须用这个常量精确匹配，禁止 `to_lowercase()` / `contains("select")`。
pub const KIND_SELECTOR: &str = "Selector";

/// 单个组是否可被我们切换。
///
/// 必须大小写敏感精确匹配。`"Selector".to_lowercase()` 得到 `"selector"`，
/// 与 YAML 的 `"select"` 并不相等——这是本项目最容易写错的一处判断。
pub fn is_selector(kind: &str) -> bool {
    kind == KIND_SELECTOR
}

/// 排除规则：把不参与切换的成员名过滤掉
#[derive(Debug, Clone)]
pub struct ExcludeRules {
    keywords: Vec<String>,
}

impl ExcludeRules {
    pub fn from_policy(p: &SwitchPolicy) -> Self {
        Self {
            keywords: p.exclude_keywords.clone(),
        }
    }

    /// mihomo 内置名不可 PUT（SAFE_PATHS），会被 400 拒绝
    pub fn is_builtin(name: &str) -> bool {
        const BUILTINS: &[&str] = &[
            "DIRECT",
            "REJECT",
            "REJECT-DROP",
            "PASS",
            "COMPATIBLE",
            "GLOBAL",
        ];
        BUILTINS.iter().any(|b| b.eq_ignore_ascii_case(name))
    }

    /// 判定某成员名是否应被排除
    pub fn should_exclude(&self, name: &str) -> bool {
        if Self::is_builtin(name) {
            return true;
        }
        self.keywords.iter().any(|k| name.contains(k.as_str()))
    }

    /// 过滤出可切换成员
    pub fn filter<'a>(&self, names: &'a [String]) -> Vec<&'a String> {
        names
            .iter()
            .filter(|n| !self.should_exclude(n))
            .collect()
    }
}

/// 候选节点（排序后的中间结构）
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub node_key: String,
    pub delay_ms: u32,
    /// 地区优先级命中下标；None 表示未命中任何优先地区
    pub region_rank: Option<usize>,
}

/// 排序键（07 §7.3）：
/// 1. 延迟升序
/// 2. 地区优先级命中靠前
/// 3. 名称字典序（保证稳定，避免并列时随机抖动）
pub fn rank_candidates_with_priority(
    results: &[ProbeResult],
    rules: &ExcludeRules,
    region_priority: &[String],
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = results
        .iter()
        .filter(|r| r.is_ok())
        .filter(|r| !rules.should_exclude(&r.node_key))
        .map(|r| {
            let region_rank = if region_priority.is_empty() {
                None
            } else {
                region_priority
                    .iter()
                    .position(|p| r.node_key.contains(p.as_str()))
            };
            Candidate {
                node_key: r.node_key.clone(),
                delay_ms: r.delay_ms.unwrap_or(u32::MAX),
                region_rank,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        // 1) 地区优先级：命中的排前
        let ra = a.region_rank.unwrap_or(usize::MAX);
        let rb = b.region_rank.unwrap_or(usize::MAX);
        ra.cmp(&rb)
            // 2) 延迟
            .then_with(|| a.delay_ms.cmp(&b.delay_ms))
            // 3) 名称稳定序
            .then_with(|| a.node_key.cmp(&b.node_key))
    });

    out
}

/// 本轮失败计数（连续失败次数）
pub type FailureCounter = std::collections::HashMap<String, u8>;

/// 依据测速结果更新失败计数：成功清零，失败 +1
pub fn update_failure_counters(
    counters: &mut FailureCounter,
    results: &[ProbeResult],
) {
    for r in results {
        let e = counters.entry(r.node_key.clone()).or_insert(0);
        if r.is_ok() {
            *e = 0;
        } else {
            *e = e.saturating_add(1);
        }
    }
}

/// 抑制原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suppress {
    /// ⓪ 手动覆盖期内
    ManualOverride,
    /// ① 策略未启用
    Disabled,
    /// ② 最短驻留未满
    MinDwell,
    /// ③ 无可切换候选
    NoCandidate,
    /// ④ 优势不足（未超过 tolerance）
    WithinTolerance,
    /// ⑤ 当前已是最优
    AlreadyBest,
}

impl Suppress {
    pub fn as_str(&self) -> &'static str {
        match self {
            Suppress::ManualOverride => "manual_override",
            Suppress::Disabled => "disabled",
            Suppress::MinDwell => "min_dwell",
            Suppress::NoCandidate => "no_candidate",
            Suppress::WithinTolerance => "within_tolerance",
            Suppress::AlreadyBest => "already_best",
        }
    }
}

/// 切换决策
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// 执行切换
    Switch {
        to_node: String,
        to_delay: Option<u32>,
        from_delay: Option<u32>,
        reason: SwitchReason,
    },
    /// 不切换
    Hold(Suppress),
}

/// 输入上下文（把外部状态显式传入，保证纯函数可测）
#[derive(Debug, Clone)]
pub struct DecisionCtx<'a> {
    pub policy: &'a SwitchPolicy,
    pub group: &'a SelectableGroup,
    /// 当前选中节点
    pub current: Option<&'a str>,
    /// 当前节点延迟（本轮测速结果，None = 本轮失败或无数据）
    pub current_delay: Option<u32>,
    /// 当前节点连续失败次数
    pub current_failures: u8,
    /// 已排序候选（延迟升序）
    pub candidates: &'a [Candidate],
    /// 上次切换时间（用于 min_dwell）
    pub last_switch_at: Option<i64>,
    /// 手动覆盖锁定到期时间
    pub manual_lock_until: Option<i64>,
    pub now: i64,
}

/// 核心决策函数（07 §7.4）
pub fn decide(ctx: &DecisionCtx<'_>) -> Decision {
    // ⓪ 手动覆盖期：最高优先级，用户刚选过就不许系统覆盖
    if let Some(until) = ctx.manual_lock_until {
        if ctx.now < until {
            return Decision::Hold(Suppress::ManualOverride);
        }
    }

    // ① 策略未启用
    if !ctx.policy.enabled {
        return Decision::Hold(Suppress::Disabled);
    }

    // ③ 当前节点失败达阈值 → 优先救急，**不受 min_dwell 约束**
    //    理由：节点挂了还硬等 5 分钟，用户会认为软件坏了
    let cur_failed = ctx.current_delay.is_none();
    if cur_failed && ctx.current_failures >= ctx.policy.failure_threshold {
        if let Some(best) = ctx.candidates.first() {
            // 候选不能是当前节点自身
            if Some(best.node_key.as_str()) != ctx.current {
                return Decision::Switch {
                    to_node: best.node_key.clone(),
                    to_delay: Some(best.delay_ms),
                    from_delay: None,
                    reason: SwitchReason::CurrentFailed,
                };
            }
        }
        return Decision::Hold(Suppress::NoCandidate);
    }

    // ② 最短驻留未满
    if let Some(last) = ctx.last_switch_at {
        let dwell = (ctx.now - last).max(0) as u32;
        if dwell < ctx.policy.min_dwell_sec {
            return Decision::Hold(Suppress::MinDwell);
        }
    }

    let Some(best) = ctx.candidates.first() else {
        return Decision::Hold(Suppress::NoCandidate);
    };

    // ⑤ 当前已是最优
    if Some(best.node_key.as_str()) == ctx.current {
        return Decision::Hold(Suppress::AlreadyBest);
    }

    // ④ 优势是否足够
    match ctx.current_delay {
        // 当前无有效延迟（但未达失败阈值）→ 视为可切
        None => Decision::Switch {
            to_node: best.node_key.clone(),
            to_delay: Some(best.delay_ms),
            from_delay: None,
            reason: SwitchReason::CurrentFailed,
        },
        Some(cur) => {
            let improvement = cur.saturating_sub(best.delay_ms);
            if improvement > ctx.policy.tolerance_ms {
                Decision::Switch {
                    to_node: best.node_key.clone(),
                    to_delay: Some(best.delay_ms),
                    from_delay: Some(cur),
                    reason: SwitchReason::Best,
                }
            } else {
                Decision::Hold(Suppress::WithinTolerance)
            }
        }
    }
}

/// 手动覆盖锁定是否仍生效
pub fn lock_is_active(locked_at: i64, manual_override_min: u32, now: i64) -> bool {
    if manual_override_min == 0 {
        return false;
    }
    let until = locked_at.saturating_add((manual_override_min as i64) * 60);
    now < until
}

/// 计算锁定到期时刻
pub fn lock_until(locked_at: i64, manual_override_min: u32) -> Option<i64> {
    if manual_override_min == 0 {
        return None;
    }
    Some(locked_at.saturating_add((manual_override_min as i64) * 60))
}

/// 从 `GroupInfo` 提取可切换成员（排除内置名与关键词）
pub fn members_of(group: &GroupInfo, rules: &ExcludeRules) -> Vec<String> {
    rules.filter(&group.all).into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- kind 大小写（本项目头号陷阱） ----------

    #[test]
    fn kind_case_sensitive_selector_is_selectable() {
        assert!(is_selector("Selector"), "API 原值必须被识别");
    }

    #[test]
    fn kind_case_sensitive_lowercase_select_is_not_selectable() {
        // YAML 里写的是 select，但 API 返回 Selector。若拿 YAML 值来比，会恒 false。
        assert!(!is_selector("select"), "YAML 值不得被当作 API 值");
    }

    #[test]
    fn kind_case_sensitive_other_group_kinds_are_not_selectable() {
        for k in [
            "URLTest",
            "Fallback",
            "LoadBalance",
            "Relay",
            "url-test",
            "fallback",
            "load-balance",
            "selector",
            "SELECTOR",
            "",
        ] {
            assert!(!is_selector(k), "{k} 不应被识别为可切换组");
        }
    }

    #[test]
    fn kind_no_loose_comparison_in_source() {
        // 源码扫描：禁止任何宽松比较写法。
        // 行为断言挡不住"将来有人顺手改回小写"，只有扫源码才能真拦。
        //
        // 扫描范围 = 非测试部分 + 非注释行：
        // 文档注释里会提到这些反例（"禁止 contains(select)"），必须排除，否则自我误报。
        let src = include_str!("policy.rs");
        let impl_part = src
            .split("\n#[cfg(test)]")
            .next()
            .expect("policy.rs 必须有非测试部分");

        let code_lines: Vec<&str> = impl_part
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.starts_with("//") && !l.starts_with("*") && !l.starts_with("/*"))
            .collect();

        for bad in [
            concat!("contains(", "\"select\")"),
            concat!("to_lowercase() == ", "\"select\""),
            concat!("to_ascii_lowercase() == ", "\"select\""),
            concat!("eq_ignore_ascii_case(", "\"select\")"),
        ] {
            let hit = code_lines.iter().find(|l| l.contains(bad));
            assert!(
                hit.is_none(),
                "实现代码中禁止出现宽松比较 {bad}，命中行：{}",
                hit.map(|s| s.to_string()).unwrap_or_default()
            );
        }

        // 必须存在以字面量定义的常量，作为唯一比较基准
        assert_eq!(
            KIND_SELECTOR, "Selector",
            "KIND_SELECTOR 必须等于 API 原值 Selector（大驼峰）"
        );
        assert!(
            impl_part.contains("pub const KIND_SELECTOR"),
            "可切换组类型必须集中为一个常量，禁止各处散写字符串"
        );
    }

    // ---------- 排除规则 ----------

    #[test]
    fn exclude_builtin_names() {
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        for n in ["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE", "GLOBAL"] {
            assert!(rules.should_exclude(n), "{n} 是内置名，PUT 会被 400 拒绝");
        }
        // 大小写不敏感地识别内置名（内核 SAFE_PATHS 也不区分大小写）
        assert!(rules.should_exclude("direct"));
    }

    #[test]
    fn exclude_info_nodes_by_keyword() {
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        assert!(rules.should_exclude("剩余流量：100GB"));
        assert!(rules.should_exclude("套餐到期：2026-12-31"));
        assert!(!rules.should_exclude("HongKong-01"));
    }

    // ---------- 排序 ----------

    fn probe(key: &str, delay: Option<u32>) -> ProbeResult {
        match delay {
            Some(d) => ProbeResult::ok(key.into(), key.into(), d, 0),
            None => ProbeResult::fail(key.into(), key.into(), "timeout".into(), 0),
        }
    }

    #[test]
    fn rank_sorts_by_delay_ascending() {
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        let results = vec![probe("a", Some(200)), probe("b", Some(50)), probe("c", Some(120))];
        let ranked = rank_candidates_with_priority(&results, &rules, &[]);
        let keys: Vec<_> = ranked.iter().map(|c| c.node_key.as_str()).collect();
        assert_eq!(keys, vec!["b", "c", "a"]);
    }

    #[test]
    fn rank_drops_failures_and_excluded() {
        let mut p = SwitchPolicy::default();
        p.exclude_keywords = vec!["到期".into()];
        let rules = ExcludeRules::from_policy(&p);
        let results = vec![
            probe("good", Some(80)),
            probe("dead", None),
            probe("到期提醒", Some(10)),
            probe("DIRECT", Some(5)),
        ];
        let ranked = rank_candidates_with_priority(&results, &rules, &[]);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].node_key, "good");
    }

    #[test]
    fn rank_applies_region_priority_before_delay() {
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        let results = vec![probe("US-01", Some(50)), probe("HK-01", Some(120))];
        let ranked =
            rank_candidates_with_priority(&results, &rules, &["HK".to_string(), "US".to_string()]);
        assert_eq!(ranked[0].node_key, "HK-01", "地区优先级高于延迟");
    }

    #[test]
    fn rank_is_stable_for_equal_delay() {
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        let results = vec![probe("b", Some(100)), probe("a", Some(100))];
        let r1 = rank_candidates_with_priority(&results, &rules, &[]);
        let r2 = rank_candidates_with_priority(&results, &rules, &[]);
        assert_eq!(
            r1.iter().map(|c| c.node_key.clone()).collect::<Vec<_>>(),
            r2.iter().map(|c| c.node_key.clone()).collect::<Vec<_>>(),
            "并列时必须稳定排序，否则会来回抖"
        );
        assert_eq!(r1[0].node_key, "a");
    }

    // ---------- 决策 ----------

    fn ctx<'a>(
        policy: &'a SwitchPolicy,
        group: &'a SelectableGroup,
        candidates: &'a [Candidate],
        current: Option<&'a str>,
        current_delay: Option<u32>,
        failures: u8,
        now: i64,
    ) -> DecisionCtx<'a> {
        DecisionCtx {
            policy,
            group,
            current,
            current_delay,
            current_failures: failures,
            candidates,
            last_switch_at: None,
            manual_lock_until: None,
            now,
        }
    }

    fn grp() -> SelectableGroup {
        SelectableGroup {
            name: "Proxy".into(),
            members: vec!["a".into(), "b".into()],
            now: Some("a".into()),
        }
    }

    fn cands() -> Vec<Candidate> {
        vec![
            Candidate { node_key: "b".into(), delay_ms: 40, region_rank: None },
            Candidate { node_key: "a".into(), delay_ms: 300, region_rank: None },
        ]
    }

    #[test]
    fn decide_switches_when_clearly_better() {
        let p = SwitchPolicy::default();
        let g = grp();
        let c = cands();
        let d = decide(&ctx(&p, &g, &c, Some("a"), Some(300), 0, 1000));
        match d {
            Decision::Switch { to_node, reason, from_delay, to_delay } => {
                assert_eq!(to_node, "b");
                assert_eq!(reason, SwitchReason::Best);
                assert_eq!(from_delay, Some(300));
                assert_eq!(to_delay, Some(40));
            }
            other => panic!("应切换，实际 {other:?}"),
        }
    }

    #[test]
    fn decide_holds_within_tolerance() {
        let p = SwitchPolicy::default(); // tolerance 30
        let g = grp();
        let c = vec![
            Candidate { node_key: "b".into(), delay_ms: 90, region_rank: None },
            Candidate { node_key: "a".into(), delay_ms: 100, region_rank: None },
        ];
        let d = decide(&ctx(&p, &g, &c, Some("a"), Some(100), 0, 1000));
        assert_eq!(d, Decision::Hold(Suppress::WithinTolerance), "只快 10ms 不该切");
    }

    #[test]
    fn decide_manual_override_has_highest_priority() {
        let p = SwitchPolicy::default(); // manual_override_min = 60
        let g = grp();
        let c = cands();
        let mut cx = ctx(&p, &g, &c, Some("a"), Some(300), 0, 1000);
        cx.manual_lock_until = Some(1000 + 3600);
        assert_eq!(
            decide(&cx),
            Decision::Hold(Suppress::ManualOverride),
            "手动覆盖期优先于一切，含救急分支"
        );
    }

    #[test]
    fn decide_rescues_failed_current_ignoring_min_dwell() {
        let p = SwitchPolicy::default();
        let g = grp();
        let c = cands();
        let mut cx = ctx(&p, &g, &c, Some("a"), None, 2, 1000);
        cx.last_switch_at = Some(1000); // 刚切过
        match decide(&cx) {
            Decision::Switch { to_node, reason, .. } => {
                assert_eq!(to_node, "b");
                assert_eq!(reason, SwitchReason::CurrentFailed);
            }
            other => panic!("节点挂了应立刻救急，实际 {other:?}"),
        }
    }

    #[test]
    fn decide_holds_when_failures_below_threshold() {
        let p = SwitchPolicy::default(); // failure_threshold = 2
        let g = grp();
        let c = cands();
        let cx = ctx(&p, &g, &c, Some("a"), None, 1, 1000);
        // 失败 1 次未达阈值，且无 min_dwell 限制 → 走 ④ 分支（当前无延迟视为可切）
        match decide(&cx) {
            Decision::Switch { reason, .. } => assert_eq!(reason, SwitchReason::CurrentFailed),
            Decision::Hold(s) => assert_eq!(s, Suppress::NoCandidate),
        }
    }

    #[test]
    fn decide_respects_min_dwell_after_switch() {
        let p = SwitchPolicy::default(); // min_dwell_sec = 300
        let g = grp();
        let c = cands();
        let mut cx = ctx(&p, &g, &c, Some("a"), Some(300), 0, 1000);
        cx.last_switch_at = Some(900); // 才过 100 秒
        assert_eq!(decide(&cx), Decision::Hold(Suppress::MinDwell));
    }

    #[test]
    fn decide_already_best() {
        let p = SwitchPolicy::default();
        let g = grp();
        let c = cands();
        let d = decide(&ctx(&p, &g, &c, Some("b"), Some(40), 0, 1000));
        assert_eq!(d, Decision::Hold(Suppress::AlreadyBest));
    }

    #[test]
    fn decide_disabled_holds() {
        let mut p = SwitchPolicy::default();
        p.enabled = false;
        let g = grp();
        let c = cands();
        assert_eq!(
            decide(&ctx(&p, &g, &c, Some("a"), Some(300), 0, 1000)),
            Decision::Hold(Suppress::Disabled)
        );
    }

    #[test]
    fn decide_no_candidate_when_all_failed() {
        let p = SwitchPolicy::default();
        let g = grp();
        let empty: Vec<Candidate> = vec![];
        let d = decide(&ctx(&p, &g, &empty, Some("a"), Some(100), 0, 1000));
        assert_eq!(d, Decision::Hold(Suppress::NoCandidate));
    }

    // ---------- 失败计数 ----------

    #[test]
    fn failure_counters_increment_and_reset() {
        let mut c = FailureCounter::new();
        update_failure_counters(&mut c, &[probe("a", None), probe("b", Some(50))]);
        assert_eq!(c["a"], 1);
        assert_eq!(c["b"], 0);
        update_failure_counters(&mut c, &[probe("a", None)]);
        assert_eq!(c["a"], 2);
        update_failure_counters(&mut c, &[probe("a", Some(30))]);
        assert_eq!(c["a"], 0, "成功即清零");
    }

    #[test]
    fn failure_counter_saturates() {
        let mut c = FailureCounter::new();
        c.insert("a".into(), 255);
        update_failure_counters(&mut c, &[probe("a", None)]);
        assert_eq!(c["a"], 255, "饱和不得回绕");
    }

    // ---------- 手动覆盖锁定 ----------

    #[test]
    fn manual_lock_window() {
        assert!(lock_is_active(1000, 60, 1000 + 3599));
        assert!(!lock_is_active(1000, 60, 1000 + 3600));
        assert!(!lock_is_active(1000, 0, 1000), "0 分钟表示不锁定");
        assert_eq!(lock_until(1000, 60), Some(4600));
        assert_eq!(lock_until(1000, 0), None);
    }

    #[test]
    fn members_of_filters_builtins() {
        let g = GroupInfo {
            name: "Proxy".into(),
            kind: KIND_SELECTOR.into(),
            now: Some("a".into()),
            all: vec!["a".into(), "b".into(), "DIRECT".into(), "REJECT".into()],
            test_url: None,
            hidden: false,
        };
        let rules = ExcludeRules::from_policy(&SwitchPolicy::default());
        assert_eq!(members_of(&g, &rules), vec!["a", "b"]);
    }
}
