//! 内核流量统计的持久化与差分采集（v3）。
//!
//! # 为什么不能只读内核 API
//!
//! mihomo 的 `/traffic` 给的是**当次进程累计**，内核一重启就归零；
//! 用户要的「今日 / 累计」必须由我们自己跨进程持久化。
//!
//! 采集方式（每 60 秒一次）：
//! 1. 读内核 `/connections` 的汇总（或 `/traffic` 的瞬时值累加）
//! 2. 与上次读数做差 → 得到本区间增量
//! 3. 增量累进 `today_*` / `total_*` / 小时桶
//!
//! **关键坑**：内核重启后累计值会跳回 0，差值会是负数。
//! 因此差分必须用「饱和减法」并在检测到回退时视为"计数器重置"，
//! 把本次读数整个当作增量（而不是记一笔 0 或巨大值）。

use std::sync::RwLock;

use crate::domain::model::TrafficStats;
use crate::system::paths::{read_json_recover, write_json, Paths};

/// 上次读到的内核原始累计值（纯内存，不落盘：内核重启后本来就该重置）
#[derive(Debug, Clone, Copy, Default)]
struct LastReading {
    up: u64,
    down: u64,
    /// 已初始化过（否则第一次读数全是"增量"，会把历史流量算进今日）
    primed: bool,
}

pub struct TrafficStore {
    path: std::path::PathBuf,
    stats: RwLock<TrafficStats>,
    last: RwLock<LastReading>,
}

impl TrafficStore {
    pub fn load(paths: &Paths) -> Self {
        let path = paths.traffic_stats_file();
        let stats = read_json_recover::<TrafficStats>(&path)
            .ok()
            .flatten()
            .unwrap_or_default();
        Self {
            path,
            stats: RwLock::new(stats),
            last: RwLock::new(LastReading::default()),
        }
    }

    /// 当前统计快照
    pub fn snapshot(&self) -> TrafficStats {
        self.stats
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// 喂入内核的**累计**读数（`GET /connections` 的 up/down 总和）。
    ///
    /// 首次调用只记基线不算增量——否则会把内核启动前就有的累计值
    /// 一次性算进"今日"，用户会看到凭空多出几个 GB。
    pub fn feed_cumulative(&self, now: i64, up: u64, down: u64) -> Option<(u64, u64)> {
        let mut last = self.last.write().ok()?;

        // 首次：只建基线
        if !last.primed {
            last.up = up;
            last.down = down;
            last.primed = true;
            tracing::debug!(up, down, "流量采集已建立基线");
            return None;
        }

        // 计数器回退 = 内核重启，本次读数整体视为增量
        let (d_up, d_down) = if up < last.up || down < last.down {
            tracing::info!(
                prev_up = last.up,
                now_up = up,
                prev_down = last.down,
                now_down = down,
                "内核流量计数器归零（内核重启），重新建立基线"
            );
            last.up = up;
            last.down = down;
            return None;
        } else {
            (up - last.up, down - last.down)
        };

        last.up = up;
        last.down = down;

        if d_up == 0 && d_down == 0 {
            return Some((0, 0));
        }

        self.apply_delta(now, d_up, d_down);
        Some((d_up, d_down))
    }

    /// 直接喂增量（测试/备用路径）
    pub fn feed_delta(&self, now: i64, up: u64, down: u64) {
        self.apply_delta(now, up, down);
    }

    fn apply_delta(&self, now: i64, up: u64, down: u64) {
        let snapshot = {
            let Ok(mut s) = self.stats.write() else {
                return;
            };
            s.record(now, up, down);
            s.clone()
        };
        // 落盘失败只记日志：统计不是关键路径，不能因为它中断采集循环
        if let Err(e) = write_json(&self.path, &snapshot) {
            tracing::warn!(err = %e, "流量统计落盘失败");
        }
    }

    /// 内核停止时调用：清掉基线，避免下次启动把旧累计值当增量
    pub fn reset_baseline(&self) {
        if let Ok(mut l) = self.last.write() {
            *l = LastReading::default();
        }
    }

    /// 清空今日统计（用户手动"清零"用，累计不动）
    pub fn clear_today(&self, now: i64) {
        let snapshot = {
            let Ok(mut s) = self.stats.write() else {
                return;
            };
            s.today_up = 0;
            s.today_down = 0;
            s.day = TrafficStats::day_of(now);
            s.updated_at = now;
            s.clone()
        };
        let _ = write_json(&self.path, &snapshot);
    }
}

/// 从 `/connections` 的 connections 数组汇总上下行。
///
/// 单独抽成纯函数便于单测：真实响应结构复杂，但汇总规则就一条——
/// 每条的 `upload` / `download` 累加。
pub fn sum_connection_bytes(conns: &[ConnBytes]) -> (u64, u64) {
    let mut up = 0u64;
    let mut down = 0u64;
    for c in conns {
        up = up.saturating_add(c.upload);
        down = down.saturating_add(c.download);
    }
    (up, down)
}

/// `/connections` 里单条连接我们关心的字段
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
pub struct ConnBytes {
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, TrafficStore) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(&paths.roaming).unwrap();
        let store = TrafficStore::load(&paths);
        (dir, store)
    }

    #[test]
    fn first_reading_only_primes_baseline() {
        let (_d, store) = setup();
        // 首次读数：不产生增量（否则会把内核启动前的累计算进今日）
        assert!(store.feed_cumulative(1000, 5000, 9000).is_none());
        let s = store.snapshot();
        assert_eq!(s.today_up, 0);
        assert_eq!(s.total_up, 0);
    }

    #[test]
    fn cumulative_delta_accumulates() {
        let (_d, store) = setup();
        store.feed_cumulative(1000, 100, 200); // 基线
        let d = store.feed_cumulative(1060, 300, 700).unwrap();
        assert_eq!(d, (200, 500));

        let s = store.snapshot();
        assert_eq!(s.today_up, 200);
        assert_eq!(s.today_down, 500);
        assert_eq!(s.total_up, 200);
        assert_eq!(s.total_down, 500);

        // 再来一次
        store.feed_cumulative(1120, 400, 1000).unwrap();
        let s = store.snapshot();
        assert_eq!(s.today_up, 300);
        assert_eq!(s.today_down, 800);
    }

    #[test]
    fn counter_reset_does_not_produce_negative_or_huge_delta() {
        let (_d, store) = setup();
        store.feed_cumulative(1000, 10_000, 20_000); // 基线
        store.feed_cumulative(1060, 11_000, 22_000).unwrap(); // +1000/+2000

        // 内核重启：累计值归零
        assert!(
            store.feed_cumulative(1120, 50, 100).is_none(),
            "计数器回退时只重建基线，不产生增量"
        );

        let s = store.snapshot();
        assert_eq!(s.today_up, 1000, "不得因重启而暴涨");
        assert_eq!(s.today_down, 2000);

        // 重启后继续正常累加
        store.feed_cumulative(1180, 150, 300).unwrap();
        let s = store.snapshot();
        assert_eq!(s.today_up, 1100);
        assert_eq!(s.today_down, 2200);
    }

    #[test]
    fn zero_delta_is_fine() {
        let (_d, store) = setup();
        store.feed_cumulative(1000, 100, 200);
        let d = store.feed_cumulative(1060, 100, 200).unwrap();
        assert_eq!(d, (0, 0));
        assert_eq!(store.snapshot().today_up, 0);
    }

    #[test]
    fn hourly_buckets_accumulate_within_same_hour() {
        let (_d, store) = setup();
        store.feed_delta(3_700, 100, 200); // 01:01:40
        store.feed_delta(3_900, 50, 60); // 01:05:00 同一小时

        let s = store.snapshot();
        assert_eq!(s.hourly.len(), 1, "同一小时只一个桶");
        assert_eq!(s.hourly[0].hour, 3_600);
        assert_eq!(s.hourly[0].up, 150);
        assert_eq!(s.hourly[0].down, 260);
    }

    #[test]
    fn hourly_buckets_capped() {
        let (_d, store) = setup();
        // 灌 100 个不同小时的桶
        for i in 0..100i64 {
            store.feed_delta(i * 3_600 + 10, 1, 1);
        }
        let s = store.snapshot();
        assert!(
            s.hourly.len() <= crate::domain::model::TRAFFIC_HOURLY_KEEP,
            "小时桶必须有上限，实际 {}",
            s.hourly.len()
        );
        // 保留的必须是最新的
        let last_hour = s.hourly.last().unwrap().hour;
        assert_eq!(last_hour, 99 * 3_600);
    }

    #[test]
    fn day_rollover_resets_today_keeps_total() {
        let (_d, store) = setup();
        let day1 = 86_400 + 100;
        store.feed_delta(day1, 500, 700);
        let s = store.snapshot();
        assert_eq!(s.today_up, 500);
        assert_eq!(s.total_up, 500);

        // 次日
        let day2 = 2 * 86_400 + 100;
        store.feed_delta(day2, 10, 20);
        let s = store.snapshot();
        assert_eq!(s.today_up, 10, "跨天后今日归零重计");
        assert_eq!(s.today_down, 20);
        assert_eq!(s.total_up, 510, "累计不清零");
        assert_eq!(s.total_down, 720);
        assert_eq!(s.day, 2);
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(&paths.roaming).unwrap();

        {
            let store = TrafficStore::load(&paths);
            store.feed_delta(1000, 111, 222);
        }
        let store2 = TrafficStore::load(&paths);
        let s = store2.snapshot();
        assert_eq!(s.total_up, 111);
        assert_eq!(s.total_down, 222);
    }

    #[test]
    fn reset_baseline_reprimes() {
        let (_d, store) = setup();
        store.feed_cumulative(1000, 1000, 2000);
        store.reset_baseline();
        // 重置后第一次喂值只建基线
        assert!(store.feed_cumulative(1060, 50, 60).is_none());
        assert_eq!(store.snapshot().today_up, 0);
    }

    #[test]
    fn clear_today_keeps_total() {
        let (_d, store) = setup();
        store.feed_delta(1000, 100, 200);
        store.clear_today(1000);
        let s = store.snapshot();
        assert_eq!(s.today_up, 0);
        assert_eq!(s.today_down, 0);
        assert_eq!(s.total_up, 100, "累计不受清零影响");
    }

    #[test]
    fn sum_connection_bytes_works() {
        let conns = vec![
            ConnBytes { upload: 100, download: 200 },
            ConnBytes { upload: 50, download: 60 },
        ];
        assert_eq!(sum_connection_bytes(&conns), (150, 260));
        assert_eq!(sum_connection_bytes(&[]), (0, 0));
    }

    #[test]
    fn recent_hours_fills_gaps() {
        let (_d, store) = setup();
        let now = 10 * 3_600 + 100;
        store.feed_delta(now, 10, 20);
        let hours = store.snapshot().recent_hours(now, 3);
        assert_eq!(hours.len(), 3, "缺失小时要补 0 保证曲线连续");
        assert_eq!(hours[2].up, 10, "最新一小时在最后");
        assert_eq!(hours[0].up, 0);
    }
}
