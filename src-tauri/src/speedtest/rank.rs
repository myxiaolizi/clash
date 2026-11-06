//! 排序视图（P4）。给 UI 展示用，与决策共用 `policy::rank_candidates_with_priority`。

use crate::domain::model::ProbeResult;
use crate::domain::policy::Candidate;

/// UI 展示用的排序结果
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankOutcome {
    pub rows: Vec<RankRow>,
    pub ok_count: u32,
    pub fail_count: u32,
    /// 最快节点延迟（无可用节点时 None）
    pub best_delay: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankRow {
    pub node: String,
    pub delay_ms: Option<u32>,
    pub ok: bool,
    /// 名次（从 1 开始；失败节点为 None）
    pub rank: Option<usize>,
    pub error: Option<String>,
}

impl From<Vec<Candidate>> for RankOutcome {
    fn from(cands: Vec<Candidate>) -> Self {
        let rows: Vec<RankRow> = cands
            .iter()
            .enumerate()
            .map(|(i, c)| RankRow {
                node: c.node_key.clone(),
                delay_ms: Some(c.delay_ms),
                ok: true,
                rank: Some(i + 1),
                error: None,
            })
            .collect();
        let best_delay = rows.first().and_then(|r| r.delay_ms);
        let ok_count = rows.len() as u32;
        Self {
            rows,
            ok_count,
            fail_count: 0,
            best_delay,
        }
    }
}

/// 从原始探测结果生成完整视图（含失败节点）
pub fn build_view(results: &[ProbeResult]) -> RankOutcome {
    let mut rows: Vec<RankRow> = results
        .iter()
        .map(|r| RankRow {
            node: r.node_key.clone(),
            delay_ms: r.delay_ms,
            ok: r.is_ok(),
            rank: None,
            error: r.error.clone(),
        })
        .collect();

    // 成功节点按延迟升序，失败节点排最后
    rows.sort_by(|a, b| match (a.ok, b.ok) {
        (true, true) => a
            .delay_ms
            .unwrap_or(u32::MAX)
            .cmp(&b.delay_ms.unwrap_or(u32::MAX))
            .then_with(|| a.node.cmp(&b.node)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => a.node.cmp(&b.node),
    });

    let mut rank = 0usize;
    for r in rows.iter_mut() {
        if r.ok {
            rank += 1;
            r.rank = Some(rank);
        }
    }

    let ok_count = rows.iter().filter(|r| r.ok).count() as u32;
    let fail_count = rows.len() as u32 - ok_count;
    let best_delay = rows.first().filter(|r| r.ok).and_then(|r| r.delay_ms);

    RankOutcome {
        rows,
        ok_count,
        fail_count,
        best_delay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(k: &str, d: Option<u32>) -> ProbeResult {
        match d {
            Some(x) => ProbeResult::ok(k.into(), k.into(), x, 0),
            None => ProbeResult::fail(k.into(), k.into(), "timeout".into(), 0),
        }
    }

    #[test]
    fn view_sorts_ok_first_then_by_delay() {
        let v = build_view(&[
            probe("slow", Some(300)),
            probe("dead", None),
            probe("fast", Some(20)),
        ]);
        let names: Vec<_> = v.rows.iter().map(|r| r.node.as_str()).collect();
        assert_eq!(names, vec!["fast", "slow", "dead"], "失败节点排最后");
        assert_eq!(v.ok_count, 2);
        assert_eq!(v.fail_count, 1);
        assert_eq!(v.best_delay, Some(20));
    }

    #[test]
    fn ranks_only_ok_nodes() {
        let v = build_view(&[probe("a", Some(10)), probe("b", None), probe("c", Some(20))]);
        assert_eq!(v.rows[0].rank, Some(1));
        assert_eq!(v.rows[1].rank, Some(2));
        assert_eq!(v.rows[2].rank, None, "失败节点无名次");
    }

    #[test]
    fn all_failed_has_no_best() {
        let v = build_view(&[probe("a", None), probe("b", None)]);
        assert_eq!(v.best_delay, None);
        assert_eq!(v.ok_count, 0);
        assert_eq!(v.fail_count, 2);
    }

    #[test]
    fn empty_results() {
        let v = build_view(&[]);
        assert!(v.rows.is_empty());
        assert_eq!(v.best_delay, None);
    }

    #[test]
    fn candidate_conversion_ranks_in_order() {
        let cands = vec![
            Candidate { node_key: "a".into(), delay_ms: 10, region_rank: None },
            Candidate { node_key: "b".into(), delay_ms: 20, region_rank: None },
        ];
        let v: RankOutcome = cands.into();
        assert_eq!(v.rows.len(), 2);
        assert_eq!(v.rows[0].rank, Some(1));
        assert_eq!(v.best_delay, Some(10));
    }
}
