//! 程序管家：管理这台电脑上已经安装的程序（APT 与 Snap）。
//!
//! GTK4 + libadwaita 原生应用，直接读取 dpkg 与 snapd 的真实数据。

mod model;
mod system;
mod ui;
mod util;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "dev.nanh.PakManager";

fn main() -> glib::ExitCode {
    if let Some(code) = cli::run() {
        return code;
    }

    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|_| {
        glib::set_application_name("程序管家");
        glib::set_prgname(Some("pakmanager"));
    });

    app.connect_activate(|app| {
        ui::build(app);
        let Some(state) = ui::ui() else {
            return;
        };
        state.window.present();

        if ui::probe::enabled() {
            // 自动化验证模式：只截图与读取状态，不注册菜单和快捷键。
            ui::probe::run(&state);
            return;
        }
        actions::install(app, &state);
    });

    app.run()
}

/// 菜单与快捷键。
mod actions {
    use super::*;
    use crate::ui::{self, Ui};

    pub fn install(app: &adw::Application, state: &std::rc::Rc<Ui>) {
        add_action(app, "refresh", {
            let state = state.clone();
            move || {
                ui::reload(&state);
                state.toast.add_toast(adw::Toast::new("正在刷新…"));
            }
        });

        for (name, key) in [
            ("sort-name", ui::SortKey::Name),
            ("sort-size", ui::SortKey::Size),
            ("sort-date", ui::SortKey::Date),
        ] {
            let state = state.clone();
            add_action(app, name, move || {
                state.filters.borrow_mut().sort = Some(key);
                ui::refresh_filter(&state);
            });
        }

        {
            let state = state.clone();
            add_action(app, "clean-leftovers", move || {
                ui::actions::clean_leftovers(&state);
            });
        }

        {
            let state = state.clone();
            add_action(app, "about", move || show_about(&state));
        }

        // 受保护系统包的显示开关（有状态动作，菜单里会显示勾选）。
        let toggle = gio::SimpleAction::new_stateful("toggle-protected", None, &false.to_variant());
        {
            let state = state.clone();
            toggle.connect_activate(move |action, _| {
                let current = action
                    .state()
                    .and_then(|value| value.get::<bool>())
                    .unwrap_or(false);
                action.set_state(&(!current).to_variant());
                state.filters.borrow_mut().show_protected = !current;
                ui::refresh_filter(&state);
            });
        }
        app.add_action(&toggle);

        // 快捷键
        add_action(app, "focus-search", {
            let state = state.clone();
            move || {
                state.search.grab_focus();
            }
        });
        app.set_accels_for_action("app.focus-search", &["<Control>f"]);

        add_action(app, "refresh-shortcut", {
            let state = state.clone();
            move || ui::reload(&state)
        });
        app.set_accels_for_action("app.refresh-shortcut", &["<Control>r"]);

        add_action(app, "clear-selection", {
            let state = state.clone();
            move || {
                state.selection.set_selected(gtk::INVALID_LIST_POSITION);
                state.detail.show(None);
            }
        });
        app.set_accels_for_action("app.clear-selection", &["Escape"]);

        add_action(app, "remove-selected", {
            let state = state.clone();
            move || {
                let Some(package) = state
                    .selection
                    .selected_item()
                    .and_then(|object| ui::package_of(&object))
                else {
                    return;
                };
                ui::actions::confirm_remove(&state, package, &state.detail.remove_button());
            }
        });
        app.set_accels_for_action("app.remove-selected", &["Delete"]);
    }

    fn add_action(app: &adw::Application, name: &str, handler: impl Fn() + 'static) {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| handler());
        app.add_action(&action);
    }

    fn show_about(state: &Ui) {
        let about = adw::AboutDialog::new();
        about.set_application_name("程序管家");
        about.set_application_icon("dev.nanh.PakManager");
        about.set_version(env!("CARGO_PKG_VERSION"));
        about.set_developer_name("nanh");
        about.set_comments(
            "管理这台电脑上已经安装的程序：查看占用、搜索、安全卸载。\n\
             直接读取 dpkg 与 snapd 的真实数据。",
        );
        about.set_license_type(gtk::License::MitX11);
        about.set_developers(&["nanh"]);
        about.present(Some(&state.window));
    }
}

/// 命令行诊断子命令（不打开窗口）。
mod cli {
    use super::*;
    use crate::model::{human_bytes, Source};
    use crate::system;

    pub fn run() -> Option<glib::ExitCode> {
        let args: Vec<String> = std::env::args().collect();
        let flag = args.iter().position(|arg| arg.starts_with("--"))?;
        let command = args[flag].as_str();
        let value = args.get(flag + 1).cloned().unwrap_or_default();

        match command {
            "--dump" => Some(dump()),
            "--plan" => Some(plan(&value)),
            "--remove" => Some(remove(&value, args.iter().any(|arg| arg == "--clean"))),
            "--help" => {
                println!(
                    "程序管家\n\
                     \n用法：\n  pakmanager                打开图形界面\n\
                     \x20 pakmanager --dump         打印扫描统计\n\
                     \x20 pakmanager --plan <包名>   预演卸载影响（只读）\n\
                     \x20 pakmanager --remove <包名> [--clean]  执行卸载"
                );
                Some(glib::ExitCode::SUCCESS)
            }
            _ => None,
        }
    }

    fn dump() -> glib::ExitCode {
        let result = system::scan(&|_| {});
        let apt = result
            .packages
            .iter()
            .filter(|package| package.source == Source::Apt)
            .count();
        let snap = result.packages.len() - apt;
        let protected = result.packages.iter().filter(|p| p.protected).count();
        let desktop = result
            .packages
            .iter()
            .filter(|p| p.is_desktop_app())
            .count();
        let with_icon = result.packages.iter().filter(|p| p.icon.is_some()).count();
        let total: u64 = result.packages.iter().map(|p| p.size_bytes).sum();

        println!("APT 已安装: {apt}");
        println!("Snap 已安装: {snap}");
        println!("总计: {}", result.packages.len());
        println!("受保护: {protected}");
        println!("桌面应用: {desktop}");
        println!("有图标: {with_icon}");
        println!("残留配置: {}", result.leftovers.len());
        println!("总占用: {}", human_bytes(total));
        for warning in &result.warnings {
            println!("警告: {warning}");
        }
        glib::ExitCode::SUCCESS
    }

    fn plan(name: &str) -> glib::ExitCode {
        let Some(package) = find(name) else {
            eprintln!("没有找到已安装的包：{name}");
            return glib::ExitCode::FAILURE;
        };
        let plan = system::ops::plan_remove(&package);
        println!(
            "ok={} removes={:?} auto={:?} reverse_deps={:?} error={}",
            plan.ok, plan.removes, plan.auto_removed, plan.reverse_deps, plan.error
        );
        glib::ExitCode::SUCCESS
    }

    fn remove(name: &str, clean: bool) -> glib::ExitCode {
        let Some(package) = find(name) else {
            eprintln!("没有找到已安装的包：{name}");
            return glib::ExitCode::FAILURE;
        };
        let output = system::ops::remove(&package, clean);
        println!("success={} message={}", output.success, output.message());
        if output.success {
            glib::ExitCode::SUCCESS
        } else {
            glib::ExitCode::FAILURE
        }
    }

    fn find(name: &str) -> Option<crate::model::Package> {
        let result = system::scan(&|_| {});
        result
            .packages
            .iter()
            .chain(result.leftovers.iter())
            .find(|package| package.name == name)
            .cloned()
    }
}
