//! ===== Tauri IPC Commands（按领域分模块） =====
//! 说明：
//! - 所有 #[tauri::command] 函数名、参数、返回值 100% 保持与 lib.rs 原版一致
//! - 每个文件 pub fn 命令（全部 pub 可见性，这里 pub use * 后 crate 根可见）
//! - 最终 lib.rs 会 `use crate::commands::*;` 后，generate_handler!() 直接拿到全部
//! - 前端 invoke("xxx") 完全无感

pub mod auth;
pub mod passwords;
pub mod window;
pub mod snippets;
pub mod temp;
pub mod trash;
pub mod apps;
pub mod settings_backup;
pub mod data_root_setup;

// 把所有领域的命令 pub use 到 commands 模块作用域
pub use auth::*;
pub use passwords::*;
pub use window::*;
pub use snippets::*;
pub use temp::*;
pub use trash::*;
pub use apps::*;
pub use settings_backup::*;
pub use data_root_setup::*;
