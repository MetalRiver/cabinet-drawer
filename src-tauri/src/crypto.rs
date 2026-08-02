use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{self, Argon2};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};

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