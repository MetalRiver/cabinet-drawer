//! 应用目录名解析（G7-2 隔离修正）
//!
//! 背景：图标缓存根与前端日志路径曾硬编码 `com.drawer-box.app`，导致任何以
//! 其他 identifier 构建的候选/验收版都会写穿到正式版目录（G7-2 审查发现的隔离缺口）。
//!
//! 机制：lib.rs setup 最早期注入 tauri config 的真实 identifier；
//! `app_dir_name()` 返回该值。未注入时回退生产目录名——保证：
//! 1. 正式版（identifier=com.drawer-box.app）解析结果与历史行为**逐字节一致**；
//! 2. 任何在 setup 之前意外调用本模块的路径也回落到生产名（不会写到陌生目录）。
//!
//! 候选版（identifier=com.drawer-box.candidate）由此获得：
//!   图标缓存  %LOCALAPPDATA%\com.drawer-box.candidate\icons   ← 与资产 scope `$APPLOCALDATA/icons/**/*` 一致
//!   前端日志  %APPDATA%\com.drawer-box.candidate\frontend.log ← 与 Config Root（app_data_dir）同身份

use std::sync::OnceLock;

static APP_DIR_NAME: OnceLock<String> = OnceLock::new();

/// 生产应用目录名（历史硬编码值；回退默认，保证正式版零漂移）
pub const PROD_APP_DIR_NAME: &str = "com.drawer-box.app";

/// lib.rs setup 最早期调用：注入 tauri config 的真实 identifier。
/// 幂等（OnceLock）：重复调用仅首次生效。
pub fn set_app_dir_name(identifier: &str) {
    let _ = APP_DIR_NAME.set(identifier.to_string());
}

/// 当前构建的应用数据目录名（identifier）。
/// 未初始化时回退生产名（不会 panic、不会写到未知目录）。
pub fn app_dir_name() -> &'static str {
    APP_DIR_NAME.get().map(String::as_str).unwrap_or(PROD_APP_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_prod_name() {
        // OnceLock 未注入（测试进程独立）→ 回退生产名
        assert_eq!(app_dir_name(), PROD_APP_DIR_NAME);
    }
}
