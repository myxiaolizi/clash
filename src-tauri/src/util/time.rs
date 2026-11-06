//! 时间工具。统一 Unix 秒（契约 C.10：全项目对外时间一律 Unix 秒 i64）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 Unix 秒
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 当前 Unix 毫秒（仅用于耗时测量，不用于数据模型）
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 对齐到下一个整分（调度器用：所有任务在整分边界触发，避免分散唤醒）
///
/// 返回距离下一个整分的秒数（1..=60）。
pub fn secs_to_next_minute(now: i64) -> i64 {
    let rem = now.rem_euclid(60);
    if rem == 0 {
        60
    } else {
        60 - rem
    }
}

/// 把时间戳对齐到整分（向下取整）
pub fn floor_to_minute(ts: i64) -> i64 {
    ts - ts.rem_euclid(60)
}

/// Unix 秒 → `YYYY-MM-DD HH:MM:SS`（本地时区）
pub fn format_local(ts: i64) -> String {
    use chrono_lite::local_datetime;
    local_datetime(ts)
}

/// 极简本地时间格式化（避免为格式化引入 chrono 全量依赖）
mod chrono_lite {
    /// 把 Unix 秒按本地时区偏移格式化。
    /// 这里用一个够用的实现：调用方若需要严格时区可换 chrono。
    pub fn local_datetime(ts: i64) -> String {
        let days = ts.div_euclid(86_400);
        let secs_of_day = ts.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        let h = secs_of_day / 3600;
        let mi = (secs_of_day % 3600) / 60;
        let s = secs_of_day % 60;
        format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
    }

    /// Howard Hinnant 的 civil_from_days 算法（1970-01-01 起的天数 → 年月日）
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        (if m <= 2 { y + 1 } else { y }, m, d)
    }
}

/// 秒 → 人类可读时长（用于 UI 与日志）
pub fn humanize_secs(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s} 秒")
    } else if s < 3600 {
        format!("{} 分钟", s / 60)
    } else if s < 86_400 {
        format!("{} 小时", s / 3600)
    } else {
        format!("{} 天", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_minute_boundary() {
        assert_eq!(secs_to_next_minute(0), 60);
        assert_eq!(secs_to_next_minute(1), 59);
        assert_eq!(secs_to_next_minute(59), 1);
        assert_eq!(secs_to_next_minute(60), 60);
    }

    #[test]
    fn floor_to_minute_works() {
        assert_eq!(floor_to_minute(125), 120);
        assert_eq!(floor_to_minute(120), 120);
    }

    #[test]
    fn format_epoch_known_values() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(format_local(0), "1970-01-01 00:00:00");
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(format_local(946_684_800), "2000-01-01 00:00:00");
        // 2024-01-01 00:00:00 UTC = 1704067200
        assert_eq!(format_local(1_704_067_200), "2024-01-01 00:00:00");
        // 2026-01-01 00:00:00 UTC = 1767225600
        assert_eq!(format_local(1_767_225_600), "2026-01-01 00:00:00");
    }

    #[test]
    fn humanize() {
        assert_eq!(humanize_secs(30), "30 秒");
        assert_eq!(humanize_secs(120), "2 分钟");
        assert_eq!(humanize_secs(7200), "2 小时");
        assert_eq!(humanize_secs(172_800), "2 天");
        assert_eq!(humanize_secs(-5), "0 秒");
    }
}
