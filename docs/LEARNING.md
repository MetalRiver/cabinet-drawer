# 抽屉柜（Wind-zp）学习与定位指南

> 用途：把"市场调研 + 优秀项目分析 + 打包要点 + GitHub 入门"一次性整理好，按需查阅。
> 阅读时间：约 25 分钟。
> 凡是 `【大白话】` 注释的术语，都是我帮你翻成人话了，不用查百度。

---

## 目录

1. [市场调研：同类工具与用户痛点](#1-市场调研同类工具与用户痛点)
2. [产品定位建议](#2-产品定位建议)
3. [优秀 Tauri 项目打包分析](#3-优秀-tauri-项目打包分析)
4. [组件化（拆分代码）是否必要](#4-组件化拆分代码是否必要)
5. [打包前要准备的清单](#5-打包前要准备的清单)
6. [GitHub 入门：注册与发布](#6-github-入门注册与发布)
7. [大白话术语表](#7-大白话术语表)

---

## 1. 市场调研：同类工具与用户痛点

### 1.1 国际主流

| 工具 | 平台 | 类型 | 收费 | 一句话定位 |
|---|---|---|---|---|
| **Raycast** | macOS / Win(Beta) | 启动器 + 命令面板 | 免费基础 / Pro $8/月 | 苹果生态"瑞士军刀"，靠 AI 和插件生态 |
| **Alfred** | macOS | 启动器 + 工作流 | £34 买断 | 老牌效率神器，Powerpack 之后才完整 |
| **Quicker** | Windows | 动作面板 | 免费 / Pro ¥48/年 | 鼠标右键轮盘 + 动作编排 |
| **uTools** | 全平台 | 启动器 + 插件市场 | 免费 / Pro ¥99 永久 | 国内"小而美"的 uTools 生态 |

> 【大白话】
> - 启动器 = 你按一个快捷键，弹出一个搜索框，输入啥就给你找啥 / 干啥的软件。
> - 工作流 = 把多个操作串起来自动跑（比如"截图→OCR→翻译→粘贴"一键完成）。
> - 插件市场 = 别人写好小功能让你一键装，类似手机应用商店。

### 1.2 国内 / 桌面小工具类

| 工具 | 定位 | 与抽屉柜的对比 |
|---|---|---|
| **小黄条** | 透明常驻桌面待办清单 | 都是"桌面常驻"思路；它专注待办，抽屉柜更通用 |
| **Yue Launcher（椰启动）** | Win 启动器 + 拼音搜 | 类似 Raymond 的 Windows 版 |
| **燕子启动器** | 右键轮盘 + AI 生成插件 | 强调"AI 帮你写工具"，定位更激进 |
| **Everything / Listary** | 文件搜索 | 是抽屉柜"软件区"的间接竞品 |
| **Windhawk** | Windows 美化 / 行为定制 | 不冲突，反而用户群重合 |
| **EcoPaste** | 剪贴板增强（Tauri 2 写的） | 同样是 Tauri + 本地优先，可对标学习 |

### 1.3 痛点大盘点（来自 v2ex / 知乎 / 评测网站）

> 数据来源：[V2EX 桌面整理讨论](https://v2ex.com/t/1139028) | [Raycast 评测](https://hypertools.so/article/raycast-vs-alfred-why-raycast-wins) | [CSDN Windows 神器清单](https://blog.csdn.net/M_H_D_/article/details/155931456) | [V2EX 燕子启动器](https://global.v2ex.co/t/1224596)

**A. 通用痛点（5 大类）**

1. **桌面乱** - 文件/图标太多，懒得整理，又离不开
2. **找东西慢** - 系统搜索慢、不准；Everything 装一个又嫌多一个
3. **重复操作多** - 同一段话/密码/路径每天敲几遍
4. **工具太多** - 装了 5-6 个小工具，互相打架，开机自启卡
5. **隐私焦虑** - 1Password / LastPass 服务器被攻击过；密码放云端心里没底

**B. 各竞品的具体槽点**

| 工具 | 用户最常抱怨的 |
|---|---|
| Raycast | Pro 订阅太贵（$8/月）；Windows 版仍 Beta |
| Alfred | 界面老旧（2010 风格）；AI 全要自己写 workflow |
| Quicker | 免费版功能砍得狠；闭源，配置文件隐私没保障；按月订阅 |
| uTools | 插件质量参差，加载要联网；想关 Pro 弹窗烦 |
| 小黄条 | 只做待办，扩展性弱 |
| 燕子启动器 | 项目较新（1.4k Stars），原生插件生态还在建设 |

**C. 用户最深处的需求（隐性）**

- **"我想要一个，但不想再多装一个"** - 都希望一个工具解决 N 个问题
- **"想自定义又不想写代码"** - 配置要够灵活但门槛要够低
- **"数据必须在我自己电脑上"** - 隐私和可控性是底线
- **"别弹广告别收钱"** - 国产工具的"信任税"

### 1.4 抽屉柜在地图上的位置

```
       重 (功能复杂) ▲
                      │   Raycast Pro
                      │   Alfred Powerpack
                      │
        ◀ 桌面中枢  ┼─────────────►  ← 你在这里
        (uTools)    │   抽屉柜 Wind-zp ✓
                      │   Yue Launcher
                      │   EcoPaste
       轻 (单一)     │   小黄条
                      │   Everything
                      └───────────────────► 广 (平台覆盖)
                          Win only     全平台
```

---

## 2. 产品定位建议

> 锚定原则：基于**你已经实现的四个模块**（密码/软件/片段/临时）+ **桌面常驻 widget 形态**。

### 2.1 推荐定位（一句话）

> **"为 Windows 重度用户打造的本地优先桌面抽屉柜：4 个高频数据模块 + 1 个永远在屏幕上的小窗口，能装就装下、能搜就搜到。"**

### 2.2 不做什么（明确边界）

| 不做 | 原因 |
|---|---|
| ✗ AI 集成（短期） | 你的偏好是"开箱即用"+"本地优先"，AI 反而引入云依赖 |
| ✗ 插件市场 | 维护成本高，对小工具是负担；做 AI 扩展是更轻的路 |
| ✗ 跨平台 | 桌面形态依赖 Win32 深度集成（如贴边隐藏，当前版本未包含），移植到 Mac 等于重写 |
| ✗ 云同步 | 隐私敏感用户是核心人群，加密云同步是个 2.0 大工程 |
| ✗ 取代系统启动器 | Alfred/Raycast 已经很强，没必要正面竞争 |

### 2.3 核心差异化（对用户的"我为什么装你"）

1. **桌面常驻 + 玻璃质感** - 区别于"打开才能用"的传统密码管理器
2. **本地优先 + 密码字段本地加密** - 数据永远不离开电脑，区别于 1Password（注：当前加密范围为密码字段，非全库加密）
3. **四合一** - 密码/软件/片段/临时一个抽屉搞定，区别于 5 个分散工具
4. **极简自动行为** - 15s 剪贴板自动清空、5min 自动锁；用户不用配置（注：贴边自动隐藏为早期原型演示，当前版本未包含）
5. **完全免费 + 开源** - 区别于 Quicker / 1Password 等收费工具

### 2.4 下一阶段可投入的功能（按价值排序）

| 优先级 | 功能 | 价值 | 工作量 |
|---|---|---|---|
| 🥇 | 全局快捷命令面板（Ctrl+Space 弹搜索） | 让 4 个模块真正"联动"起来 | 中 |
| 🥈 | 剪贴板自动监听归类 | 减少手动操作，符合"自动化"偏好 | 中 |
| 🥉 | 自动锁定倒计时（5min 闲置锁屏） | 安全性补完 | 小 |
| 4 | 已有模块深挖（密码强度可视化、片段标签） | 稳扎稳打 | 小-中 |
| 5 | 简单云同步（WebDAV/S3） | 多设备用户 | 大 |

---

## 3. 优秀 Tauri 项目打包分析

> 我下载并对比了 3 个项目：
> - [SecondDesk](https://github.com/Ryanisgood/SecondDesk)（v1.3.3，最像抽屉柜）
> - [EcoPaste](https://github.com/EcoPasteHub/EcoPaste)（剪贴板，Tauri 2 标杆）
> - [sys-ghost](https://github.com/hsinhan-h/sys-ghost)（系统监控，简化骨架）

### 3.1 三个项目的 `tauri.conf.json` 关键差异

| 配置项 | SecondDesk | EcoPaste | sys-ghost | 抽屉柜（当前） | 建议 |
|---|---|---|---|---|---|
| `productName` | "Second Desk" | "EcoPaste" | "SYS-GHOST" | "抽屉柜" | 保持中文产品名 OK |
| `identifier` | `com.seconddesk.app` | `com.ecopaste.app` | - | `com.drawer-box.app` | ✅ 规范 |
| `bundle.targets` | `["nsis", "msi"]` | `["nsis"]` | `["nsis"]` | `["nsis"]` | ✅ 够用；想上架商店再加 msi |
| `bundle.createUpdaterArtifacts` | `true` | - | - | 默认 false | ⭐ 强烈建议开启（自动更新基础） |
| `updater` 插件 | 已配 + 公钥 + 端点 | - | - | 没配 | ⭐ 强烈建议加上 |
| `webviewInstallMode.type` | `downloadBootstrapper` | - | - | 默认 | 推荐 `downloadBootstrapper`（小安装包+按需下载 WebView2） |
| `nsis.languages` | `["SimpChinese","English"]` | - | - | 同 | ✅ 一致 |
| `nsis.displayLanguageSelector` | `false` | - | - | `true` | 改 `false`（自动按系统语言） |
| `nsis.headerImage/sidebarImage` | 自定义 bmp | - | - | 没配 | ⭐ 加这两张图让安装器好看 |
| `shortDescription/longDescription` | 完整双语 | - | - | 没配 | ⭐ 必加，商店展示用 |
| `publisher` | "SecondDesk" | - | - | 没配 | 必加 |
| `csp` | 严格白名单 | - | - | `null`（不设） | ⭐ 建议加基础 CSP |
| `assetProtocol.enable` | `true`（读图） | - | - | 默认 | 需要读本地图片就开 |

> 【大白话】
> - **NSIS** = 一种 Windows 安装包格式（像 .exe 一样双击安装）。
> - **MSI** = 微软标准安装包，公司批量部署用，个人用户不太关心。
> - **CSP**（Content Security Policy）= 一种安全策略，告诉浏览器"只准从这些地方加载图片/脚本"。空着 = 默认全开 = 不太安全。
> - **updater** = Tauri 自带的"检查新版本→下载→自动装"功能，跟手机 App 自动更新一个意思。
> - **headerImage/sidebarImage** = 安装器左侧那张大图，绿色对勾旁边那块的背景。

### 3.2 项目结构对比

| 目录 | SecondDesk | EcoPaste | sys-ghost | 抽屉柜（当前） | 说明 |
|---|---|---|---|---|---|
| `.github/workflows/` | ✅ | ✅ | - | ❌ | ⭐ 强烈建议加（自动打包 release） |
| `.claude/` 或 `.agents/skills/` | ✅ | ✅ | - | ❌ | 工具/技能配置，可选 |
| `scripts/` | ✅ | ✅ | - | ✅ build.ps1 / check-env.ps1 | 已有，✅ |
| `src-tauri/src/` | 多模块拆分 | 多模块 | 单文件 | 5 文件已分 | ✅ |
| `src/composables/` | ✅ | - | - | ✅ useWindowDrag | ✅ |
| `src/stores/` | ✅ | - | - | ✅ 5 个 store | ✅ |
| `latest.json` | ✅（updater 元数据） | - | - | ❌ | ⭐ 加上 |
| `check.ps1 / release.ps1` | ✅ | - | - | ✅ build.ps1 已有 | ✅ |

### 3.3 SecondDesk 的"分发链"完整流程（值得抄作业）

```
开发者改代码
    ↓
git push
    ↓
.github/workflows/release.yml 自动触发
    ↓
1. pnpm install
2. pnpm tauri build  (编译 + 打包 → NSIS .exe)
3. 生成 latest.json （updater 需要的元数据）
4. 调 softprops/action-gh-release 上传 .exe 和 latest.json 到 GitHub Releases
    ↓
用户打开抽屉柜 → updater 插件读 latest.json
    ↓
检测到新版本 → 弹"是否更新" → 下载 → 重启安装
```

> 你照搬这个流程即可。

---

## 4. 组件化（拆分代码）是否必要

### 4.1 你现在的代码状态

| 文件 | 行数（估） | 状态 |
|---|---|---|
| `src-tauri/src/lib.rs` | ~1000+ 行 | 偏大，命令都堆这里 |
| `src/components/MainLayout.vue` | ~500+ 行 | 模板 + 样式 + 逻辑一体 |
| `src/views/*.vue` | 各 100-300 | OK |
| `src/stores/*.ts` | 各 50-150 | OK |

### 4.2 结论：暂不需要拆分

**理由**：
- 总代码量 < 3000 行，单文件 1000 行是"可读但不舒服"区间
- 团队只有你一个人，拆太多反而跳转成本高
- 优秀项目（SecondDesk / EcoPaste）核心 lib.rs 也都在 1000-2000 行

**触发拆分的信号**（什么时候才该拆）：
- 单文件 > 3000 行
- 出现 3 个人同时改一个文件
- 编译时间 > 30 秒（拆 Rust 模块可减少）

### 4.3 但建议做的小重构

1. `lib.rs` 按模块拆出 `commands/` 子目录（参考 SecondDesk）
   - `commands/passwords.rs`
   - `commands/apps.rs`
   - `commands/widget.rs`
   - `commands/window.rs`
   - `lib.rs` 只留 setup + invoke_handler 注册

2. 前端 Pinia store 按职责合并
   - 已有 5 个，OK 不用动

---

## 5. 打包前要准备的清单

> 按顺序执行，每项 5-15 分钟。

### 5.1 图标（必须）

| 图标 | 大小 | 来源 |
|---|---|---|
| `icon.png` | 1024×1024 | 你画一张 / 找 AI 生成 |
| `32x32.png` | 32×32 | `tauri icon` 自动生成 |
| `128x128.png` | 128×128 | 同上 |
| `128x128@2x.png` | 256×256 | 同上（视网膜屏） |
| `icon.ico` | 多尺寸合一 | 同上（Windows 用） |
| `icon.icns` | 多尺寸合一 | 同上（macOS 用，v1 不用也行） |
| `installer-header.bmp` | 150×57 | 画一张或用占位图 |
| `installer-sidebar.bmp` | 164×314 | 同上 |

```bash
# 一行命令生成所有图标
pnpm tauri icon path/to/source-1024.png
```

### 5.2 配置补充

最小必改项（在 `src-tauri/tauri.conf.json`）：

```json
{
  "publisher": "你的名字/组织",
  "shortDescription": "桌面抽屉柜：密码/软件/片段/临时一站搞定",
  "longDescription": "...",
  "bundle": {
    "createUpdaterArtifacts": true,
    "windows": {
      "nsis": {
        "headerImage": "icons/installer-header.bmp",
        "sidebarImage": "icons/installer-sidebar.bmp",
        "displayLanguageSelector": false
      },
      "webviewInstallMode": {
        "type": "downloadBootstrapper"
      }
    }
  },
  "plugins": {
    "updater": {
      "active": true,
      "pubkey": "<后面生成>",
      "endpoints": ["https://github.com/<user>/<repo>/releases/latest/download/latest.json"]
    }
  }
}
```

### 5.3 自动打包脚本（GitHub Actions）

新建 `.github/workflows/release.yml`：

```yaml
name: Release
on:
  push:
    tags: ['v*']
jobs:
  release:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20 }
      - uses: dtolnay/rust-toolchain@stable
      - run: npm install
      - run: npm run tauri build
      - uses: softprops/action-gh-release@v2
        with:
          files: src-tauri/target/release/bundle/nsis/*.exe
          generate_release_notes: true
```

### 5.4 打包命令

```bash
# 调试版（带控制台输出）
pnpm tauri dev

# 发布版（生成 .exe 安装包）
pnpm tauri build
# 产物位置：
#   src-tauri/target/release/bundle/nsis/抽屉柜_0.1.0_x64-setup.exe
#   src-tauri/target/release/drawer-box.exe  (裸 exe，绿色版)
```

> 【大白话】
> - **dev = 开发版** = 跑起来给你看效果，改代码自动刷新。慢（带调试信息）。
> - **build = 发布版** = 给最终用户装的，打了很多优化，启动快 10 倍。

### 5.5 首次打包会遇到的小坑（提前心理建设）

| 现象 | 原因 | 解决 |
|---|---|---|
| 编译 10+ 分钟 | 首次拉 crates 依赖 | 正常，等等就好 |
| 提示 "MSVC not found" | 没装 C++ 编译工具 | 装 Visual Studio Build Tools |
| 生成的 .exe 启动黑屏 | WebView2 没装 | 改 `webviewInstallMode: downloadBootstrapper` 让安装器自带 |
| .exe 报毒 | 没代码签名 | 个人项目 → 加白名单；正式 → 买 EV 证书 |
| 体积 50MB+ | 静态资源没压缩 | `vite build` 加 `vite-plugin-compress` |

---

## 6. GitHub 入门：注册与发布

> 你说"我不会用 GitHub"，所以从最基础的写起。

### 6.1 注册（5 分钟）

1. 打开 https://github.com
2. 点右上角 **Sign up**
3. 填邮箱、密码、用户名
4. 验证邮箱
5. 完事

> 【大白话】GitHub = "代码的网盘"，但比网盘多了"版本历史"和"让别人看/改你代码"的功能。

### 6.2 你需要理解的 4 个概念

| 概念 | 大白话 | 类比 |
|---|---|---|
| **Repository（仓库）** | 一个项目文件夹 | 一个网盘文件夹 |
| **Commit（提交）** | 一次"保存"，带说明 | Word 的"修订历史" |
| **Branch（分支）** | 一份独立的草稿 | 文档的"副本"，改好再合并回去 |
| **Release（发行版）** | 一个带版本号的打包 | 软件的"v1.0" "v2.0" 正式版 |

### 6.3 私有 vs 公开（可随时切换）

> **可以随时切换**，代码不会丢。

| 类型 | 谁能看 | 费用 |
|---|---|---|
| **Private（私有）** | 只有你（和邀请的人） | 免费 |
| **Public（公开）** | 任何人 | 免费 |

切换方法：仓库页面 → Settings → General → Danger Zone → "Change repository visibility"。

### 6.4 我可以代为操作的部分

| 任务 | 你能做 | 我能做 |
|---|---|---|
| 注册账号 | ✅ 必须你来 | ❌ 需要你邮箱收验证 |
| 创建仓库 | ✅ / 委托我 | ✅ 给 token 我可以代 |
| push 代码 | 给我 token | ✅ 完全可以 |
| 创建 Release | ✅ / 委托我 | ✅ 给权限后可以 |
| 申请市场账号 | ✅ 必须你来 | ❌ |

**最简方案**：
1. 你注册账号 → 把用户名告诉我
2. 我在 Trae 里帮你生成 SSH key 或者 Personal Access Token
3. 你把 token 贴到 Trae → 我帮你 push 代码 + 创建 Release
4. 全程不用你懂 Git 命令

> 我会在你想发布的时候再具体教，不用现在记。

---

## 7. 大白话术语表

按字母/拼音排序，遇到不会的随时 ctrl+F 搜。

| 术语 | 大白话 |
|---|---|
| **AES-256** | 一种给数据"上锁"的方法，暴力破解要算 2^256 次，宇宙毁灭都算不完 |
| **Argon2** | 把你的主密码"搅一搅"变成钥匙的方法，故意慢，挡黑客 |
| **API** | 应用程序之间互相喊话的"对讲机频道" |
| **Cargo** | Rust 的"包管理工具"，相当于 Python 的 pip、Node 的 npm |
| **CDN** | 把你的文件复制到全球各地的服务器，让用户访问更快 |
| **CSP** | 浏览器安全清单，规定"只准从这几个地方加载东西" |
| **crate（Rust 包）** | Rust 里的"一个库" / "一个 npm 包" |
| **DPI** | 屏幕像素密度，影响窗口实际像素大小 |
| **Electron** | 另一种"网页套壳"桌面框架，比 Tauri 体积大 10 倍 |
| **Frontend / Backend** | 前端 = 你看到的界面；后端 = 背后干活的（这里指 Rust） |
| **Git** | 记录代码每次改动的"时光机" |
| **GitHub** | 把 Git 时光机放到云上 + 加上社交功能 |
| **HMAC** | 给消息加"防伪标签"的方法 |
| **i18n / l10n** | i18n = 国际化（多语言）；l10n = 本地化（翻译成某地语言） |
| **IPC** | 进程间通信，前端喊后端干活的"对讲机" |
| **JNI / N-API** | 两种让 JS 和 Rust/C++ 互相喊话的方法 |
| **NSIS** | Windows 上一种把程序打成 .exe 安装包的工具 |
| **OAuth** | "用 Google/微信账号登录第三方网站"的协议 |
| **Pinia** | Vue 的状态管理库 = 跨组件共享变量的"中央仓库" |
| **RAT** | 远程控制木马（病毒），跟"打包"无关，别误会 |
| **RSA / 公钥私钥** | 一对数学上关联的钥匙：公钥加密，私钥解密（反过来签） |
| **SDK** | 软件开发工具包 = 别人写好的工具箱 |
| **Subclass（子类化）** | 在 Windows 里"偷听"窗口消息的技术 |
| **Tauri** | 用 Rust 写后端 + 用网页写界面来开发桌面 App 的框架 |
| **Tray（托盘）** | Windows 任务栏右下角的小图标区 |
| **VPN** | 翻墙软件（你说工具时通常不是这个意思，但提一下别混淆） |
| **WebView** | 操作系统自带的浏览器引擎，App 用它来渲染网页 |
| **WebView2** | Windows 10/11 自带的 Edge 内核，Win 桌面应用首选 |
| **WSL** | Windows 上的 Linux 子系统，跑 Linux 命令的 |
| **XSS** | 跨站脚本攻击 = 黑客在你页面里塞恶意代码 |
| **YAML / JSON / TOML** | 三种"配置文件的写法"，长得不一样但干的活一样 |
| **零拷贝（zero-copy）** | 让数据"原地不动"，不复制一遍，提高性能 |
| **端到端加密** | 只有你和对方能解密，连服务器运营方都看不到内容 |
| **工作流（workflow）** | 把多个操作串起来自动跑 |
| **钩子（hook）** | 系统/事件留的"插头"，你可以塞代码进去"偷听" |
| **进程 / 线程** | 进程 = 一个独立运行的程序；线程 = 进程里的"工人" |
| **壳（shell）** | 命令行窗口（PowerShell、cmd、bash 都叫壳） |
| **跨平台** | 同一份代码能跑在 Windows / Mac / Linux |
| **类（class）/ 结构体（struct）** | 两种把数据和方法打包的方式（C++ 用 class，Rust 用 struct） |
| **冒烟测试** | "随便点几下看会不会崩"的初级测试 |
| **内存泄漏** | 程序用了内存忘了还，越用越卡 |
| **热更新（HMR）** | 改代码界面自动刷新，不用重启程序 |
| **沙箱（sandbox）** | 隔离的"小房间"，程序在里面乱搞也影响不到外面 |
| **生态（ecosystem）** | 一个技术 + 围绕它的库/工具/社区 |
| **守护进程** | 一直在后台跑的程序，不出现在任务栏 |
| **回调（callback）** | "这事干完了叫我一声"，常用在异步操作 |
| **栈（stack）/ 堆（heap）** | 两种内存使用方式，栈快但小，堆慢但大 |
| **中间件（middleware）** | 在请求/响应中间"加塞"做点事的程序 |

---

## 附录：项目代号对照

| 我们的项目 | 对应官方名称 | 对应打包产物 |
|---|---|---|
| Wind-zp / 抽屉柜 | productName: "抽屉柜" | 抽屉柜_0.1.0_x64-setup.exe |

---

**最后更新**：2026-07-28
**作者**：Trae IDE (M3) 整理
**适用项目**：d:\AI工具\桌面助手2.0\Wind-zp
