//! 桌面集成：读取 `.desktop` 文件，给程序配上中文名字、图标和「打开」入口。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::model::Package;

const DESKTOP_DIRS: [&str; 4] = [
    "/var/lib/snapd/desktop/applications",
    "/usr/share/applications",
    "/usr/local/share/applications",
    "/var/lib/flatpak/exports/share/applications",
];

/// 一个 .desktop 文件里我们关心的字段。
#[derive(Debug, Clone, Default)]
pub struct DesktopEntry {
    pub path: String,
    pub name: String,
    pub local_name: Option<String>,
    pub comment: Option<String>,
    pub icon: Option<String>,
    pub exec: Option<String>,
    pub no_display: bool,
}

/// 全部 .desktop 条目的索引。
#[derive(Debug, Default)]
pub struct DesktopIndex {
    entries: HashMap<String, DesktopEntry>,
}

impl DesktopIndex {
    /// 扫描标准位置（含用户目录）的 .desktop 文件。
    pub fn load() -> Self {
        let mut index = DesktopIndex::default();
        let mut dirs: Vec<String> = DESKTOP_DIRS.iter().map(|s| s.to_string()).collect();
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            dirs.push(format!("{}/applications", data_home.to_string_lossy()));
        } else if let Some(home) = std::env::var_os("HOME") {
            dirs.push(format!("{}/.local/share/applications", home.to_string_lossy()));
        }

        for dir in dirs {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(text) = fs::read_to_string(&path).ok() else {
                    continue;
                };
                let Some(parsed) = parse_desktop(&text, &path.to_string_lossy()) else {
                    continue;
                };
                if parsed.no_display {
                    continue;
                }
                index
                    .entries
                    .entry(parsed.path.clone())
                    .or_insert(parsed.clone());

                // 同时按文件名登记一份（不带扩展名），供 apt 包名匹配。
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    index
                        .entries
                        .entry(format!("stem:{stem}"))
                        .or_insert(parsed);
                }
            }
        }
        index
    }

    pub fn get(&self, desktop_file: &str) -> Option<&DesktopEntry> {
        self.entries.get(desktop_file)
    }

    fn by_id(&self, id: &str) -> Option<&DesktopEntry> {
        self.entries
            .get(id)
            .or_else(|| self.entries.get(&format!("stem:{id}")))
            .or_else(|| {
                let file_name = id.rsplit('/').next().unwrap_or(id);
                let stem = file_name.strip_suffix(".desktop").unwrap_or(file_name);
                self.entries.get(&format!("stem:{stem}"))
            })
    }

    /// 给扫描结果补上显示名、图标与 .desktop 路径。
    pub fn attach(&mut self, packages: &mut [Package]) {
        for package in packages.iter_mut() {
            let entry = package
                .desktop_file
                .as_deref()
                .and_then(|path| self.get(path))
                .or_else(|| self.by_id(&package.name))
                .cloned();
            let Some(entry) = entry else {
                continue;
            };
            if package.desktop_file.is_none() {
                package.desktop_file = Some(entry.path.clone());
            }
            if package.icon.is_none() {
                package.icon = entry.icon.clone();
            }
            // 有本地化名字的桌面程序，用它的名字更符合「已安装的程序」这个视角。
            let name = entry.local_name.clone().unwrap_or_else(|| entry.name.clone());
            if !name.is_empty() {
                package.display_name = name;
                package.keywords = format!("{} {}", package.keywords, package.name).to_lowercase();
            }
            if package.summary.is_empty() {
                if let Some(comment) = &entry.comment {
                    package.summary = comment.clone();
                }
            }
        }
    }
}

/// 解析 .desktop 文件。`locale` 用于挑 `Name[zh_CN]` 这类本地化字段。
pub fn parse_desktop(text: &str, path: &str) -> Option<DesktopEntry> {
    let locales = locale_candidates();
    let mut entry = DesktopEntry {
        path: path.to_string(),
        ..Default::default()
    };
    let mut in_main = false;
    let mut localized_names: HashMap<String, String> = HashMap::new();
    let mut localized_comments: HashMap<String, String> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unescape(value.trim());
        if let Some(locale) = key.strip_prefix("Name[").and_then(|k| k.strip_suffix(']')) {
            localized_names.insert(locale.to_string(), value);
            continue;
        }
        if let Some(locale) = key.strip_prefix("Comment[").and_then(|k| k.strip_suffix(']')) {
            localized_comments.insert(locale.to_string(), value);
            continue;
        }
        match key {
            "Name" => entry.name = value,
            "Comment" => entry.comment = Some(value),
            "Icon" => entry.icon = Some(value),
            "Exec" => entry.exec = Some(value),
            "NoDisplay" => entry.no_display = value == "true",
            "Type" => {
                if value != "Application" {
                    return None;
                }
            }
            _ => {}
        }
    }

    for locale in &locales {
        if let Some(name) = localized_names.get(locale) {
            entry.local_name = Some(name.clone());
            break;
        }
    }
    for locale in &locales {
        if let Some(comment) = localized_comments.get(locale) {
            entry.comment = Some(comment.clone());
            break;
        }
    }

    if entry.name.is_empty() {
        return None;
    }
    Some(entry)
}

/// 当前语言下的候选 locale，从具体到宽泛。
fn locale_candidates() -> Vec<String> {
    let mut locales = Vec::new();
    let lang = std::env::var("LANG").unwrap_or_else(|_| "zh_CN.UTF-8".to_string());
    let base = lang.split(['.', '@']).next().unwrap_or("zh_CN").to_string();
    locales.push(base.clone());
    if let Some(short) = base.split('_').next() {
        if short != base {
            locales.push(short.to_string());
        }
    }
    locales
}

fn unescape(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\\", "\\")
}

/// 直接读一个 .desktop 文件，判断它的程序是否还存在于系统上。
pub fn desktop_entry_runnable(desktop_file: &str) -> bool {
    let Ok(text) = fs::read_to_string(desktop_file) else {
        return false;
    };
    parse_desktop(&text, desktop_file)
        .and_then(|entry| entry.exec)
        .map(|exec| exec_available(&exec))
        .unwrap_or(false)
}

/// 桌面文件指向的可执行文件是否存在（用来判断这个条目还能不能启动）。
fn exec_available(exec: &str) -> bool {
    let program = exec
        .trim_start_matches(|c: char| c.is_whitespace())
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('"');
    if program.is_empty() {
        return false;
    }
    if Path::new(program).is_absolute() {
        return Path::new(program).exists();
    }
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|dir| Path::new(dir).join(program).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chinese_name() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[zh_CN]=文件\nComment=Access files\nExec=nautilus --new-window %U\nIcon=org.gnome.Nautilus\n";
        let entry = parse_desktop(text, "/usr/share/applications/org.gnome.Nautilus.desktop").unwrap();
        assert_eq!(entry.name, "Files");
        assert!(entry.local_name.is_some());
        assert_eq!(entry.icon.as_deref(), Some("org.gnome.Nautilus"));
    }

    #[test]
    fn skips_non_application_entries() {
        let text = "[Desktop Entry]\nType=Link\nName=Website\nURL=https://example.com\n";
        assert!(parse_desktop(text, "/tmp/x.desktop").is_none());
    }

    #[test]
    fn respects_nodisplay() {
        let text = "[Desktop Entry]\nType=Application\nName=Hidden\nNoDisplay=true\n";
        let entry = parse_desktop(text, "/tmp/y.desktop").unwrap();
        assert!(entry.no_display);
    }
}
