//! ===== 抽屉柜 加密备份/恢复模块 =====
//! 文件格式：后缀 .drawerbox
//!   前 64 字节为明文 Header（可读校验 + 密钥派生）
//!   64 字节之后为 AES-256-GCM 加密流（内容 = zstd 压缩后的 JSON 数据）
//!
//! Header 布局（总计 64 字节，全部为明文）：
//!   Offset | Size | 含义
//!   0..10  | 10B  | Magic：b"DRAWERBOX1"（校验文件格式）
//!   10..42 | 32B  | Salt（Argon2id 密钥派生的随机盐，每次导出重新生成）
//!   42..54 | 12B  | Nonce（AES-GCM 随机 nonce，每次导出重新生成）
//!   54..64 | 10B  | 预留 padding（全 0，留作未来扩展 kdf 参数/版本号用）
//!
//! 加密内容体（紧跟 Header 之后）：
//!   = AES-256-GCM(zstd(level=3, json_bytes), key=derive(master_password, salt), nonce=header_nonce)
//!   AES-GCM 的 16 字节认证 Tag 附在密文末尾（aes-gcm crate 自动拼接）

use aes_gcm::{aead::{Aead, KeyInit, Payload}, Aes256Gcm, Nonce};
use crate::crypto;
use zeroize::Zeroizing;

pub const HEADER_SIZE: usize = 64;
pub const MAGIC: &[u8; 10] = b"DRAWERBOX1";
pub const MAGIC_V2: &[u8; 10] = b"DRAWERBOX2";
pub const V2_BACKUP_VERSION: u32 = 2;
const MAGIC_LEN: usize = 10;
const SALT_OFF: usize = 10;
const SALT_LEN: usize = 32;
const NONCE_OFF: usize = 42;
const NONCE_LEN: usize = 12;
const VERSION_OFF: usize = 54;
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    LegacyV1,
    StableDekV2,
    Unknown,
}

pub fn classify_backup(backup_bytes: &[u8]) -> BackupKind {
    if backup_bytes.len() < MAGIC_LEN {
        return BackupKind::Unknown;
    }
    match &backup_bytes[..MAGIC_LEN] {
        bytes if bytes == MAGIC => BackupKind::LegacyV1,
        bytes if bytes == MAGIC_V2 => BackupKind::StableDekV2,
        _ => BackupKind::Unknown,
    }
}

/// 加密一份 JSON 字节流，产出完整的 .drawerbox 单文件字节流（含 Header）
pub fn encrypt_backup(json_bytes: &[u8], master_password: &str) -> Result<Vec<u8>, String> {
    // 1. 压缩 JSON（先压缩后加密，安全性更强，也显著减少备份大小）
    let compressed = zstd::encode_all(json_bytes, ZSTD_LEVEL)
        .map_err(|e| format!("zstd 压缩失败: {}", e))?;

    // 2. 生成 32B 盐 + 12B nonce（每次导出完全随机，抗重放 / 抗彩虹表）
    let salt = crypto::generate_salt();
    let nonce_bytes = crypto::generate_nonce();
    assert_eq!(salt.len(), SALT_LEN);
    assert_eq!(nonce_bytes.len(), NONCE_LEN);

    // 3. 用 Argon2id 派生 32B AES 密钥（抗暴力破解的核心）
    let key = crypto::derive_key(master_password, &salt);

    // 4. AES-256-GCM 加密压缩体（aes-gcm 自动把 16B tag 拼到密文尾）
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, compressed.as_ref())
        .map_err(|e| format!("AES-GCM 加密失败: {}", e))?;

    // 5. 组装 64B Header
    let mut header = vec![0u8; HEADER_SIZE];
    header[0..MAGIC_LEN].copy_from_slice(MAGIC);
    header[SALT_OFF..SALT_OFF + SALT_LEN].copy_from_slice(&salt);
    header[NONCE_OFF..NONCE_OFF + NONCE_LEN].copy_from_slice(&nonce_bytes);

    // 6. Header (64B) + 密文
    let mut out = Vec::with_capacity(HEADER_SIZE + ciphertext.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密 .drawerbox 文件字节流，产出原始 JSON 字节流（解压后、未序列化）
pub fn decrypt_backup(backup_bytes: &[u8], master_password: &str) -> Result<Vec<u8>, String> {
    // 1. 长度校验：至少要有 Header + 1B 密文 + 16B tag
    if backup_bytes.len() < HEADER_SIZE + 1 + 16 {
        return Err("备份文件损坏：大小不足".to_string());
    }

    // 2. Magic 校验（防止让用户选了非 .drawerbox 文件）
    if &backup_bytes[0..MAGIC_LEN] != MAGIC {
        return Err("不是有效的抽屉柜备份文件（缺少 DRAWERBOX1 标识）".to_string());
    }

    // 3. 读出 Header 里的 salt + nonce
    let salt = &backup_bytes[SALT_OFF..SALT_OFF + SALT_LEN];
    let nonce_bytes = &backup_bytes[NONCE_OFF..NONCE_OFF + NONCE_LEN];

    // 4. 派生密钥
    let key = crypto::derive_key(master_password, salt);

    // 5. AES-GCM 解密（密码错误、文件损坏都会在这里 Err）
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let ciphertext = &backup_bytes[HEADER_SIZE..];
    let compressed = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "主密码错误，或备份文件已损坏".to_string())?;

    // 6. zstd 解压
    let json_bytes = zstd::decode_all(std::io::Cursor::new(compressed))
        .map_err(|e| format!("备份文件损坏（解压失败）: {}", e))?;

    Ok(json_bytes)
}

/// v2 备份：独立 Magic + 显式版本，并把整个 Header 作为 AES-GCM AAD。
/// 因此 header/version/salt/nonce/body 任一字节变化都会关闭恢复路径。
pub fn encrypt_backup_v2(payload: &[u8], master_password: &str) -> Result<Vec<u8>, String> {
    let compressed = zstd::encode_all(payload, ZSTD_LEVEL)
        .map_err(|_| "无法压缩 v2 备份".to_string())?;
    let salt = crypto::generate_salt();
    let nonce_bytes = crypto::generate_nonce();
    let mut header = vec![0u8; HEADER_SIZE];
    header[..MAGIC_LEN].copy_from_slice(MAGIC_V2);
    header[SALT_OFF..SALT_OFF + SALT_LEN].copy_from_slice(&salt);
    header[NONCE_OFF..NONCE_OFF + NONCE_LEN].copy_from_slice(&nonce_bytes);
    header[VERSION_OFF..VERSION_OFF + 4].copy_from_slice(&V2_BACKUP_VERSION.to_le_bytes());
    // 固定算法标识：1=Argon2id(default v1), 1=zstd, 1=AES-256-GCM。
    header[58] = 1;
    header[59] = 1;
    header[60] = 1;

    let key = Zeroizing::new(crypto::derive_key(master_password, &salt));
    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|_| "无法加密 v2 备份".to_string())?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload { msg: &compressed, aad: &header },
        )
        .map_err(|_| "无法加密 v2 备份".to_string())?;
    let mut out = Vec::with_capacity(HEADER_SIZE + ciphertext.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt_backup_v2(backup_bytes: &[u8], master_password: &str) -> Result<Vec<u8>, String> {
    if backup_bytes.len() < HEADER_SIZE + 17
        || classify_backup(backup_bytes) != BackupKind::StableDekV2
    {
        return Err("不是受支持的 v2 备份文件".to_string());
    }
    let version = u32::from_le_bytes(
        backup_bytes[VERSION_OFF..VERSION_OFF + 4]
            .try_into()
            .map_err(|_| "v2 备份版本无效".to_string())?,
    );
    if version != V2_BACKUP_VERSION
        || backup_bytes[58] != 1
        || backup_bytes[59] != 1
        || backup_bytes[60] != 1
    {
        return Err("不支持的备份版本".to_string());
    }
    let header = &backup_bytes[..HEADER_SIZE];
    let salt = &backup_bytes[SALT_OFF..SALT_OFF + SALT_LEN];
    let nonce_bytes = &backup_bytes[NONCE_OFF..NONCE_OFF + NONCE_LEN];
    let key = Zeroizing::new(crypto::derive_key(master_password, salt));
    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|_| "无法读取 v2 备份".to_string())?;
    let compressed = cipher
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload { msg: &backup_bytes[HEADER_SIZE..], aad: header },
        )
        .map_err(|_| "主密码错误，或备份文件已损坏".to_string())?;
    zstd::decode_all(std::io::Cursor::new(compressed))
        .map_err(|_| "备份文件损坏".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_roundtrip_ok() {
        let payload = json!({"apps":[1,2,3],"passwords":[{"id":1}]});
        let raw = serde_json::to_vec(&payload).unwrap();
        let enc = encrypt_backup(&raw, "hello123").unwrap();
        assert!(enc.len() > HEADER_SIZE);
        // magic 对
        assert_eq!(&enc[0..10], b"DRAWERBOX1");
        let dec = decrypt_backup(&enc, "hello123").unwrap();
        let got: serde_json::Value = serde_json::from_slice(&dec).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn test_wrong_password_fails() {
        let raw = serde_json::to_vec(&json!({"a":1})).unwrap();
        let enc = encrypt_backup(&raw, "pw1").unwrap();
        assert!(decrypt_backup(&enc, "pw2").is_err());
    }

    #[test]
    fn test_bad_magic_fails() {
        let mut enc = encrypt_backup(b"{}", "pw").unwrap();
        enc[0] = b'X';
        assert!(decrypt_backup(&enc, "pw").is_err());
    }

    #[test]
    fn v2_header_and_body_are_authenticated() {
        let raw = br#"{"backup_version":2}"#;
        let enc = encrypt_backup_v2(raw, "pw-v2").unwrap();
        assert_eq!(classify_backup(&enc), BackupKind::StableDekV2);
        assert_eq!(decrypt_backup_v2(&enc, "pw-v2").unwrap(), raw);
        assert!(decrypt_backup_v2(&enc, "wrong").is_err());

        for index in [0usize, VERSION_OFF, SALT_OFF, NONCE_OFF, HEADER_SIZE] {
            let mut tampered = enc.clone();
            tampered[index] ^= 1;
            assert!(decrypt_backup_v2(&tampered, "pw-v2").is_err());
        }
    }
}
