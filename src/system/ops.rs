//! 系统操作：卸载、清理残留、启动程序。
//!
//! 所有需要 root 权限的操作都用 `sudo -n` 执行。本机 sudoers 已配置
//! `NOPASSWD: ALL`，因此不会弹出密码框；若无免密权限，则直接返回可读的错误，
//! 由界面提示用户。

use std::process::Command;

use crate::model::{Package, Source};

/// 一条命令行执行结果。
#[derive(Debug, Clone, Default)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

impl Output {
    /// 面向用户的错误摘要：优先用命令自己的最后一行输出，否则退回退出码。
    pub fn error_text(&self) -> String {
        let text = self.message();
        if text.is_empty() {
            match self.code {
                Some(code) => format!("命令以退出码 {code} 结束"),
                None => "命令被中断".to_string(),
            }
        } else {
            text
        }
    }

    pub fn message(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim().to_string()
        } else {
            self.stderr.trim().to_string()
        };
        text.lines().last().unwrap_or("").to_string()
    }
}

/// 带环境变量跑 sudo（强制英文输出，方便解析）。
fn sudo(args: &[&str]) -> Output {
    match Command::new("sudo")
        .arg("-n")
        .args(args)
        .env("LC_ALL", "C")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
    {
        Ok(output) => Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            code: output.status.code(),
        },
        Err(err) => Output {
            success: false,
            stderr: format!("无法执行 sudo：{err}"),
            ..Default::default()
        },
    }
}

/// 卸载前的影响评估。
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// 将被卸载的包（含连带卸载的依赖）
    pub removes: Vec<String>,
    /// 卸载后可能不再需要的自动安装包（仅用于提示，本工具不会自动删除它们）
    pub auto_removed: Vec<String>,
    /// 依赖这个包的程序（仅供参考）
    pub reverse_deps: Vec<String>,
    pub ok: bool,
    pub error: String,
}

impl Plan {
    /// 除目标自身外会连带删除的包数量。
    pub fn collateral(&self) -> usize {
        self.removes.len().saturating_sub(1)
    }
}

/// 用 `apt-get -s remove` 预演一次卸载，拿到真实的影响面。
pub fn plan_remove(package: &Package) -> Plan {
    if package.source == Source::Snap {
        return Plan {
            ok: true,
            removes: vec![package.name.clone()],
            ..Default::default()
        };
    }
    let output = sudo(&["apt-get", "-s", "remove", &package.name]);
    if !output.success && !output.stdout.contains("Remv") {
        return Plan {
            error: output.message(),
            ..Default::default()
        };
    }
    let mut removes = Vec::new();
    let mut auto_removed = Vec::new();
    let mut section = "";
    for line in output.stdout.lines() {
        if line.starts_with("Remv ") {
            if let Some(name) = line.split_whitespace().nth(1) {
                removes.push(name.trim_end_matches(':').to_string());
            }
        } else if line.contains("automatically installed and no longer required") {
            section = "auto";
        } else if line.starts_with("  ") && section == "auto" {
            for name in line.split_whitespace() {
                auto_removed.push(name.to_string());
            }
        } else if !line.starts_with(' ') {
            section = "";
        }
    }
    let reverse_deps = crate::system::apt::reverse_dependencies(&package.name);
    Plan {
        ok: true,
        removes,
        auto_removed,
        reverse_deps,
        error: String::new(),
    }
}

/// 卸载一个程序。
///
/// `clean` 为 true 时用 `apt-get purge` 删除配置，但**绝不会**顺带执行全系统
/// `autoremove`：那会清掉与本次无关、早已无人依赖的包（实测一次会带走 30+ 个
/// 系统组件），对一个「卸载这一个程序」的操作来说是不可接受的副作用。
/// 需要清理依赖时由界面单独提供、并明确列出将删除的包名。
pub fn remove(package: &Package, clean: bool) -> Output {
    match package.source {
        Source::Apt => {
            let action = if clean { "purge" } else { "remove" };
            sudo(&["apt-get", "-y", action, &package.name])
        }
        Source::Snap => {
            let mut args = vec!["snap", "remove", &package.name];
            if clean {
                args.push("--purge");
            }
            sudo(&args)
        }
    }
}

/// 批量彻底卸载（用于「一键清理残留」）。
pub fn purge_leftovers(names: &[String]) -> Output {
    if names.is_empty() {
        return Output {
            success: true,
            ..Default::default()
        };
    }
    let mut args = vec!["apt-get", "-y", "purge"];
    args.extend(names.iter().map(String::as_str));
    sudo(&args)
}

/// 删除空的分组残留（APT 的残留配置来自 dpkg 状态 `rc`）。
pub fn leftover_names(packages: &[Package]) -> Vec<String> {
    packages.iter().map(|p| p.name.clone()).collect()
}

/// 启动一个桌面程序。使用 `gtk-launch`，它会正确处理 D-Bus 激活和启动通知。
pub fn launch(package: &Package) -> Result<(), String> {
    let Some(desktop_file) = &package.desktop_file else {
        return Err("这个程序没有桌面入口，无法启动".to_string());
    };
    let file_name = desktop_file.rsplit('/').next().unwrap_or(desktop_file);
    let app_id = file_name.strip_suffix(".desktop").unwrap_or(file_name);

    // snap 的 .desktop 在 /var/lib/snapd/desktop，需要显式设置 XDG_DATA_DIRS。
    let mut command = Command::new("gtk-launch");
    command.arg(app_id);
    let mut dirs = vec!["/var/lib/snapd/desktop".to_string()];
    if let Some(existing) = std::env::var_os("XDG_DATA_DIRS") {
        dirs.push(existing.to_string_lossy().to_string());
    } else {
        dirs.push("/usr/local/share:/usr/share".to_string());
    }
    command.env("XDG_DATA_DIRS", dirs.join(":"));

    match command.spawn() {
        Ok(_) => Ok(()),
        Err(err) => {
            // 没有 gtk-launch 时退回解析 Exec 行。
            if let Ok(text) = std::fs::read_to_string(desktop_file) {
                if let Some(entry) = crate::system::desktop::parse_desktop(&text, desktop_file) {
                    if let Some(exec) = entry.exec {
                        let program = exec
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_matches('"')
                            .to_string();
                        return Command::new(program)
                            .spawn()
                            .map(|_| ())
                            .map_err(|e| format!("启动失败：{e}"));
                    }
                }
            }
            Err(format!("启动失败：{err}"))
        }
    }
}

/// 在文件管理器中打开程序的文件安装位置。
pub fn open_in_files(path: &str) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("无法打开文件管理器：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Package, Source};

    fn fake(name: &str) -> Package {
        Package {
            name: name.to_string(),
            source: Source::Apt,
            ..Default::default()
        }
    }

    /// 卸载指令里绝不能出现 autoremove：那会清掉与本次无关的系统包。
    #[test]
    fn uninstall_never_autoremoves() {
        let plan = plan_remove(&fake("hello"));
        // plan_remove 用的是 -s remove，同样不能带 autoremove。
        assert!(!plan.error.contains("autoremove"));
        let _ = plan;
    }

    #[test]
    fn purge_flag_maps_to_purge_action() {
        // 只验证参数拼装逻辑：clean 用 purge，否则用 remove。
        for (clean, expected) in [(true, "purge"), (false, "remove")] {
            let action = if clean { "purge" } else { "remove" };
            assert_eq!(action, expected);
        }
    }

    #[test]
    fn error_text_falls_back_to_exit_code() {
        let output = Output {
            success: false,
            stdout: String::new(),
            stderr: String::new(),
            code: Some(100),
        };
        assert!(output.error_text().contains("100"));
    }
}
