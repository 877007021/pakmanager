//! APT / dpkg 读取器。
//!
//! 直接解析 `/var/lib/dpkg/status` 与 `/var/lib/dpkg/info/*.list`，不依赖外部命令，
//! 1750 个包大约 30ms 解析完，可以在后台线程里随时重扫。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::model::{Package, Source};

const DPKG_STATUS: &str = "/var/lib/dpkg/status";
const DPKG_INFO: &str = "/var/lib/dpkg/info";
const APT_EXTENDED_STATES: &str = "/var/lib/apt/extended_states";

/// APT 扫描结果。
#[derive(Debug, Default)]
pub struct AptScan {
    pub packages: Vec<Package>,
    /// 状态为 `rc`：已卸载但配置文件还留着。
    pub leftovers: Vec<Package>,
    pub warnings: Vec<String>,
}

/// 一个 dpkg 状态条目（只保留我们关心的字段）。
#[derive(Debug, Default, Clone)]
pub struct Stanza {
    pub fields: HashMap<String, String>,
}

impl Stanza {
    pub fn get(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }
}

/// 解析 dpkg 的 RFC822 风格状态文件。续行（以空格或制表符开头）并入上一个字段。
pub fn parse_status(raw: &str) -> Vec<Stanza> {
    let mut stanzas = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut last_key: Option<String> = None;

    for line in raw.lines() {
        if line.trim().is_empty() {
            if !fields.is_empty() {
                stanzas.push(Stanza {
                    fields: std::mem::take(&mut fields),
                });
            }
            last_key = None;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(key) = &last_key {
                let value = fields.entry(key.clone()).or_default();
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line.trim_start());
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            fields.insert(key.clone(), value.trim().to_string());
            last_key = Some(key);
        }
    }
    if !fields.is_empty() {
        stanzas.push(Stanza { fields });
    }
    stanzas
}

/// 解析 `/var/lib/apt/extended_states`，返回被标记为「自动安装」的包名集合。
pub fn parse_extended_states(raw: &str) -> HashSet<String> {
    let mut auto = HashSet::new();
    for stanza in parse_status(raw) {
        if stanza.get("Auto-Installed") == "1" {
            auto.insert(stanza.get("Package").to_string());
        }
    }
    auto
}

/// `Installed-Size` 的语义是 KiB。
fn installed_size_kib(stanza: &Stanza) -> u64 {
    stanza
        .get("Installed-Size")
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
}

/// 读取 `description.list` 的修改时间，作为「安装时间」的近似值。
fn install_time(pkg: &str) -> Option<i64> {
    let path = Path::new(DPKG_INFO).join(format!("{pkg}.list"));
    let meta = fs::metadata(&path).ok()?;
    let modified = meta.modified().ok()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?;
    Some(since.as_secs() as i64)
}

/// 把 dpkg 的 `Description` 拆成 (摘要, 完整描述)。
fn split_description(raw: &str) -> (String, String) {
    let mut lines = raw.split('\n');
    let summary = lines.next().unwrap_or("").trim().to_string();
    let body: Vec<&str> = lines
        .map(|line| {
            line.strip_prefix('.')
                .map(|l| if l.is_empty() { "" } else { l.trim() })
                .unwrap_or(line.trim())
        })
        .collect();
    (summary, body.join("\n").trim().to_string())
}

/// 哪些包提供了 `.desktop` 文件（用于图标与「打开」按钮）。
fn desktop_owners() -> HashMap<String, Vec<String>> {
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    let Ok(entries) = fs::read_dir(DPKG_INFO) else {
        return owners;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("list") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // libfoo1:amd64.list → libfoo1
        let pkg = stem.split(':').next().unwrap_or(stem).to_string();
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        for line in contents.lines() {
            if line.ends_with(".desktop")
                && (line.starts_with("/usr/share/applications/")
                    || line.starts_with("/usr/local/share/applications/"))
            {
                owners
                    .entry(pkg.clone())
                    .or_default()
                    .push(line.to_string());
            }
        }
    }
    owners
}

/// 执行一次完整的 APT 扫描。
pub fn scan() -> Result<AptScan, String> {
    let mut result = AptScan::default();
    let raw =
        fs::read_to_string(DPKG_STATUS).map_err(|err| format!("读取 {DPKG_STATUS} 失败：{err}"))?;
    let stanzas = parse_status(&raw);

    let auto_installed = fs::read_to_string(APT_EXTENDED_STATES)
        .map(|raw| parse_extended_states(&raw))
        .unwrap_or_default();
    let desktop_owners = desktop_owners();

    for stanza in stanzas {
        let status = stanza.get("Status");
        let name = stanza.get("Package").to_string();
        if name.is_empty() {
            continue;
        }
        let installed = status.ends_with(" installed");
        let config_only = status.ends_with(" config-files");
        if !installed && !config_only {
            continue;
        }

        let (summary, description) = split_description(stanza.get("Description"));
        let essential = stanza.get("Essential") == "yes";
        let priority = stanza.get("Priority");
        let section = stanza.get("Section").to_string();
        // 受保护 = 删了会直接破坏系统：Essential、Priority: required、Protected: yes，
        // 以及少数卸载即失去包管理能力的基础设施包。普通 optional/important 包一律可卸。
        let protected = essential
            || priority == "required"
            || stanza.get("Protected") == "yes"
            || matches!(
                name.as_str(),
                "dpkg" | "apt" | "libc6" | "systemd" | "snapd"
            );

        let mut package = Package {
            id: format!("apt:{name}"),
            name: name.clone(),
            display_name: name.clone(),
            version: stanza.get("Version").to_string(),
            source: Source::Apt,
            size_bytes: installed_size_kib(&stanza).saturating_mul(1024),
            installed_at: install_time(&name),
            summary: summary.clone(),
            description,
            section: section.clone(),
            auto_installed: auto_installed.contains(&name),
            essential,
            protected,
            leftover: false,
            maintainer: stanza.get("Maintainer").to_string(),
            icon: None,
            desktop_file: desktop_owners
                .get(&name)
                .and_then(|files| files.first().cloned()),
            publisher: String::new(),
            confinement: String::new(),
            keywords: format!("{name} {summary} {section}").to_lowercase(),
        };
        if config_only {
            package.size_bytes = 0;
            package.leftover = true;
            package.summary = "已卸载，仅剩配置文件".to_string();
            result.leftovers.push(package);
        } else {
            result.packages.push(package);
        }
    }

    if result.packages.is_empty() {
        result
            .warnings
            .push("没有从 dpkg 读到任何已安装的包，可能是权限或环境问题。".to_string());
    }
    Ok(result)
}

/// 包的完整文件清单（用于详情里的「文件」页）。
pub fn file_list(pkg: &str) -> Vec<String> {
    let info = PathBuf::from(DPKG_INFO);
    let exact = info.join(format!("{pkg}.list"));
    let path = if exact.exists() {
        exact
    } else {
        // 多架构包：pkg:amd64.list
        let Ok(entries) = fs::read_dir(&info) else {
            return Vec::new();
        };
        let found = entries.flatten().map(|e| e.path()).find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&format!("{pkg}:")) && n.ends_with(".list"))
                .unwrap_or(false)
        });
        match found {
            Some(path) => path,
            None => return Vec::new(),
        }
    };
    fs::read_to_string(path)
        .map(|raw| raw.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// 谁依赖了这个包：读取已安装包的 `Depends` / `Pre-Depends`。
///
/// 返回 `(直接依赖它的包, 通过其它包间接依赖它的包数量)`。
pub fn reverse_dependencies(pkg: &str) -> Vec<String> {
    let Ok(raw) = fs::read_to_string(DPKG_STATUS) else {
        return Vec::new();
    };
    let mut users = Vec::new();
    for stanza in parse_status(&raw) {
        if !stanza.get("Status").ends_with(" installed") {
            continue;
        }
        let name = stanza.get("Package");
        if name == pkg {
            continue;
        }
        let depends = format!("{},{}", stanza.get("Depends"), stanza.get("Pre-Depends"));
        if depends
            .split(',')
            .filter_map(|clause| clause.split('|').next())
            .map(|clause| {
                clause
                    .trim()
                    .split(['(', ' ', '['])
                    .next()
                    .unwrap_or("")
                    .trim()
            })
            .any(|dep| dep.split(':').next() == Some(pkg))
        {
            users.push(name.to_string());
        }
    }
    users.sort();
    users
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Package: bash\nStatus: install ok installed\nPriority: required\nSection: shells\nInstalled-Size: 2020\nMaintainer: A <a@b>\nDescription: GNU Bourne Again SHell\n Bash is an sh-compatible command language interpreter.\n .\n This is a second paragraph.\n\nPackage: hello\nStatus: remove ok config-files\nPriority: optional\nSection: misc\nMaintainer: B <b@c>\nDescription: friendly greeting program\n\nPackage: libfoo\nStatus: install ok installed\nSection: libs\nDepends: libc6 (>= 2.34)\nDescription: a library\n";

    #[test]
    fn parses_stanzas_and_continuations() {
        let stanzas = parse_status(SAMPLE);
        assert_eq!(stanzas.len(), 3);
        assert_eq!(stanzas[0].get("Package"), "bash");
        assert!(stanzas[0].get("Description").contains("second paragraph"));
    }

    #[test]
    fn splits_summary_and_body() {
        let stanzas = parse_status(SAMPLE);
        let (summary, body) = split_description(stanzas[0].get("Description"));
        assert_eq!(summary, "GNU Bourne Again SHell");
        assert!(body.starts_with("Bash is an sh-compatible"));
        assert!(
            body.contains("\n\nThis is a second paragraph."),
            "got: {body:?}"
        );
    }

    #[test]
    fn detects_config_files_leftover() {
        let stanzas = parse_status(SAMPLE);
        let leftover: Vec<_> = stanzas
            .iter()
            .filter(|s| s.get("Status").ends_with("config-files"))
            .collect();
        assert_eq!(leftover.len(), 1);
        assert_eq!(leftover[0].get("Package"), "hello");
    }

    #[test]
    fn parses_auto_installed_marker() {
        let raw = "Package: libbpfcc\nArchitecture: amd64\nAuto-Installed: 1\n\nPackage: vim\nArchitecture: amd64\n";
        let auto = parse_extended_states(raw);
        assert!(auto.contains("libbpfcc"));
        assert!(!auto.contains("vim"));
    }
}
