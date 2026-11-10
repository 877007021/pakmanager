//! 小工具：日期格式化。

use glib::DateTime;

/// unix 秒 → `2026-08-26`（本地时区）。
pub fn format_date(ts: i64) -> String {
    match DateTime::from_unix_local(ts) {
        Ok(dt) => format!(
            "{:04}-{:02}-{:02}",
            dt.year(),
            dt.month(),
            dt.day_of_month()
        ),
        Err(_) => "—".to_string(),
    }
}

/// 把一长串文本截断到 `max` 个字符，超出补省略号。
pub fn ellipsize(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
