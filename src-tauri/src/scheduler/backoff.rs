//! 指数退避（P3）。
//!
//! 用于：订阅拉取失败后的重试、内核重启（supervisor 内有独立实现）。
//! 契约要求：退避必须有**上限**且**成功即重置**，否则一次失败会永久拉长周期。

use std::time::Duration;

/// 退避策略
#[derive(Debug, Clone)]
pub struct Backoff {
    base_ms: u64,
    max_ms: u64,
    /// 当前连续失败次数
    attempt: u32,
    /// 最大尝试次数（超过则不再重试，等下一个正常周期）
    max_attempts: u32,
}

impl Backoff {
    pub fn new(base_ms: u64, max_ms: u64, max_attempts: u32) -> Self {
        Self {
            base_ms: base_ms.max(1),
            max_ms: max_ms.max(base_ms.max(1)),
            attempt: 0,
            max_attempts: max_attempts.max(1),
        }
    }

    /// 订阅拉取的默认退避：1 分钟起，最长 30 分钟，最多 4 次
    pub fn for_subscription() -> Self {
        Self::new(60_000, 1_800_000, 4)
    }

    /// 测速的默认退避：30 秒起，最长 5 分钟，最多 3 次
    pub fn for_speedtest() -> Self {
        Self::new(30_000, 300_000, 3)
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// 记录一次失败，返回下次应等待多久；`None` 表示已达上限，不再重试
    pub fn next_delay(&mut self) -> Option<Duration> {
        self.attempt = self.attempt.saturating_add(1);
        if self.attempt > self.max_attempts {
            return None;
        }
        // 1, 2, 4, 8... 倍
        let shift = (self.attempt - 1).min(20);
        let ms = self.base_ms.saturating_mul(1u64 << shift).min(self.max_ms);
        Some(Duration::from_millis(ms))
    }

    /// 成功：重置计数
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// 是否已放弃（需等下次正常周期）
    pub fn is_exhausted(&self) -> bool {
        self.attempt > self.max_attempts
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::for_subscription()
    }
}

/// 计算纯函数版退避（无状态，便于测试与跨调用保存）
pub fn delay_for_attempt(base_ms: u64, max_ms: u64, attempt: u32) -> u64 {
    if attempt == 0 {
        return 0;
    }
    let shift = (attempt - 1).min(20);
    base_ms.saturating_mul(1u64 << shift).min(max_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_grows_exponentially_then_caps() {
        assert_eq!(delay_for_attempt(1000, 30_000, 0), 0);
        assert_eq!(delay_for_attempt(1000, 30_000, 1), 1000);
        assert_eq!(delay_for_attempt(1000, 30_000, 2), 2000);
        assert_eq!(delay_for_attempt(1000, 30_000, 3), 4000);
        assert_eq!(delay_for_attempt(1000, 30_000, 5), 16_000);
        assert_eq!(delay_for_attempt(1000, 30_000, 6), 30_000, "必须封顶");
        assert_eq!(delay_for_attempt(1000, 30_000, 100), 30_000, "不得溢出");
    }

    #[test]
    fn backoff_counts_and_resets() {
        let mut b = Backoff::new(1000, 10_000, 3);
        assert_eq!(b.next_delay().unwrap().as_millis(), 1000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 2000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 4000);
        assert!(b.next_delay().is_none(), "超过 max_attempts 应放弃");
        assert!(b.is_exhausted());

        b.reset();
        assert_eq!(b.attempt(), 0);
        assert!(!b.is_exhausted());
        assert_eq!(b.next_delay().unwrap().as_millis(), 1000, "重置后从头开始");
    }

    #[test]
    fn subscription_backoff_defaults() {
        let mut b = Backoff::for_subscription();
        assert_eq!(b.next_delay().unwrap().as_millis(), 60_000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 120_000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 240_000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 480_000);
        assert!(b.next_delay().is_none(), "默认最多 4 次");
    }

    #[test]
    fn speedtest_backoff_defaults() {
        let mut b = Backoff::for_speedtest();
        assert_eq!(b.next_delay().unwrap().as_millis(), 30_000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 60_000);
        assert_eq!(b.next_delay().unwrap().as_millis(), 120_000);
        assert!(b.next_delay().is_none(), "默认最多 3 次");
    }

    #[test]
    fn max_below_base_is_corrected() {
        // max < base 时不应产生"越退越快"的怪异行为
        let b = Backoff::new(10_000, 1_000, 3);
        assert!(b.max_ms >= b.base_ms);
    }

    #[test]
    fn zero_base_is_corrected() {
        let b = Backoff::new(0, 1000, 3);
        assert!(b.base_ms >= 1, "base 为 0 会导致忙重试");
    }
}
