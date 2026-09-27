<div align="center">

# 抽屉柜 Drawer

**一个本地优先的 Windows 桌面「抽屉」：把应用、文件、网址、片段与密码收纳进一个小窗，随手可取。**

![Version](https://img.shields.io/badge/version-v0.3.2-blue)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11%20x64-lightgrey)
![License](https://img.shields.io/badge/license-GPL--3.0--only-green)
![Data](https://img.shields.io/badge/data-local--first-orange)
![Stack](https://img.shields.io/badge/Tauri_2_%C2%B7_Vue_3_%C2%B7_Rust-24C6DB)

无需账号 · 不依赖云端服务 · 无遥测 · 数据只存你的电脑

<!-- 产品截图：待真实运行截图素材就绪后加入，不使用合成图 -->

</div>

---

## 这是什么

抽屉柜（Drawer）是一个给 Windows 重度用户的桌面收纳工具：常用的应用程序、文件、文件夹、网址，每天要用的命令与片段，还有零零散散的密码，都收进一个轻量小窗里——能装就装下，能搜就搜到。离开电脑一键锁屏，密码字段加密存储。

## 为什么做 Drawer

- **桌面乱、找东西慢**：常用入口散落在桌面/开始菜单/浏览器书签各处，抽屉柜把它们收进一个窗口，全局搜索直达
- **重复操作多**：同一段命令、同一段文字每天敲几遍，放进片段库一键复制
- **密码想自己管**：不放心把密码放上云，抽屉柜的密码只存你本地数据库
- **不想多装五个工具**：启动器 + 片段库 + 便签 + 密码本，一个应用解决；不需要注册账号，装完即用

## 功能一览

| 模块 | 能力 |
| :--- | :--- |
| 🗂️ **应用** | 收纳应用程序 / 网址 URL / 文件夹，自动识别类型；可扫描开始菜单/桌面，也可拖入文件自动分类，点击即开 |
| ⭐ **常用** | 记录使用次数与最近使用时间，高频条目聚合直达 |
| 🔐 **密码** | 密码字段本地加密存储；查看/复制密码需重新验证主密码；支持标题/用户名/网址/备注 |
| 🗒️ **片段** | 常用文字、命令行、代码片段一键保存与复制 |
| 📝 **临时** | 草稿/便签暂存，3 天自动清理（可设置） |
| 🗑️ **回收站** | 删除的内容先进回收站，可恢复或永久删除 |
| 🔒 **锁屏** | 手动一键锁屏；闲置自动锁定（默认 5 分钟，可设置） |
| 🟢 **托盘** | 后台运行不占任务栏，托盘图标快速操作 |

## 下载安装（Windows 10/11 x64）

1. 前往 [Releases](https://github.com/MetalRiver/cabinet-drawer/releases) 页面，下载 `抽屉柜 Drawer_0.3.2_x64-setup.exe`（约 4.8 MB）
2. 双击运行安装向导，一路下一步即可（自动创建开始菜单 + 桌面图标，仅安装到当前用户目录）
3. 首次启动会引导你设置「主密码」，并生成 **12 个恢复词**——主密码要记好，恢复词请抄在纸上保存
4. 开始使用

> ⚠️ 安装包当前未做代码签名，Windows Defender / 杀毒软件可能提示"未知发布者"，属正常误报，点「仍要运行」即可。安装器内嵌 WebView2 引导器，若系统缺少 WebView2 运行时会提示联网安装。

## 数据与安全

- 所有数据存储在**本地 SQLite 单文件数据库**（`%APPDATA%\com.drawer-box.app\`），不上传任何服务器
- **加密边界**：仅密码字段在本地数据库中加密存储（AES-256-GCM，主密码经 Argon2id 派生数据密钥）；标题、用户名、网址、备注等其他元数据不是全库加密
- 查看/复制密码需要重新验证主密码
- 剪贴板复制密码后自动清空（默认 15 秒，可设置）
- 应用无遥测、无数据上报、不依赖云端服务

## 忘记密码与备份

- 首次启动生成 **12 个恢复词**：忘记主密码时，凭恢复词重置主密码，数据不丢
- 恢复词 = 数据命根：抄在纸上，不要截图、不要存网盘、**不要发给任何人（包括开发者）**
- 主密码与恢复词同时丢失，数据无法恢复——唯一出路是提前导出的备份
- 支持导出 `.drawerbox` 认证加密备份：导出时校验密码，避免"备份时存得进、恢复时取不出"；恢复同样需要密码验证

## 已知限制（当前版本 v0.3.2）

- 仅支持 Windows 10/11 x64，无 macOS / Linux 版本
- 无多设备同步、无云备份、无跨平台、无 AI 功能
- 无应用内自动更新，新版本需手动下载覆盖安装
- 安装包未做代码签名，可能触发杀毒软件提示
- 早期原型曾演示"贴边自动隐藏"，**当前版本不包含该功能**
- 未强制单实例：请避免同时打开多个抽屉柜窗口
- 数据库文件损坏时应用会启动失败——请养成定期导出备份的习惯

## 从源码构建

### 环境要求

- Node.js 18+ 与 [pnpm](https://pnpm.io/)
- Rust stable（`x86_64-pc-windows-msvc` 工具链）
- [Tauri 2 系统依赖](https://tauri.app/start/prerequisites/)：WebView2（Windows 11 自带）与 Visual Studio Build Tools（"使用 C++ 的桌面开发"工作负载）

### 常用命令

```bash
pnpm install          # 安装前端依赖
pnpm tauri dev        # 开发模式运行
pnpm tauri build      # 生产构建，产物在 src-tauri/target/release/bundle/nsis/
```

Windows 下也可使用 `scripts/build.ps1`（自动加载 MSVC 环境）。

### 测试

```bash
pnpm build                    # vue-tsc 类型检查 + vite 前端构建
cd src-tauri && cargo test    # Rust 单元测试 / 密码契约测试 / 迁移测试
```

## 反馈与贡献

- Bug 与功能建议：[GitHub Issues](https://github.com/MetalRiver/cabinet-drawer/issues) / [Gitee Issues](https://gitee.com/Aa990602/cabinet-drawer/issues)
- **安全问题请勿在公开 Issue 中描述细节**，见 [SECURITY.md](SECURITY.md)
- 参与开发请先读 [CONTRIBUTING.md](CONTRIBUTING.md)，欢迎 Pull Request

## License

本项目源代码以 **GPL-3.0-only**（GNU General Public License v3.0 仅此版本）授权发布，完整许可证文本见 [LICENSE](LICENSE)。

- 本程序按"现状"提供，不附带任何担保；分发/修改须遵循 GPL-3.0 条款
- 第三方组件的版权与许可声明见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES)
- **品牌资产边界**：GPL-3.0-only 仅授权本仓库的源代码；"抽屉柜 Drawer" 的名称、Logo 及品牌视觉资产不因源代码的 GPL 授权而自动获得使用许可，在品牌项目中使用需另行获得项目作者许可

## Community

### 途有引力 PathOrbit

![微信公众号：途有引力 PathOrbit（微信搜一搜）](docs/assets/wechat-official-account.png)

X: [@PathOrbit](https://x.com/PathOrbit)

- Bug / Feature：[GitHub Issues](https://github.com/MetalRiver/cabinet-drawer/issues) · [Gitee Issues](https://gitee.com/Aa990602/cabinet-drawer/issues)
- Security：请勿在公开 Issue 描述细节，见 [SECURITY.md](SECURITY.md)
