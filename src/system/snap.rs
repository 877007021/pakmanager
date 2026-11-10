//! Snap 读取器。
//!
//! 通过 snapd 的本地 REST API（`/run/snapd.socket`，`GET /v2/snaps`）读取真实数据；
//! 失败时退回解析 `snap list` 的文本输出。

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use crate::model::{Package, Source};

const SNAP_SOCKET: &str = "/run/snapd.socket";

/// 扫描所有已安装的 snap。
pub fn scan() -> Result<Vec<Package>, String> {
    match query_snapd() {
        Ok(packages) if !packages.is_empty() => Ok(packages),
        Ok(_) => Err("snapd 没有返回任何已安装的 snap".to_string()),
        Err(err) => {
            // 退回 CLI；两者都失败时把 API 的错误报出去（信息更多）。
            fallback_cli().map_err(|cli_err| format!("{err}；备用方式也失败：{cli_err}"))
        }
    }
}

/// 用 Unix socket 手写一次最小 HTTP/1.0 请求，避免引入 HTTP 客户端依赖。
fn query_snapd() -> Result<Vec<Package>, String> {
    let mut stream =
        UnixStream::connect(SNAP_SOCKET).map_err(|err| format!("连接 snapd 失败：{err}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();

    let request = "GET /v2/snaps HTTP/1.0\r\nHost: localhost\r\nAccept: application/json\r\n\r\n";
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("写入 snapd 请求失败：{err}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|err| format!("读取 snapd 响应失败：{err}"))?;

    let body_start = find_body(&raw).ok_or_else(|| "snapd 响应格式异常".to_string())?;
    let body = String::from_utf8_lossy(&raw[body_start..]).to_string();
    parse_snapd_json(&body)
}

fn find_body(raw: &[u8]) -> Option<usize> {
    raw.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn parse_snapd_json(body: &str) -> Result<Vec<Package>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|err| format!("解析 snapd JSON 失败：{err}"))?;
    if value
        .get("status-code")
        .and_then(|v| v.as_i64())
        .unwrap_or(200)
        != 200
    {
        let message = value
            .get("result")
            .and_then(|r| r.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("snapd 返回错误：{message}"));
    }
    let Some(items) = value.get("result").and_then(|r| r.as_array()) else {
        return Err("snapd 响应缺少 result".to_string());
    };

    let mut packages = Vec::new();
    for item in items {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let snap_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("app");
        let summary = item
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let publisher = item
            .get("publisher")
            .and_then(|p| p.get("display-name"))
            .and_then(|v| v.as_str())
            .or_else(|| item.get("developer").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        let confinement = item
            .get("confinement")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = item
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let installed_size = item
            .get("installed-size")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let installed_at = item
            .get("install-date")
            .and_then(|v| v.as_str())
            .and_then(parse_rfc3339);

        // base / kernel / gadget / snapd 这类 snap 是其它 snap 的运行时，删掉会连带出事。
        let protected = matches!(snap_type, "base" | "kernel" | "gadget" | "snapd")
            || name == "snapd"
            || name == "core"
            || name == "bare"
            || name.starts_with("core");

        let channel = item
            .get("tracking-channel")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        packages.push(Package {
            id: format!("snap:{name}"),
            name: name.to_string(),
            display_name: name.to_string(),
            version,
            source: Source::Snap,
            size_bytes: installed_size.max(0) as u64,
            installed_at,
            summary: summary.clone(),
            description,
            section: snap_type.to_string(),
            auto_installed: false,
            essential: false,
            protected,
            leftover: false,
            maintainer: publisher.clone(),
            icon: snap_icon(name),
            desktop_file: snap_desktop_file(name),
            publisher,
            confinement,
            keywords: format!("{name} {summary} {channel} {snap_type}").to_lowercase(),
        });
    }
    Ok(packages)
}

/// snap 自带的图标（`meta/gui/*` 里的 png/svg）。
fn snap_icon(name: &str) -> Option<String> {
    let base = format!("/snap/{name}/current/meta/gui");
    for candidate in ["icon.svg", "icon.png", "logo.svg", "logo.png"] {
        let path = format!("{base}/{candidate}");
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

/// snap 提供的 .desktop 文件（由 snapd 生成在 /var/lib/snapd/desktop/applications）。
fn snap_desktop_file(name: &str) -> Option<String> {
    let dir = "/var/lib/snapd/desktop/applications";
    let entries = std::fs::read_dir(dir).ok()?;
    let prefix = format!("{name}_");
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with(&prefix) && file_name.ends_with(".desktop") {
            return Some(entry.path().to_string_lossy().to_string());
        }
    }
    None
}

/// `2026-08-26T13:34:41+08:00` → unix 秒（取本地时区解释）。
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year: i64 = text.get(0..4)?.parse().ok()?;
    let month: i64 = text.get(5..7)?.parse().ok()?;
    let day: i64 = text.get(8..10)?.parse().ok()?;
    let hour: i64 = text.get(11..13)?.parse().ok()?;
    let minute: i64 = text.get(14..16)?.parse().ok()?;
    let second: i64 = text.get(17..19)?.parse().ok()?;
    let offset = parse_offset(&text[19..]).unwrap_or(0);
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn parse_offset(rest: &str) -> Option<i64> {
    let rest = rest.trim_start_matches('.');
    if rest.is_empty() || rest.starts_with('Z') {
        return Some(0);
    }
    let sign = if rest.starts_with('-') { -1 } else { 1 };
    let digits: String = rest
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(4)
        .collect();
    if digits.len() < 4 {
        return None;
    }
    let hours: i64 = digits[0..2].parse().ok()?;
    let minutes: i64 = digits[2..4].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Howard Hinnant 的 `days_from_civil`：公历日期 → 1970-01-01 起的天数。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 没有 snapd API 时的备用方案：解析 `snap list` 输出。
fn fallback_cli() -> Result<Vec<Package>, String> {
    let output = std::process::Command::new("snap")
        .args(["list", "--color=never"])
        .env("LC_ALL", "C")
        .output()
        .map_err(|err| format!("执行 snap 失败：{err}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut packages = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 2 {
            continue;
        }
        let name = cols[0];
        let version = cols[1];
        let protected = matches!(name, "snapd" | "core" | "bare") || name.starts_with("core");
        packages.push(Package {
            id: format!("snap:{name}"),
            name: name.to_string(),
            display_name: name.to_string(),
            version: version.to_string(),
            source: Source::Snap,
            summary: String::new(),
            keywords: name.to_lowercase(),
            protected,
            section: "app".to_string(),
            ..Default::default()
        });
    }
    Ok(packages)
}

/// 把 C 字符串转换交给系统调用时用到的小工具（保持 libc 依赖为零）。
#[allow(dead_code)]
fn c_string(text: &str) -> Option<CString> {
    CString::new(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_install_date() {
        // 2026-08-26T13:34:41+08:00 == 2026-08-26T05:34:41Z
        let ts = parse_rfc3339("2026-08-26T13:34:41+08:00").unwrap();
        let utc = parse_rfc3339("2026-08-26T05:34:41Z").unwrap();
        assert_eq!(ts, utc);
        assert_eq!(ts, 1787722481);
    }

    #[test]
    fn parses_snapd_payload() {
        let body = r#"{"type":"sync","status-code":200,"status":"OK","result":[
            {"name":"firefox","version":"154.0-1","revision":"8763","type":"app",
             "summary":"Mozilla Firefox","description":"Long description",
             "publisher":{"display-name":"Mozilla","username":"mozilla"},
             "confinement":"strict","installed-size":272957440,
             "install-date":"2026-08-26T13:34:41+08:00","tracking-channel":"latest/stable/ubuntu-26.04"},
            {"name":"core24","version":"20260410","type":"base","confinement":"strict",
             "installed-size":70049792,"summary":"Base snap"}
        ]}"#;
        let packages = parse_snapd_json(body).unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].name, "firefox");
        assert_eq!(packages[0].publisher, "Mozilla");
        assert_eq!(packages[0].size_bytes, 272957440);
        assert!(!packages[0].protected);
        assert!(packages[1].protected, "base snap 应当受保护");
    }

    #[test]
    fn finds_http_body() {
        let raw = b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"result\":[]}";
        assert_eq!(&raw[find_body(raw).unwrap()..], b"{\"result\":[]}");
    }
}
