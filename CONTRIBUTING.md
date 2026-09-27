# 贡献指南（Contributing）

感谢关注抽屉柜 Drawer！当前项目处于早期阶段，欢迎 Issue 与 Pull Request。

## 开发环境

- Node.js 18+ 与 [pnpm](https://pnpm.io/)
- Rust stable（`x86_64-pc-windows-msvc` 工具链）
- [Tauri 2 系统依赖](https://tauri.app/start/prerequisites/)：WebView2（Windows 11 自带）与 Visual Studio Build Tools（"使用 C++ 的桌面开发"工作负载）

## 常用命令

```bash
pnpm install                  # 安装前端依赖
pnpm tauri dev                # 开发模式运行
pnpm tauri build              # 生产构建
pnpm build                    # vue-tsc 类型检查 + vite 前端构建
cd src-tauri && cargo test    # Rust 单元测试 / 密码契约测试 / 迁移测试
```

Windows 下生产构建也可使用 `scripts/build.ps1`。

## 提交规范

- Commit message：`type(scope): 中文描述`，type 取 `feat / fix / docs / chore / refactor / test`
- **密码、加密、数据迁移相关改动必须附带单元测试**，并保证 `cargo test` 全绿
- 不在提交中引入任何凭据、密钥、真实用户数据或本地绝对路径

## 流程

1. 先开 Issue 描述问题或建议，避免重复工作
2. Fork 仓库并新建分支（如 `feat/xxx`）
3. 提交 Pull Request，说明改动内容与验证方式
4. 安全问题请勿公开提交，走 [SECURITY.md](SECURITY.md) 的私密渠道

## 许可

向本项目提交贡献，即表示同意以 **GPL-3.0-only** 许可证授权这些贡献（与项目本体一致）。
