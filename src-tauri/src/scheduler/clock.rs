//! 调度时钟与抖动（P3）。
//!
//! 设计原则（09 章）：
//! 1. **单 tick 循环**：只有一个定时任务，每轮算"谁到期了"，避免 N 个定时器
//!    各自唤醒（省电，也避免 UI 线程被多个唤醒打断）。
//! 2. **对齐整分**：所有判断基于 `floor_to_minute(now)`，唤醒时刻取下一个整分，
//!    这样休眠/唤醒后行为可预测，不会因为开机时刻不同导致周期漂移。
//! 3. **抖动 0..30s**：多台设备/多个订阅同时更新会打机场接口，加随机抖动分散。
//!    抖动只影响"何时执行"，不影响"是否到期"。

use std::time::Duration;

use crate::util::time::{floor_to_minute, now_secs, secs_to_next_minute};

/// 抖动上限（秒）。契约 C.6：0..30s。
pub const JITTER_MAX_SECS: u32 = 30;

/// 计算下一次唤醒应等待的时长。
///
/// 返回 `Duration`（至少 1 秒，避免忙轮询）。
pub fn next_wake_delay() -> Duration {
    let now = now_secs();
    let to_minute = secs_to_next_minute(now);
    Duration::from_secs(to_minute.max(1) as u64)
}

/// 为某类任务生成抖动偏移（0..=JITTER_MAX_SECS）。
///
/// 用任务名做种子的一部分，保证**同一任务每次抖动稳定**（不会一会儿 3 秒一会儿 28 秒），
/// 但不同任务错开。这样既分散又不引入额外随机性导致难以复现。
pub fn jitter_offset_secs(task_key: &str) -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    task_key.hash(&mut h);
    // 混入当天日期：每天抖动值不同，避免长期固定时刻形成规律
    (now_secs() / 86_400).hash(&mut h);
    (h.finish() % (JITTER_MAX_SECS as u64 + 1)) as u32
}

/// 判断某任务在 `now` 时刻是否已过抖动窗口，可以执行。
///
/// `window_start` 是整分边界（该任务本轮应执行的时刻）。
pub fn jitter_elapsed(task_key: &str, window_start: i64, now: i64) -> bool {
    let offset = jitter_offset_secs(task_key) as i64;
    now >= window_start.saturating_add(offset)
}

/// 统一对齐到整分（供各处判断"同一周期内只执行一次"）
pub fn period_bucket(now: i64, period_min: u32) -> i64 {
    let period = (period_min.max(1) as i64) * 60;
    floor_to_minute(now) / period
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_delay_is_within_a_minute() {
        for _ in 0..20 {
            let d = next_wake_delay();
            assert!(d.as_secs() >= 1, "不得为 0，避免忙轮询");
            assert!(d.as_secs() <= 60, "必须在 1 分钟内唤醒");
        }
    }

    #[test]
    fn jitter_is_bounded() {
        for key in ["sub", "speedtest", "cleanup", "a", "b", "very-long-task-key-name"] {
            let j = jitter_offset_secs(key);
            assert!(j <= JITTER_MAX_SECS, "抖动 {j} 超过上限 {JITTER_MAX_SECS}");
        }
    }

    #[test]
    fn jitter_is_stable_within_same_day() {
        // 同一天同任务必须稳定，否则周期判断会漂
        let a = jitter_offset_secs("sub-update");
        let b = jitter_offset_secs("sub-update");
        assert_eq!(a, b);
    }

    #[test]
    fn jitter_differs_between_tasks_usually() {
        // 不强求一定不同（可能碰撞），但抽查多个任务应至少有两个不同值
        let vals: Vec<u32> = ["task-a", "task-b", "task-c", "task-d", "task-e"]
            .iter()
            .map(|k| jitter_offset_secs(k))
            .collect();
        let uniq: std::collections::HashSet<_> = vals.iter().collect();
        assert!(uniq.len() > 1, "多个任务完全同抖动说明哈希失效: {vals:?}");
    }

    #[test]
    fn jitter_elapsed_respects_offset() {
        let key = "speedtest";
        let off = jitter_offset_secs(key) as i64;
        let window_start = 1_000_000;
        assert!(!jitter_elapsed(key, window_start, window_start));
        if off > 0 {
            assert!(!jitter_elapsed(key, window_start, window_start + off - 1));
        }
        assert!(jitter_elapsed(key, window_start, window_start + off));
        assert!(jitter_elapsed(key, window_start, window_start + off + 100));
    }

    #[test]
    fn period_bucket_groups_by_minutes() {
        // 取一个 15 分钟桶的起点（900 秒整除）
        let bucket_start = 1_000_800i64; // 1000800 / 900 = 1112 整除
        assert_eq!(bucket_start % 900, 0, "测试前提：必须是桶起点");

        let a = period_bucket(bucket_start, 15);
        let b = period_bucket(bucket_start + 899, 15);
        assert_eq!(a, b, "同一 15 分钟周期内应同桶");

        let c = period_bucket(bucket_start + 900, 15);
        assert_ne!(a, c, "跨周期应换桶");
    }

    #[test]
    fn period_bucket_is_stable_within_bucket_for_any_offset() {
        // 对桶内任意偏移都必须同桶（这才是"幂等"的真实要求）
        let bucket_start = 1_000_800i64;
        let expect = period_bucket(bucket_start, 15);
        for off in [0i64, 1, 30, 59, 60, 300, 899] {
            assert_eq!(
                period_bucket(bucket_start + off, 15),
                expect,
                "offset={off} 时桶不应变化"
            );
        }
    }

    #[test]
    fn period_bucket_minute_granularity() {
        // 桶以整分为边界：同分内不同秒不会换桶
        let t = 1_000_800i64;
        assert_eq!(period_bucket(t, 15), period_bucket(t + 59, 15));
    }

    #[test]
    fn period_bucket_minimum_one_minute() {
        // 传入 0 也不得除零
        let _ = period_bucket(1_000_000, 0);
    }
}
