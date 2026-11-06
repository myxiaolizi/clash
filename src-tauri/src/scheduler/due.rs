//! 到期判定纯函数（P3）。全部可单测，无 IO 无时钟依赖。

use crate::scheduler::clock::{jitter_elapsed, period_bucket};
use crate::util::time::floor_to_minute;

/// 一个需要周期执行的任务
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodicTask {
    /// 稳定标识（抖动种子）
    pub key: String,
    /// 周期（分钟）
    pub period_min: u32,
    /// 上次执行时刻（Unix 秒）
    pub last_run_at: Option<i64>,
    /// 上次执行所在周期桶（防止同一周期重复执行）
    pub last_bucket: Option<i64>,
}

impl PeriodicTask {
    pub fn new(key: impl Into<String>, period_min: u32, last_run_at: Option<i64>) -> Self {
        Self {
            key: key.into(),
            period_min: period_min.max(1),
            last_run_at,
            last_bucket: last_run_at.map(|t| period_bucket(t, period_min)),
        }
    }

    /// 本任务在 `now` 是否应执行
    ///
    /// 三重条件：
    /// 1. 已过 `period_min` 分钟（按整分对齐比较，避免秒级漂移累积）
    /// 2. 抖动窗口已到
    /// 3. 本周期桶未执行过（幂等，即便 tick 多次也不会重复跑）
    pub fn is_due(&self, now: i64) -> bool {
        let bucket = period_bucket(now, self.period_min);
        if self.last_bucket == Some(bucket) {
            return false; // 本周期已跑过
        }

        // 从窗口起点判断抖动
        let window_start = floor_to_minute(now);
        if !jitter_elapsed(&self.key, window_start, now) {
            return false;
        }

        let Some(last) = self.last_run_at else {
            return true; // 从未执行
        };

        let due_after = last.saturating_add((self.period_min as i64) * 60);
        now >= floor_to_minute(due_after)
    }

    /// 标记已执行
    pub fn mark_run(&mut self, now: i64) {
        self.last_run_at = Some(now);
        self.last_bucket = Some(period_bucket(now, self.period_min));
    }
}

/// 计算距离任务下次可执行还需多少秒（用于 UI 展示倒计时）
pub fn secs_until_due(task: &PeriodicTask, now: i64) -> i64 {
    let Some(last) = task.last_run_at else {
        return 0;
    };
    let due_at = last.saturating_add((task.period_min as i64) * 60);
    (due_at - now).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_run_is_due_only_after_jitter() {
        let t = PeriodicTask::new("sub", 15, None);
        let base = 1_000_000i64; // 整分附近
        let aligned = floor_to_minute(base);
        // 刚进窗口时，抖动可能未到
        let off = crate::scheduler::clock::jitter_offset_secs("sub") as i64;
        assert!(!t.is_due(aligned + off - 1) || off == 0);
        assert!(t.is_due(aligned + off));
        assert!(t.is_due(aligned + off + 60));
    }

    #[test]
    fn not_due_within_same_period() {
        let now = 1_000_000;
        let mut t = PeriodicTask::new("sub", 15, None);
        t.mark_run(now);
        // 同周期内不再执行
        assert!(!t.is_due(now + 60));
        assert!(!t.is_due(now + 300));
    }

    #[test]
    fn due_after_period_elapsed() {
        let now = floor_to_minute(1_000_000);
        let mut t = PeriodicTask::new("sub", 15, None);
        t.mark_run(now);
        // 15 分钟后（含抖动）
        let later = now + 15 * 60 + crate::scheduler::clock::jitter_offset_secs("sub") as i64;
        assert!(t.is_due(later));
    }

    #[test]
    fn minimum_period_is_one_minute() {
        let t = PeriodicTask::new("x", 0, None);
        assert_eq!(t.period_min, 1, "周期为 0 会导致每 tick 都跑");
    }

    #[test]
    fn secs_until_due_counts_down() {
        let now = floor_to_minute(1_000_000);
        let mut t = PeriodicTask::new("sub", 15, None);
        t.mark_run(now);
        assert_eq!(secs_until_due(&t, now), 900);
        assert_eq!(secs_until_due(&t, now + 300), 600);
        assert_eq!(secs_until_due(&t, now + 2000), 0, "已过期不下溢");
    }

    #[test]
    fn secs_until_due_zero_when_never_run() {
        let t = PeriodicTask::new("sub", 15, None);
        assert_eq!(secs_until_due(&t, 1000), 0);
    }

    #[test]
    fn mark_run_updates_bucket() {
        let now = floor_to_minute(2_000_000);
        let mut t = PeriodicTask::new("sub", 30, None);
        assert!(t.last_bucket.is_none());
        t.mark_run(now);
        assert_eq!(t.last_bucket, Some(period_bucket(now, 30)));
        assert_eq!(t.last_run_at, Some(now));
    }

    #[test]
    fn task_with_last_run_sets_bucket() {
        let now = floor_to_minute(3_000_000);
        let t = PeriodicTask::new("sub", 15, Some(now));
        assert_eq!(t.last_bucket, Some(period_bucket(now, 15)));
        assert!(!t.is_due(now), "同周期不应立即执行");
    }
}
