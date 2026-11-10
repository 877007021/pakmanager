//! 操作流程：卸载确认、进度提示、执行与刷新。

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::model::{human_bytes, Package, PackageRef};
use crate::system::ops;

use super::Ui;

/// 弹出卸载确认对话框：先预演卸载、展示真实影响面，再执行。
pub fn confirm_remove(ui: &Rc<Ui>, package: PackageRef, anchor: &gtk::Button) {
    let is_leftover = package.leftover;
    let action = if is_leftover {
        "清理配置"
    } else {
        "卸载"
    };

    let dialog = adw::AlertDialog::new(
        Some(&format!("{action} {}？", package.display_name)),
        Some("正在评估这次操作的影响…"),
    );
    dialog.add_response("cancel", "取消");
    dialog.add_response("clean", "同时删除配置");
    dialog.add_response("remove", if is_leftover { "清理" } else { "仅卸载" });
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_response_appearance("clean", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    // 先展示对话框，影响面在后台算完后再填进去，避免卡住界面。
    let (sender, receiver) = std::sync::mpsc::channel();
    let plain = package.as_ref().clone();
    std::thread::spawn(move || {
        let _ = sender.send(ops::plan_remove(&plain));
    });
    let dialog_for_plan = dialog.clone();
    let package_for_plan = package.clone();
    glib::timeout_add_local(
        std::time::Duration::from_millis(60),
        move || match receiver.try_recv() {
            Ok(plan) => {
                let body = describe_plan(&package_for_plan, &plan);
                dialog_for_plan.set_body(&body);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );

    let weak = Rc::downgrade(ui);
    let anchor_for_dialog = anchor.clone();
    let anchor_for_remove = anchor.clone();
    dialog.choose(
        Some(&anchor_for_dialog),
        gio::Cancellable::NONE,
        move |response| {
            let Some(ui) = weak.upgrade() else { return };
            let response = response.as_str().to_string();
            if response == "cancel" {
                return;
            }
            run_remove(
                &ui,
                package.clone(),
                response == "clean",
                &anchor_for_remove,
            );
        },
    );
}

/// 在文件管理器中打开这个包的安装目录（供详情面板的「位置」按钮使用）。
pub fn reveal(package: &Package) -> Result<(), String> {
    let path = if package.is_snap() {
        format!("/snap/{}/current", package.name)
    } else {
        "/usr/share/applications".to_string()
    };
    if !std::path::Path::new(&path).exists() {
        return Err("这个程序没有可浏览的安装位置".to_string());
    }
    ops::open_in_files(&path)
}

/// 把预演结果写成人能读的影响描述。
fn describe_plan(package: &Package, plan: &ops::Plan) -> String {
    if !plan.error.is_empty() {
        return format!("无法评估影响：{}", plan.error);
    }
    if package.is_snap() {
        if package.protected {
            return "这是 snapd 的运行时组件，不建议删除。".to_string();
        }
        return format!(
            "将删除 snap 应用「{}」，占用 {}。",
            package.display_name,
            human_bytes(package.size_bytes)
        );
    }

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!(
        "将卸载「{}」{}，释放约 {}。",
        package.name,
        if package.auto_installed {
            "（自动安装的依赖）"
        } else {
            ""
        },
        human_bytes(package.size_bytes)
    ));
    let collateral = plan.collateral();
    if collateral > 0 {
        let mut names = plan.removes.clone();
        names.retain(|name| name != &package.name);
        let shown = names.iter().take(6).cloned().collect::<Vec<_>>().join("、");
        let extra = if names.len() > 6 {
            format!(" 等 {} 个包", names.len())
        } else {
            String::new()
        };
        parts.push(format!("会连带卸载：{shown}{extra}。"));
    }
    if !plan.auto_removed.is_empty() {
        // autoremove 只是提示，不会随这次操作执行；如实说明，避免用户以为会一并清理。
        parts.push(format!(
            "另有 {} 个自动安装的依赖在卸载后可能不再需要（不会自动删除，可稍后用「清理残留」处理）。",
            plan.auto_removed.len()
        ));
    }
    if !plan.reverse_deps.is_empty() {
        let shown = plan
            .reverse_deps
            .iter()
            .take(4)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        let more = if plan.reverse_deps.len() > 4 {
            " 等"
        } else {
            ""
        };
        parts.push(format!(
            "有 {}{} 个已安装包依赖它，卸载可能影响它们。",
            shown, more
        ));
    }
    if collateral == 0 && plan.auto_removed.is_empty() && plan.reverse_deps.is_empty() {
        parts.push("不会影响其它已安装的包。".to_string());
    }
    parts.join("\n")
}

/// 在后台线程执行卸载，完成后刷新列表。
pub fn run_remove(ui: &Rc<Ui>, package: PackageRef, clean: bool, anchor: &gtk::Button) {
    let anchor = anchor.clone();
    anchor.set_sensitive(false);
    let toast = adw::Toast::new(&format!("正在卸载 {}…", package.display_name));
    toast.set_timeout(0);
    ui.toast.add_toast(toast.clone());
    ui.window.set_cursor_from_name(Some("progress"));

    // 用后台线程执行，避免阻塞主线程。
    let (sender, receiver) = std::sync::mpsc::channel();
    let package_for_task = package.as_ref().clone();
    std::thread::spawn(move || {
        let result = ops::remove(&package_for_task, clean);
        let _ = sender.send(result);
    });

    let weak = Rc::downgrade(ui);
    glib::timeout_add_local(
        std::time::Duration::from_millis(80),
        move || match receiver.try_recv() {
            Ok(output) => {
                toast.dismiss();
                let Some(ui) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                ui.window.set_cursor(None);
                anchor.set_sensitive(true);
                if output.success {
                    let message = if clean {
                        format!("{} 已彻底删除", package.name)
                    } else {
                        format!("{} 已卸载", package.name)
                    };
                    ui.toast.add_toast(adw::Toast::new(&message));
                } else {
                    let detail = output.error_text();
                    let toast = adw::Toast::new(&format!("卸载失败：{detail}"));
                    toast.set_timeout(8);
                    ui.toast.add_toast(toast);
                }
                ui.detail.show(None);
                super::reload(&ui);
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );
}

/// 一键清理所有残留配置（dpkg 状态 rc）。
pub fn clean_leftovers(ui: &Rc<Ui>) {
    let leftovers: Vec<Package> = ui
        .pending
        .borrow()
        .as_ref()
        .map(|result| result.leftovers.clone())
        .unwrap_or_default();
    if leftovers.is_empty() {
        ui.toast.add_toast(adw::Toast::new("没有残留配置需要清理"));
        return;
    }
    let names = ops::leftover_names(&leftovers);
    let list = names.iter().take(8).cloned().collect::<Vec<_>>().join("\n");
    let more = if names.len() > 8 {
        format!("\n…以及另外 {} 个", names.len() - 8)
    } else {
        String::new()
    };

    let dialog = adw::AlertDialog::new(
        Some(&format!("清理 {} 个残留配置？", names.len())),
        Some(&format!(
            "这些软件包已经被卸载，只剩配置文件占用空间。清理后配置不可恢复。\n\n{list}{more}"
        )),
    );
    dialog.add_response("cancel", "取消");
    dialog.add_response("clean", "全部清理");
    dialog.set_response_appearance("clean", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    let weak = Rc::downgrade(ui);
    dialog.choose(
        Some(&ui.window.clone()),
        gio::Cancellable::NONE,
        move |response| {
            if response.as_str() != "clean" {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            let toast = adw::Toast::new(&format!("正在清理 {} 个残留配置…", names.len()));
            toast.set_timeout(0);
            ui.toast.add_toast(toast.clone());
            let (sender, receiver) = std::sync::mpsc::channel();
            let names = names.clone();
            std::thread::spawn(move || {
                let _ = sender.send(ops::purge_leftovers(&names));
            });
            let weak = Rc::downgrade(&ui);
            glib::timeout_add_local(
                std::time::Duration::from_millis(100),
                move || match receiver.try_recv() {
                    Ok(output) => {
                        toast.dismiss();
                        if let Some(ui) = weak.upgrade() {
                            if output.success {
                                ui.toast.add_toast(adw::Toast::new("残留配置已清理"));
                            } else {
                                let toast =
                                    adw::Toast::new(&format!("清理失败：{}", output.error_text()));
                                toast.set_timeout(8);
                                ui.toast.add_toast(toast);
                            }
                            super::reload(&ui);
                        }
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
                },
            );
        },
    );
}
