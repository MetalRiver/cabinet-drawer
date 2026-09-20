use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{self, Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hkdf::Hkdf;
use rand::Rng;
use sha2::{Digest, Sha256};
use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

const NONCE_SIZE: usize = 12;
const SALT_SIZE: usize = 32;

/// 从主密码派生加密密钥（使用 Argon2id）
pub fn derive_key(master_password: &str, salt: &[u8]) -> Vec<u8> {
    let mut key = vec![0u8; 32];
    Argon2::default()
        .hash_password_into(master_password.as_bytes(), salt, &mut key)
        .expect("argon2 key derivation failed");
    key
}

/// 生成随机盐
pub fn generate_salt() -> Vec<u8> {
    let mut salt = vec![0u8; SALT_SIZE];
    rand::thread_rng().fill(&mut salt[..]);
    salt
}

/// 生成随机 nonce
pub fn generate_nonce() -> Vec<u8> {
    let mut nonce = vec![0u8; NONCE_SIZE];
    rand::thread_rng().fill(&mut nonce[..]);
    nonce
}

/// 加密数据，返回 base64(nonce + ciphertext)
pub fn encrypt(plaintext: &str, key: &[u8]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let nonce_bytes = generate_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| e.to_string())?;

    // 拼接 nonce + ciphertext 后 base64 编码
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

/// 解密数据，输入 base64(nonce + ciphertext)
pub fn decrypt(encrypted: &str, key: &[u8]) -> Result<String, String> {
    let combined = BASE64.decode(encrypted).map_err(|e| e.to_string())?;
    if combined.len() < NONCE_SIZE {
        return Err("invalid ciphertext".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| e.to_string())?;

    String::from_utf8(plaintext).map_err(|e| e.to_string())
}

/// 哈希主密码（用于存储校验）
pub fn hash_password(password: &str, salt: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    BASE64.encode(result)
}

/// BIP39 风格的简化词库（256 个常用词，足以做 12 词恢复短语）
const WORD_LIST: &[&str] = &[
    "abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract",
    "absurd", "abuse", "access", "accident", "account", "accuse", "achieve", "acid",
    "acoustic", "acquire", "across", "act", "action", "actor", "actress", "actual",
    "adapt", "add", "addict", "address", "adjust", "admit", "adult", "advance",
    "advice", "aerobic", "affair", "afford", "afraid", "again", "age", "agent",
    "agree", "ahead", "aim", "air", "airport", "aisle", "alarm", "album",
    "alcohol", "alert", "alien", "all", "alley", "allow", "almost", "alone",
    "alpha", "already", "also", "alter", "always", "amateur", "amazing", "among",
    "amount", "amused", "analyst", "anchor", "ancient", "anger", "angle", "angry",
    "animal", "ankle", "announce", "annual", "another", "answer", "antenna", "antique",
    "anxiety", "any", "apart", "apology", "appear", "apple", "approve", "april",
    "arch", "arctic", "area", "arena", "argue", "arm", "armed", "armor",
    "army", "around", "arrange", "arrest", "arrive", "arrow", "art", "artefact",
    "artist", "artwork", "ask", "aspect", "assault", "asset", "assist", "assume",
    "asthma", "athlete", "atom", "attack", "attend", "attitude", "attract", "auction",
    "audit", "august", "aunt", "author", "auto", "autumn", "average", "avocado",
    "avoid", "awake", "aware", "away", "awesome", "awful", "awkward", "axis",
    "baby", "bachelor", "bacon", "badge", "bag", "balance", "balcony", "ball",
    "bamboo", "banana", "banner", "bar", "barely", "bargain", "barrel", "basic",
    "basket", "battle", "beach", "bean", "beauty", "because", "become", "beef",
    "before", "begin", "behave", "behind", "believe", "below", "belt", "bench",
    "benefit", "best", "betray", "better", "between", "beyond", "bicycle", "bid",
    "bike", "bind", "biology", "bird", "birth", "bitter", "black", "blade",
    "blame", "blanket", "blast", "bleak", "bless", "blind", "blood", "blossom",
    "blouse", "blue", "blur", "blush", "board", "boat", "body", "boil",
    "bomb", "bone", "bonus", "book", "boost", "border", "boring", "borrow",
    "boss", "bottom", "bounce", "box", "boy", "bracket", "brain", "brand",
    "brass", "brave", "bread", "breeze", "brick", "bridge", "brief", "bright",
    "bring", "brisk", "broccoli", "broken", "bronze", "broom", "brother", "brown",
    "brush", "bubble", "buddy", "budget", "buffalo", "build", "bulb", "bulk",
];

/// 生成 N 个随机恢复词
pub fn generate_recovery_words(count: usize) -> Vec<String> {
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| WORD_LIST[rng.gen_range(0..WORD_LIST.len())].to_string())
        .collect()
}

/// 评估密码强度（0-100）
pub fn password_strength(password: &str) -> u8 {
    if password.is_empty() {
        return 0;
    }
    let mut score: u32 = 0;
    let len = password.chars().count() as u32;

    // 长度分
    score += (len * 4).min(40);

    // 大写字母
    if password.chars().any(|c| c.is_ascii_uppercase()) {
        score += 10;
    }
    // 小写字母
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        score += 10;
    }
    // 数字
    if password.chars().any(|c| c.is_ascii_digit()) {
        score += 10;
    }
    // 特殊字符
    if password.chars().any(|c| !c.is_alphanumeric()) {
        score += 15;
    }
    // 长度 >= 12
    if len >= 12 {
        score += 15;
    }
    // 长度 >= 8
    if len >= 8 {
        score += 5;
    }

    score.min(100) as u8
}

/// 生成随机密码
pub fn generate_password(length: usize, use_upper: bool, use_lower: bool, use_digits: bool, use_symbols: bool) -> String {
    let mut charset = String::new();
    if use_upper { charset.push_str("ABCDEFGHJKLMNPQRSTUVWXYZ"); }
    if use_lower { charset.push_str("abcdefghijkmnpqrstuvwxyz"); }
    if use_digits { charset.push_str("23456789"); }
    if use_symbols { charset.push_str("!@#$%^&*-_=+"); }
    if charset.is_empty() { charset.push_str("abcdefghijkmnpqrstuvwxyz"); }

    let charset_bytes = charset.as_bytes();
    let mut rng = rand::thread_rng();
    (0..length)
        .map(|_| charset_bytes[rng.gen_range(0..charset_bytes.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt() {
        let salt = generate_salt();
        let key = derive_key("test_password_123", &salt);
        let plaintext = "Hello, World! 你好世界";
        let encrypted = encrypt(plaintext, &key).unwrap();
        let decrypted = decrypt(&encrypted, &key).unwrap();
        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_different_keys_fail() {
        let salt1 = generate_salt();
        let salt2 = generate_salt();
        let key1 = derive_key("password1", &salt1);
        let key2 = derive_key("password2", &salt2);
        let encrypted = encrypt("secret", &key1).unwrap();
        assert!(decrypt(&encrypted, &key2).is_err());
    }
}

// ============================================================
// Phase 2A：Stable DEK 安全核心（设计稿 v3 终稿）
// 契约：Master KEK=Argon2id 单用途直用；Recovery KEK=HKDF(entropy)；
// verifier 取消（认证解封=验证）；错误统一 SecretError。
// ============================================================

/// KDF 参数（版本化，持久化于 settings.kdf_params_m / kdf_params_r）
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KdfParams {
    pub algo: String,
    pub version: u32,
    pub m_cost: u32,
    pub t_cost: u32,
    pub p_cost: u32,
    /// Base64(Salt)
    pub salt: String,
}

/// 内部统一错误：Display 不区分失败原因（不泄漏密码学细节）
#[derive(Debug, PartialEq)]
pub enum SecretError {
    AuthFailed,
    InvalidFormat,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "安全数据校验失败")
    }
}

impl std::error::Error for SecretError {}

impl From<SecretError> for String {
    fn from(e: SecretError) -> String {
        e.to_string()
    }
}

/// 当前推荐 KDF 参数（v1：Argon2id OWASP 最小推荐配置；版本化可升级）
pub fn default_kdf_params(salt_b64: String) -> KdfParams {
    KdfParams {
        algo: "argon2id".into(),
        version: 1,
        m_cost: 19456,
        t_cost: 2,
        p_cost: 1,
        salt: salt_b64,
    }
}

/// Master KEK：Argon2id(master_password 原始 UTF-8 bytes, salt, params) → 32B
/// 输出单用途（包裹 DEK），不套 HKDF；用完即 zeroize
pub fn derive_master_kek(password: &str, params: &KdfParams) -> Zeroizing<Vec<u8>> {
    // 主密码按用户实际输入的原始 UTF-8 bytes 派生（不做 Unicode normalization，
    // 避免 normalization 规则变化导致原密码无法解锁）
    let mut salt = BASE64.decode(&params.salt).unwrap_or_default();
    // 盐损坏（<8B）时用占位盐派生 → KEK 必然错误 → unwrap 认证失败关闭（不 panic）
    let argon2_params = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
        .unwrap_or_else(|_| Params::default()); // 损坏参数 → 回落默认（派生错误 KEK → 后续认证失败关闭）
    if salt.len() < 8 {
        salt = vec![0xff; 32];
    }
    let mut kek = vec![0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params)
        .hash_password_into(password.as_bytes(), &salt, &mut kek)
        .expect("Argon2 派生（参数合法时）必须成功");
    Zeroizing::new(kek)
}

/// DEK：32B CSPRNG 随机，与任何密码/短语无派生关系
pub fn generate_dek() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(rand::thread_rng().gen::<[u8; 32]>().to_vec())
}

pub const WRAP_DEK_PREFIX: &str = "DWK1:";
pub const AAD_WRAP_MASTER: &str = "drawer|wrapped-dek|master|v1";
pub const AAD_WRAP_RECOVERY: &str = "drawer|wrapped-dek|recovery|v1";

/// 包裹 DEK：AES-GCM(KEK, DEK)，独立随机 nonce
/// 序列化 = "DWK1:<Base64(nonce ‖ ct+tag)>"；AAD 由调用方传入（master/recovery 各异）
pub fn wrap_dek(dek: &[u8], kek: &[u8], aad: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(kek).map_err(|e| e.to_string())?;
    let nonce_bytes = rand::thread_rng().gen::<[u8; 12]>();
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            aes_gcm::aead::Payload { msg: dek, aad: aad.as_bytes() },
        )
        .map_err(|e| e.to_string())?;
    let mut raw = nonce_bytes.to_vec();
    raw.extend_from_slice(&ct);
    Ok(format!("{}{}", WRAP_DEK_PREFIX, BASE64.encode(raw)))
}

/// 解封 DEK：认证解封成功 = KEK 正确。任何失败统一 SecretError
pub fn unwrap_dek(serialized: &str, kek: &[u8], aad: &str) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    let body = serialized.strip_prefix(WRAP_DEK_PREFIX).ok_or(SecretError::InvalidFormat)?;
    let raw = BASE64.decode(body).map_err(|_| SecretError::InvalidFormat)?;
    if raw.len() < 12 + 16 {
        return Err(SecretError::InvalidFormat);
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(kek).map_err(|_| SecretError::AuthFailed)?;
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), aes_gcm::aead::Payload { msg: ct, aad: aad.as_bytes() })
        .map_err(|_| SecretError::AuthFailed)?;
    Ok(Zeroizing::new(pt))
}

pub const RECOVERY_KEK_INFO: &str = "drawer/recovery-kek/v1";

/// Base64 编码/解码（内部与迁移共用）
pub fn b64_encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    BASE64.decode(s).map_err(|e| e.to_string())
}

/// 恢复熵：128 bit CSPRNG（BIP39 12 词的熵来源）
pub fn generate_recovery_entropy() -> Zeroizing<Vec<u8>> {
    Zeroizing::new(rand::thread_rng().gen::<[u8; 16]>().to_vec())
}

/// entropy(16B) → BIP39 英文 12 词（含 4 bit 内置校验）
pub fn entropy_to_mnemonic(entropy: &[u8]) -> Result<String, String> {
    let m = Mnemonic::from_entropy(entropy, Language::English).map_err(|e| e.to_string())?;
    Ok(m.into_phrase())
}

/// 12 词 → 归一化 → 词表+checksum 校验 → entropy(16B)；失败统一 InvalidFormat
pub fn mnemonic_to_entropy(phrase: &str) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    // 归一化：折叠空白 + 小写（BIP39 词表全小写）；词数/词表/checksum 全部校验
    let normalized = phrase.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let m = Mnemonic::from_phrase(normalized, Language::English).map_err(|_| SecretError::InvalidFormat)?;
    Ok(Zeroizing::new(m.entropy().to_vec()))
}

/// Recovery KEK：HKDF-SHA256(ikm=entropy, salt=salt_r, info=RECOVERY_KEK_INFO) → 32B
pub fn derive_recovery_kek(entropy: &[u8], salt_r: &[u8]) -> Zeroizing<Vec<u8>> {
    let hk = Hkdf::<Sha256>::new(Some(salt_r), entropy);
    let mut okm = vec![0u8; 32];
    hk.expand(RECOVERY_KEK_INFO.as_bytes(), &mut okm)
        .expect("HKDF 输出长度合法");
    Zeroizing::new(okm)
}

/// 包裹 DEK（Recovery 路径便捷组合）：derive_recovery_kek + wrap_dek
pub fn wrap_dek_recovery(dek: &[u8], entropy: &[u8], salt_r: &[u8]) -> Result<String, String> {
    let kek = derive_recovery_kek(entropy, salt_r);
    wrap_dek(dek, &kek, AAD_WRAP_RECOVERY)
}

/// 解封 DEK（Recovery 路径便捷组合）：derive_recovery_kek + unwrap_dek
pub fn unwrap_dek_recovery(serialized: &str, entropy: &[u8], salt_r: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    let kek = derive_recovery_kek(entropy, salt_r);
    unwrap_dek(serialized, &kek, AAD_WRAP_RECOVERY)
}

pub const PASSWORD_PREFIX_DW2: &str = "DW2:";

/// 行级 AAD：绑定格式版本 + 条目类型 + 永久 record_uuid
pub fn password_aad(record_uuid: &str) -> String {
    format!("drawer|password|DW2|{}", record_uuid)
}

/// 行加密（DW2 格式）：AES-GCM(DEK, 明文, AAD)
pub fn encrypt_password_dw2(plaintext: &str, dek: &[u8], record_uuid: &str) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(dek).map_err(|e| e.to_string())?;
    let nonce_bytes = rand::thread_rng().gen::<[u8; 12]>();
    let ct = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            aes_gcm::aead::Payload { msg: plaintext.as_bytes(), aad: password_aad(record_uuid).as_bytes() },
        )
        .map_err(|e| e.to_string())?;
    let mut raw = nonce_bytes.to_vec();
    raw.extend_from_slice(&ct);
    Ok(format!("{}{}", PASSWORD_PREFIX_DW2, BASE64.encode(raw)))
}

/// 行解密（DW2 格式）：认证失败/格式错误统一 SecretError
pub fn decrypt_password_dw2(dw2: &str, dek: &[u8], record_uuid: &str) -> Result<String, SecretError> {
    let body = dw2.strip_prefix(PASSWORD_PREFIX_DW2).ok_or(SecretError::InvalidFormat)?;
    let raw = BASE64.decode(body).map_err(|_| SecretError::InvalidFormat)?;
    if raw.len() < 12 + 16 {
        return Err(SecretError::InvalidFormat);
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(dek).map_err(|_| SecretError::AuthFailed)?;
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), aes_gcm::aead::Payload { msg: ct, aad: password_aad(record_uuid).as_bytes() })
        .map_err(|_| SecretError::AuthFailed)?;
    String::from_utf8(pt).map_err(|_| SecretError::AuthFailed)
}

#[cfg(test)]
mod phase2a_tests {
    use super::*;

    const PW: &str = "Phase2A-Master-Pw!";
    fn salt_b64() -> String {
        BASE64.encode(b"0123456789abcdef0123456789abcdef")
    }
    fn params_with(salt: &str) -> KdfParams {
        KdfParams { algo: "argon2id".into(), version: 1, m_cost: 19456, t_cost: 2, p_cost: 1, salt: salt.into() }
    }

    // ===== T1：Master KEK derivation =====
    #[test]
    fn t1_master_kek_deterministic() {
        let a = derive_master_kek(PW, &params_with(&salt_b64()));
        let b = derive_master_kek(PW, &params_with(&salt_b64()));
        assert_eq!(*a, *b, "同 password+salt+params 必须产生相同 KEK");
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn t1_master_kek_differs_by_password() {
        let a = derive_master_kek(PW, &params_with(&salt_b64()));
        let b = derive_master_kek("other-password", &params_with(&salt_b64()));
        assert_ne!(*a, *b);
    }

    #[test]
    fn t1_master_kek_differs_by_salt() {
        let a = derive_master_kek(PW, &params_with(&salt_b64()));
        let b = derive_master_kek(PW, &params_with(&BASE64.encode(b"ffffffffffffffffffffffffffffffff")));
        assert_ne!(*a, *b);
    }

    #[test]
    fn t1_master_kek_params_versioned() {
        let salt = salt_b64();
        let mut p3 = params_with(&salt);
        p3.t_cost = 3;
        let a = derive_master_kek(PW, &params_with(&salt));
        let b = derive_master_kek(PW, &p3);
        assert_ne!(*a, *b, "KDF 参数版本必须影响输出");
        let json = serde_json::to_string(&params_with(&salt)).unwrap();
        let back: KdfParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, params_with(&salt), "KdfParams 必须可 JSON 持久化");
    }

    // ===== T2：DEK wrap / unwrap =====
    #[test]
    fn t2_wrap_unwrap_roundtrip() {
        let kek = derive_master_kek(PW, &params_with(&salt_b64()));
        let dek = generate_dek();
        assert_eq!(dek.len(), 32);
        let wrapped = wrap_dek(&dek, &kek, AAD_WRAP_MASTER).unwrap();
        assert!(wrapped.starts_with(WRAP_DEK_PREFIX));
        let back = unwrap_dek(&wrapped, &kek, AAD_WRAP_MASTER).unwrap();
        assert_eq!(*back, *dek);
    }

    #[test]
    fn t2_wrong_kek_rejected() {
        let kek = derive_master_kek(PW, &params_with(&salt_b64()));
        let bad = derive_master_kek("wrong", &params_with(&salt_b64()));
        let dek = generate_dek();
        let wrapped = wrap_dek(&dek, &kek, AAD_WRAP_MASTER).unwrap();
        assert!(unwrap_dek(&wrapped, &bad, AAD_WRAP_MASTER).is_err());
    }

    #[test]
    fn t2_tamper_rejected() {
        let kek = derive_master_kek(PW, &params_with(&salt_b64()));
        let dek = generate_dek();
        let wrapped = wrap_dek(&dek, &kek, AAD_WRAP_MASTER).unwrap();
        let body = wrapped.strip_prefix(WRAP_DEK_PREFIX).unwrap();
        let mut raw = BASE64.decode(body).unwrap();
        let n = raw.len();
        let mut v1 = raw.clone(); v1[3] ^= 0xff;
        let mut v2 = raw.clone(); v2[n - 20] ^= 0xff;
        let mut v3 = raw.clone(); v3[n - 1] ^= 0xff;
        for (name, v) in [("nonce", v1), ("ciphertext", v2), ("tag", v3)] {
            let s = format!("{}{}", WRAP_DEK_PREFIX, BASE64.encode(&v));
            assert!(unwrap_dek(&s, &kek, AAD_WRAP_MASTER).is_err(), "{} 篡改必须失败", name);
        }
        let s = format!("DWK9:{}", body);
        assert!(unwrap_dek(&s, &kek, AAD_WRAP_MASTER).is_err(), "未知版本必须失败");
        assert!(unwrap_dek(&wrapped, &kek, AAD_WRAP_RECOVERY).is_err(), "AAD 不匹配必须失败");
    }

    #[test]
    fn t2_error_surface_unified() {
        let kek = derive_master_kek(PW, &params_with(&salt_b64()));
        let dek = generate_dek();
        let wrapped = wrap_dek(&dek, &kek, AAD_WRAP_MASTER).unwrap();
        let e1 = unwrap_dek(&wrapped, &kek, AAD_WRAP_RECOVERY).unwrap_err().to_string();
        let e2 = unwrap_dek("DWK1:!!!notbase64", &kek, AAD_WRAP_MASTER).unwrap_err().to_string();
        let e3 = unwrap_dek(&wrapped, &derive_master_kek("x", &params_with(&BASE64.encode(b"003456789abcdef0123456789abcdef"))), AAD_WRAP_MASTER).unwrap_err().to_string();
        assert_eq!(e1, e2, "失败原因不得通过错误消息区分");
        assert_eq!(e2, e3);
        assert!(!e1.contains("nonce") && !e1.contains("key") && !e1.contains("tag"));
    }

    // ===== T3：Recovery Phrase（BIP39 编码） =====
    #[test]
    fn t3_entropy_mnemonic_roundtrip() {
        let entropy = generate_recovery_entropy();
        assert_eq!(entropy.len(), 16, "128 bit");
        let phrase = entropy_to_mnemonic(&entropy).unwrap();
        assert_eq!(phrase.split_whitespace().count(), 12);
        let back = mnemonic_to_entropy(&phrase).unwrap();
        assert_eq!(*back, *entropy);
    }

    #[test]
    fn t3_word_count_rejected() {
        let entropy = generate_recovery_entropy();
        let phrase = entropy_to_mnemonic(&entropy).unwrap();
        let words: Vec<&str> = phrase.split_whitespace().collect();
        assert!(mnemonic_to_entropy(&words[..11].join(" ")).is_err(), "11 词必须拒绝");
        assert!(mnemonic_to_entropy(&format!("{} {}", phrase, words[0])).is_err(), "13 词必须拒绝");
    }

    #[test]
    fn t3_unknown_word_rejected() {
        assert!(mnemonic_to_entropy("this-is-not-a-bip39-word list entry alpha beta gamma delta").is_err());
    }

    #[test]
    fn t3_checksum_and_order_rejected() {
        let entropy = generate_recovery_entropy();
        let phrase = entropy_to_mnemonic(&entropy).unwrap();
        let words: Vec<&str> = phrase.split_whitespace().collect();
        let swapped = format!("{} {}", words[1], words[0]);
        if swapped != phrase {
            assert!(mnemonic_to_entropy(&swapped).is_err(), "顺序错误/校验和错误必须拒绝");
        }
    }

    #[test]
    fn t3_whitespace_normalization_ok() {
        let entropy = generate_recovery_entropy();
        let phrase = entropy_to_mnemonic(&entropy).unwrap();
        let noisy = format!("  {}  ", phrase.replace(' ', "   "));
        let back = mnemonic_to_entropy(&noisy).unwrap();
        assert_eq!(*back, *entropy, "普通空格差异必须被归一化");
    }

    // ===== T4：Recovery KEK + wrapped DEK =====
    #[test]
    fn t4_recovery_roundtrip_same_dek_as_master() {
        let salt_r = b"recovery-salt-r-16b";
        let entropy = generate_recovery_entropy();
        let phrase = entropy_to_mnemonic(&entropy).unwrap();
        let kek_r = derive_recovery_kek(mnemonic_to_entropy(&phrase).unwrap().as_slice(), salt_r);
        let dek = generate_dek();
        let wrapped = wrap_dek(&dek, &kek_r, AAD_WRAP_RECOVERY).unwrap();
        let back = unwrap_dek_recovery(&wrapped, mnemonic_to_entropy(&phrase).unwrap().as_slice(), salt_r).unwrap();
        assert_eq!(*back, *dek);
        let kek_m = derive_master_kek(PW, &params_with(&salt_b64()));
        let w_m = wrap_dek(&dek, &kek_m, AAD_WRAP_MASTER).unwrap();
        assert_eq!(*unwrap_dek(&w_m, &kek_m, AAD_WRAP_MASTER).unwrap(), *back, "两条包裹路径解出同一 DEK");
    }

    #[test]
    fn t4_different_entropy_different_kek() {
        let salt_r = b"recovery-salt-r-16b";
        let e1 = generate_recovery_entropy();
        let e2 = generate_recovery_entropy();
        assert_ne!(*derive_recovery_kek(e1.as_slice(), salt_r), *derive_recovery_kek(e2.as_slice(), salt_r));
    }

    #[test]
    fn t4_rotation_touches_only_recovery_wrap() {
        let entropy_old = generate_recovery_entropy();
        let entropy_new = generate_recovery_entropy();
        let dek = generate_dek();
        let w_old = wrap_dek(&dek, &derive_recovery_kek(entropy_old.as_slice(), b"old-salt-r-16b"), AAD_WRAP_RECOVERY).unwrap();
        let w_new = wrap_dek(&dek, &derive_recovery_kek(entropy_new.as_slice(), b"new-salt-r-16b"), AAD_WRAP_RECOVERY).unwrap();
        assert_ne!(w_old, w_new, "轮换后 wrapped_dek_r 必须变化");
        assert_eq!(*unwrap_dek_recovery(&w_new, entropy_new.as_slice(), b"new-salt-r-16b").unwrap(), *dek, "新短语解出同一 DEK");
        assert!(unwrap_dek_recovery(&w_new, entropy_old.as_slice(), b"old-salt-r-16b").is_err(), "旧短语立即失效");
    }
}