//! v0.4.3 Batch 1-B + 2.1：resolve_lnk_paths_raw 解析/映射纯函数回归。
//! 旧实现缺陷：PowerShell 输出 filter 掉空行后按原始索引配对——
//! 任一快捷方式解析为空，其后所有条目错位一位。
//! 协议（Batch 2.1 扩展）：每行 `<索引>\t<目标>\t<参数>\t<工作目录>`，
//! 按标记配对；字段数可不足（缺省空串）。本文件钉死该协议。

use crate::commands::apps::{parse_lnk_output, LnkResolution};

const LNK_A: &str = "C:\\Users\\u\\AppData\\Roaming\\a.lnk";
const LNK_B: &str = "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\b.lnk";
const LNK_C: &str = "C:\\Users\\u\\Desktop\\c.lnk";
const LNK_D: &str = "C:\\Users\\u\\Desktop\\d.lnk";

fn paths() -> Vec<&'static str> {
    vec![LNK_A, LNK_B, LNK_C, LNK_D]
}

fn t(target: &str, args: &str, workdir: &str) -> LnkResolution {
    LnkResolution {
        target: target.to_string(),
        args: args.to_string(),
        workdir: workdir.to_string(),
    }
}

#[test]
fn t_lk01_全部解析成功_逐项正确映射() {
    let out = parse_lnk_output(
        &paths(),
        "0\tC:\\Apps\\alpha.exe\t--min\tD:\\wd\r\n1\tC:\\Apps\\beta.exe\t\t\r\n2\tC:\\Apps\\gamma.exe\t\t\r\n3\tC:\\Apps\\delta.exe\t\t\r\n",
    );
    assert_eq!(
        out.get(LNK_A),
        Some(&t("C:\\Apps\\alpha.exe", "--min", "D:\\wd"))
    );
    assert_eq!(out.get(LNK_B), Some(&t("C:\\Apps\\beta.exe", "", "")));
    assert_eq!(out.get(LNK_C), Some(&t("C:\\Apps\\gamma.exe", "", "")));
    assert_eq!(out.get(LNK_D), Some(&t("C:\\Apps\\delta.exe", "", "")));
}

#[test]
fn t_lk02_首项解析失败_后续全部不错位() {
    // 旧实现的致命场景：第 0 项为空行 → 旧代码 filter 后 1→0、2→1、3→2 全部错绑
    let out = parse_lnk_output(&paths(), "0\t\t\t\r\n1\tC:\\Apps\\beta.exe\t\t\r\n2\tC:\\Apps\\gamma.exe\t\t\r\n3\tC:\\Apps\\delta.exe\t\t\r\n");
    assert!(!out.contains_key(LNK_A), "失败项不得产生映射");
    assert_eq!(
        out.get(LNK_B),
        Some(&t("C:\\Apps\\beta.exe", "", "")),
        "错位回归：b.lnk 被绑到 beta"
    );
    assert_eq!(out.get(LNK_C), Some(&t("C:\\Apps\\gamma.exe", "", "")));
    assert_eq!(out.get(LNK_D), Some(&t("C:\\Apps\\delta.exe", "", "")));
}

#[test]
fn t_lk03_中间单项失败_两侧均不受影响() {
    let out = parse_lnk_output(&paths(), "0\tC:\\Apps\\alpha.exe\t\t\r\n1\t\t\t\r\n2\tC:\\Apps\\gamma.exe\t\t\r\n3\tC:\\Apps\\delta.exe\t\t\r\n");
    assert_eq!(out.get(LNK_A), Some(&t("C:\\Apps\\alpha.exe", "", "")));
    assert!(!out.contains_key(LNK_B));
    assert_eq!(out.get(LNK_C), Some(&t("C:\\Apps\\gamma.exe", "", "")));
    assert_eq!(out.get(LNK_D), Some(&t("C:\\Apps\\delta.exe", "", "")));
}

#[test]
fn t_lk04_连续多项失败_仅失败项缺映射() {
    let out = parse_lnk_output(
        &paths(),
        "0\t\t\t\r\n1\t\t\t\r\n2\t\t\t\r\n3\tC:\\Apps\\delta.exe\t\t\r\n",
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out.get(LNK_D), Some(&t("C:\\Apps\\delta.exe", "", "")));
    assert!(!out.contains_key(LNK_A) && !out.contains_key(LNK_B) && !out.contains_key(LNK_C));
}

#[test]
fn t_lk05_空格与中文路径逐字保留() {
    let p = vec![LNK_A];
    let target = "C:\\Program Files\\腾讯游戏\\英雄联盟 WeGame\\client.exe";
    let out = parse_lnk_output(&p, &format!("0\t{target}\t--flag 值\tD:\\工作 目录\r\n"));
    assert_eq!(
        out.get(LNK_A),
        Some(&t(target, "--flag 值", "D:\\工作 目录"))
    );
}

#[test]
fn t_lk06_两字段旧格式行_参数与目录缺省为空() {
    // 兼容旧格式/截断输出：字段数不足不报错，缺省空串
    let out = parse_lnk_output(&vec![LNK_A], "0\tC:\\Apps\\alpha.exe\r\n");
    assert_eq!(out.get(LNK_A), Some(&t("C:\\Apps\\alpha.exe", "", "")));
}

#[test]
fn t_lk07_无标记杂讯行_忽略不参与配对() {
    let out = parse_lnk_output(
        &paths(),
        "WARNING: some noise\r\n0\tC:\\Apps\\alpha.exe\t\t\r\nconverted file foo\r\n2\tC:\\Apps\\gamma.exe\t\t\r\n",
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out.get(LNK_A), Some(&t("C:\\Apps\\alpha.exe", "", "")));
    assert_eq!(out.get(LNK_C), Some(&t("C:\\Apps\\gamma.exe", "", "")));
    assert!(!out.contains_key(LNK_B) && !out.contains_key(LNK_D));
}

#[test]
fn t_lk08_索引越界与非法索引_忽略() {
    let out = parse_lnk_output(&vec![LNK_A], "9\tC:\\x.exe\t\t\r\nzero\tC:\\y.exe\t\t\r\n");
    assert!(out.is_empty());
}

#[test]
fn t_lk09_空输出与全空目标_零映射不报错() {
    assert!(parse_lnk_output(&paths(), "").is_empty());
    assert!(parse_lnk_output(&paths(), "0\t\t\t\r\n1\t\t\t\r\n").is_empty());
    // 旧协议残留（无 tab 的纯路径行）不得被误配
    assert!(parse_lnk_output(&paths(), "C:\\Apps\\alpha.exe\r\n").is_empty());
}

#[test]
fn t_lk10_参数含路径分隔符与空格_原样保留不参与目标匹配() {
    // PWA 场景：chrome.exe --app=https://x.y——参数里的 / 不影响身份键的路径归一
    let out = parse_lnk_output(
        &vec![LNK_A],
        "0\tC:\\Chrome\\chrome.exe\t--app=https://example.com\r\n",
    );
    assert_eq!(
        out.get(LNK_A),
        Some(&t(
            "C:\\Chrome\\chrome.exe",
            "--app=https://example.com",
            ""
        ))
    );
}
