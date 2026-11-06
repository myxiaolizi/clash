//! 端口探测与分配。
//!
//! 分配策略：从首选端口开始顺序试绑，成功后**立刻释放**再交给内核。
//! 这里存在固有竞态（释放到内核绑定之间的窗口），因此启动后必须用
//! `GET /configs` 回读核对实际端口，不一致就重试。

use std::net::TcpListener;

use crate::domain::errors::{AppError, AppResult, ErrorCode};

/// 混合代理端口候选起始值
pub const MIXED_PORT_START: u16 = 10_801;
/// external-controller 端口候选起始值
pub const API_PORT_START: u16 = 49_152;

/// 在 [start, start+span) 范围内找第一个可用端口
pub fn find_free_port(start: u16, span: u16) -> AppResult<u16> {
    for offset in 0..span {
        let port = start.saturating_add(offset);
        if port == 0 {
            continue;
        }
        if is_port_free(port) {
            return Ok(port);
        }
    }
    Err(AppError::msg(
        ErrorCode::CorePortExhausted,
        format!("{start}..{} 范围内无可用端口", start.saturating_add(span)),
    ))
}

/// 端口是否可绑（绑定后立即释放）
pub fn is_port_free(port: u16) -> bool {
    // 绑 127.0.0.1 而非 0.0.0.0：我们只监听回环，避免与其它网卡的占用误判
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => {
            drop(l);
            true
        }
        Err(_) => false,
    }
}

/// 分配混合端口与 API 端口，保证两者不冲突
pub fn allocate_port_pair() -> AppResult<(u16, u16)> {
    let mixed = find_free_port(MIXED_PORT_START, 100)?;
    // API 端口从自己的区间起，且不能等于 mixed
    let mut api = find_free_port(API_PORT_START, 100)?;
    let mut tries = 0;
    while api == mixed && tries < 100 {
        api = find_free_port(api.saturating_add(1), 100)?;
        tries += 1;
    }
    if api == mixed {
        return Err(AppError::new(ErrorCode::CorePortExhausted));
    }
    Ok((mixed, api))
}

/// 生成 32 位 hex secret
pub fn generate_secret() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_free_port() {
        let p = find_free_port(MIXED_PORT_START, 100).expect("应能找到可用端口");
        assert!(p >= MIXED_PORT_START);
    }

    #[test]
    fn port_pair_never_collides() {
        for _ in 0..5 {
            let (m, a) = allocate_port_pair().unwrap();
            assert_ne!(m, a, "混合端口与 API 端口不得相同");
            assert!(m >= MIXED_PORT_START && m < MIXED_PORT_START + 100);
            assert!(a >= API_PORT_START && a < API_PORT_START + 100);
        }
    }

    #[test]
    fn occupied_port_is_skipped() {
        // 占住起始端口，应跳过它
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        assert!(!is_port_free(port), "已占用端口应判定为不可用");
        drop(occupied);
        // 释放后可能立即可用（也可能被 TIME_WAIT 占用，故不断言）
    }

    #[test]
    fn secret_is_32_hex() {
        let s = generate_secret();
        assert_eq!(s.len(), 32, "必须是 32 位 hex");
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(generate_secret(), generate_secret(), "两次生成必须不同");
    }

    #[test]
    fn exhausted_range_reports_error() {
        // 起始端口 + span=1 且占用时必然失败
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        let r = find_free_port(port, 1);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().code, ErrorCode::CorePortExhausted);
    }
}
