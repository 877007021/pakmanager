//! 系统扫描：把 dpkg / snapd 的真实状态整理成统一的 [`Package`] 列表。
//!
//! 所有扫描都是只读的：本模块不会修改系统上的任何东西。

pub mod apt;
pub mod desktop;
pub mod ops;
pub mod snap;

use std::sync::mpsc::Sender;

use crate::model::Package;

/// 一次完整扫描的结果。
#[derive(Debug, Default)]
pub struct ScanResult {
    pub packages: Vec<Package>,
    /// 已卸载但残留配置文件的 APT 包（dpkg 状态 `rc`）。
    pub leftovers: Vec<Package>,
    /// 扫描过程中的非致命问题，会在界面上以提示形式出现。
    pub warnings: Vec<String>,
}

/// 扫描进度回调。
pub type Progress = dyn Fn(&str) + Send + 'static;

/// 同步执行完整扫描。调用方应放在后台线程里跑。
pub fn scan(progress: &Progress) -> ScanResult {
    let mut result = ScanResult::default();

    progress("正在读取 APT 软件包数据库…");
    match apt::scan() {
        Ok(mut apt_result) => {
            result.packages.append(&mut apt_result.packages);
            result.leftovers = apt_result.leftovers;
            result.warnings.append(&mut apt_result.warnings);
        }
        Err(err) => result.warnings.push(format!("APT 扫描失败：{err}")),
    }

    progress("正在读取 Snap 应用…");
    match snap::scan() {
        Ok(mut snaps) => result.packages.append(&mut snaps),
        Err(err) => result.warnings.push(format!("Snap 扫描失败：{err}")),
    }

    progress("正在匹配桌面图标…");
    let mut desktop = desktop::DesktopIndex::load();
    desktop.attach(&mut result.packages);
    desktop.attach(&mut result.leftovers);

    result
        .packages
        .sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    result.leftovers.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

/// 在后台线程执行扫描，结果通过 `Sender` 交回 GTK 主线程。
pub fn scan_async<F>(progress: F, done: Sender<ScanResult>)
where
    F: Fn(&str) + Send + 'static,
{
    std::thread::spawn(move || {
        let result = scan(&progress);
        let _ = done.send(result);
    });
}
