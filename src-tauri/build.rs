fn main() {
    tauri_build::build();

    // P0-#VERSION#BUILD#TIME：把编译时的时间戳写进 env（供 lib.rs env! 宏读取）
    // 之前：lib.rs 用 chrono::Local::now() → RUNTIME 时间（app 打开瞬间）
    //   → 用户在 17:52 打开 app 就看到 "1752"，以为用了新 build
    //   → 实际装的还是 15:53 编译的 exe，所有代码改动都没生效
    // 现在：build.rs 在编译时把时间戳写入 env → 每次运行显示同一个 build 时间
    //   → 用户能一眼看出来用的是不是新 build
    //
    // 实现：build.rs 不能用主 crate 的依赖（chrono 在 [dependencies] 里），
    //   所以用 std::time::SystemTime 拿时间（无外部依赖）
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 把 unix 时间戳转成 YYYYMMDD-HHMM 格式（手工算，简单）
    // 用 Howard Hinnant date 算法（civil_from_days）
    let secs_per_day = 86400;
    let secs_per_hour = 3600;
    let secs_per_min = 60;
    let days = (now / secs_per_day) as i64;
    let secs_today = (now % secs_per_day) as u32;
    let hour = secs_today / secs_per_hour;
    let minute = (secs_today % secs_per_hour) / secs_per_min;
    // 1970-01-01 是 day 0
    // 用 civil_from_days 算 Y M D
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    let build_time = format!(
        "{:04}{:02}{:02}-{:02}{:02}",
        year, m, d, hour, minute
    );
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", build_time);

    // 强制 build.rs 每次都重跑（不然 cargo 会跳过）
    println!("cargo:rerun-if-changed=build.rs");
}