// ======================================================================
// 应用内版本更新历史（v0.4.1 任务 B）—— 单一版本记录源（随应用打包的快照）
//
// 事实源契约：
// 1. 本文件的唯一上游事实源 = GitHub Releases（MetalRiver/cabinet-drawer），
//    notes 为对应 Release 正文**逐字快照**（2026-10-11 经 GitHub API 核对）；
// 2. 发布流程约束：每个正式版本发布时（打 tag + Release published 后），
//    必须在同一次发布提交里把该版本条目追加进本文件——应用内历史、
//    GitHub Release、更新清单 latest.json 三处内容同源，禁止漂移；
// 3. 未正式发布的版本允许以 `unreleased: true + date: ""` 登记（UI 显示"未发布"），
//    但**禁止虚构发布日期**；
// 4. 离线可用：本文件随应用打包，不依赖网络；联网时的"检查更新"发布说明
//    来自 updater 的 latest.json（与 Release 正文同源）；
// 5. **发布前元数据冻结流程（v0.4.1 定）**——最终安装包内不得残留"未发布"：
//    ① 确定发布日 D → ② 冻结 commit：回填 date=D、删除 unreleased 标记、
//    notes 终稿（该 notes 即 GitHub Release 正文与 latest.json notes 的唯一来源）
//    → ③ 用冻结 commit 构建并签名安装包 → ④ 当日发布 Release（正文=notes）+
//    上传 latest.json（指向③的安装包与 .sig）。
//    任何冻结后源码改动 → 必须重新走②③④，禁止发布后只改源码不重建安装包。
// ======================================================================

export const RELEASES_PAGE_URL = "https://github.com/MetalRiver/cabinet-drawer/releases";

export interface VersionHistoryEntry {
  /** 正式版本号（tag 名，含 v 前缀） */
  version: string;
  /** Release 标题 */
  name: string;
  /** 发布日期（UTC，YYYY-MM-DD）；空串 = 未发布（发布时由单一源回填，禁止虚构日期） */
  date: string;
  /** true = 尚未正式发布（UI 显示"未发布"，不计入已发行版本口径） */
  unreleased?: boolean;
  /** 一句话摘要（折叠态展示） */
  summary: string;
  /** Release 正文逐字快照（展开态展示） */
  notes: string;
  /** 该版本 Release 页直达链接 */
  url: string;
}

export const VERSION_HISTORY: VersionHistoryEntry[] = [
  {
    version: "v0.4.1",
    name: "抽屉柜 Drawer v0.4.1",
    date: "2026-10-12",
    unreleased: false,
    summary: "设置中心 V2 重构 + 持久化修复 + 便签编辑/续期/永久 + 失效治理 + 版本历史 + 外链修复（发布时回填日期）",
    url: "https://github.com/MetalRiver/cabinet-drawer/releases/tag/v0.4.1",
    notes: `## 修复

- **设置持久化**：永不锁定 / 永不过期 / 永久保留等 0 值语义修复，重启不再回退
- **回收站永久保留**：保留天数设为 0 时绝不自动清理（修复可能误删回收站内容的严重缺陷）
- **窗口位置**：125%/150% 缩放下恢复不再偏移；位置预设不再改变窗口尺寸

## 新增

- **设置中心 V2**：五分类重构（基础 / 安全与密码 / 窗口与外观 / 数据与备份 / 高级与更新）
- **便签**：编辑、追加、续期（1/3/7/30 天与自定义）、永不过期
- **软件失效治理**：失效集中列表、手动重新检查（自动重定位优先）、固定盘缺失批量清退（仅软删除）、移动盘/网络/不确定状态保护
- **版本更新历史**：应用内查看（离线可用）与 GitHub 直达
- **深色下拉组件**：全设置页替换原生下拉

## 安全

- 密码加密、主密码验证、恢复短语、备份恢复机制与 v0.4.0 完全一致，未做任何变更`,
  },
  {
    version: "v0.4.0",
    name: "抽屉柜 Drawer v0.4.0",
    date: "2026-10-05",
    summary: "自定义数据存储位置 + 迁移/重装重连 + 加密备份 + 应用路径失效治理 + 应用内签名更新",
    url: "https://github.com/MetalRiver/cabinet-drawer/releases/tag/v0.4.0",
    notes: `## 新增

- **自定义数据存储位置**：可以把数据保存到 D 盘等其他本地磁盘
- **数据位置迁移**：随时把数据安全迁移到其他本地磁盘（保留安全副本，旧数据不会被误删）
- **使用已有数据目录**：重装系统 / 换机后，直接指定原来的数据目录继续使用
- **加密备份与恢复**：一键导出单个 .drawerbox 加密备份；首次安装即可从备份恢复
- **恢复出厂设置**：一键清空当前数据位置，回到首次安装状态
- **应用路径失效治理**：软件被移动/卸载后明确标示，可重新关联、打开原位置或从抽屉柜移除（绝不替你卸载软件）
- **应用内检查更新**：官方签名校验的自动更新

## 改进

- 修复重复启动会打开多个实例；关闭窗口 = 最小化到托盘
- 应用内检查更新支持下载进度与失败重试

## 安全说明

- **升级不会自动移动您现有的数据目录**，密码库安全格式不变，原主密码与恢复短语继续有效
- 更新包使用签名校验（minisign），下载后自动验证`,
  },
  {
    version: "v0.3.2",
    name: "抽屉柜 Drawer v0.3.2 · 首个公开发布版本 (First Public Release)",
    date: "2026-09-27",
    summary: "首个公开发布版本：本地优先收纳 + 密码字段加密 + 恢复词 + .drawerbox 加密备份",
    url: "https://github.com/MetalRiver/cabinet-drawer/releases/tag/v0.3.2",
    notes: `**抽屉柜 Drawer v0.3.2（首个公开发布版本 · First Public Release）**

一个本地优先的 Windows 桌面「抽屉」：把应用、文件、网址、片段与密码收纳进一个小窗，随手可取。

## 特点

- 本地优先：数据只存你的电脑（本地 SQLite），无需账号、不依赖云端服务、无遥测
- 收纳：应用 / 文件 / 文件夹 / 网址自动识别，点击即开；高频条目常用页直达
- 密码：仅密码字段在本地数据库中加密存储（AES-256-GCM + Argon2id）；查看/复制需主密码验证；剪贴板自动清空（默认 15 秒）
- 片段：常用命令、代码片段一键复制｜临时便签 3 天自动清理｜回收站｜闲置自动锁定（默认 5 分钟）
- 恢复与备份：忘记主密码可用首次生成的 12 个恢复词重置；支持 \`.drawerbox\` 认证加密备份/恢复

## 安装

1. 下载下方 \`Drawer_0.3.2_x64-setup.exe\`
2. 双击安装（当前用户目录，无需管理员权限）
3. 首次启动设置主密码，并**抄下 12 个恢复词**（不要截图、不要存网盘、不要发给任何人）

## 重要限制

- 仅 Windows 10/11 x64；无同步 / 云备份 / 跨平台 / AI 功能
- 无应用内自动更新，新版本需手动下载
- 加密范围仅为密码字段，其他元数据（标题/网址/备注等）为明文存储
- 「贴边自动隐藏」当前版本不包含

## 签名与杀软提示

安装包未做代码签名，Windows Defender / 杀毒软件可能提示「未知发布者」，点「仍要运行」即可。

## 校验

- 文件：\`Drawer_0.3.2_x64-setup.exe\`（GitHub Releases 对非 ASCII 资产名自动去前缀，文件内容与校验值不受影响）
- 大小：4,999,460 bytes
- SHA256：\`7574301fa3e7cb66724ae7c465b05580ff8f616be37873c8c73aa173431b145d\`

## License

GPL-3.0-only（源代码）· 品牌资产另有边界 · 反馈请走 Issues，安全问题见 [SECURITY.md](https://github.com/MetalRiver/cabinet-drawer/blob/master/SECURITY.md)。

v0.3.0 / v0.3.1 为 Closed Beta 历史版本。

---

*(EN)* Drawer is a local-first Windows desktop drawer for apps, files, URLs, snippets and passwords. No account. No cloud dependency. No telemetry. Password fields are encrypted locally (AES-256-GCM with an Argon2id-derived key); other metadata is stored unencrypted. Requires Windows 10/11 x64. Unsigned installer — your antivirus may warn about an unknown publisher.`,
  },
];

/**
 * 找到本机版本对应的历史条目（语义化比较：v0.4.1 匹配 v0.4.0 时不误标，
 * 只在完全相同时标记"本机安装"）；找不到（如本机为未登记的候选版）返回 null。
 */
export function findInstalledEntry(currentVersion: string): VersionHistoryEntry | null {
  const norm = (v: string) => v.trim().replace(/^v/i, "");
  return VERSION_HISTORY.find((e) => norm(e.version) === norm(currentVersion)) ?? null;
}
