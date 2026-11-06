//! Windows DPAPI 加解密（契约 C.3：订阅 URL 与 secret 落盘前必须加密）。
//!
//! 用 `CryptProtectData` / `CryptUnprotectData`，密钥绑定当前用户 + `CRYPTPROTECT_UI_FORBIDDEN`
//! （绝不弹 UI，否则后台线程会卡死）。密文以 base64 存 JSON。

use base64::Engine;
use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
};

use crate::domain::errors::{AppError, AppResult, ErrorCode};

fn blob_from_bytes(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

/// 加密并返回 base64 字符串
pub fn protect(plain: &str) -> AppResult<String> {
    let input = blob_from_bytes(plain.as_bytes());
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // SAFETY: input 指向的切片在整个调用期间有效；output 由系统分配，调用后 LocalFree 释放。
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),          // 描述文本
            std::ptr::null(),          // 附加熵
            std::ptr::null_mut(),      // 保留
            std::ptr::null_mut(),      // 提示结构
            CRYPTPROTECT_UI_FORBIDDEN, // 关键：不弹 UI
            &mut output,
        )
    };

    if ok == 0 {
        return Err(AppError::with_detail(
            ErrorCode::StoreCrypto,
            std::io::Error::last_os_error(),
        ));
    }

    // SAFETY: 成功时 output.pbData 由系统分配，长度 cbData 有效。
    let cipher = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let v = slice.to_vec();
        LocalFree(output.pbData as HLOCAL);
        v
    };

    Ok(base64::engine::general_purpose::STANDARD.encode(cipher))
}

/// 解密 base64 密文
pub fn unprotect(cipher_b64: &str) -> AppResult<String> {
    let cipher = base64::engine::general_purpose::STANDARD
        .decode(cipher_b64.trim())
        .map_err(|e| AppError::with_detail(ErrorCode::StoreCrypto, e))?;

    let input = blob_from_bytes(&cipher);
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // SAFETY: 同 protect；input 生命周期覆盖调用。
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };

    if ok == 0 {
        return Err(AppError::with_detail(
            ErrorCode::StoreCrypto,
            std::io::Error::last_os_error(),
        ));
    }

    // SAFETY: 同 protect。
    let plain = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let v = slice.to_vec();
        LocalFree(output.pbData as HLOCAL);
        v
    };

    String::from_utf8(plain).map_err(|e| AppError::with_detail(ErrorCode::StoreCrypto, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protect_unprotect_roundtrip() {
        let plain = "https://example.com/api/v1/client/subscribe?token=abc123secret";
        let c = protect(plain).expect("加密应成功");
        assert_ne!(c, plain, "密文不应等于明文");
        assert!(!c.contains("abc123secret"), "密文不得泄露明文片段");
        let back = unprotect(&c).expect("解密应成功");
        assert_eq!(back, plain);
    }

    #[test]
    fn roundtrip_handles_chinese_and_empty() {
        for s in ["", "中文订阅地址", "a", &"x".repeat(4096)] {
            let c = protect(s).unwrap();
            assert_eq!(unprotect(&c).unwrap(), s, "往返失败: {s:?}");
        }
    }

    #[test]
    fn unprotect_rejects_garbage() {
        assert!(unprotect("not-base64!!!").is_err());
        assert!(unprotect("aGVsbG8=").is_err(), "非 DPAPI 密文应解密失败");
    }

    #[test]
    fn different_plaintext_gives_different_cipher() {
        let a = protect("aaa").unwrap();
        let b = protect("bbb").unwrap();
        assert_ne!(a, b);
    }
}
