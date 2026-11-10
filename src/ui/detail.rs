//! 右侧详情面板：选中程序的完整信息与操作。

use std::cell::RefCell;

use adw::prelude::*;

use crate::model::{section_label, Package, PackageRef};
use crate::system::ops;
use crate::util;


/// 详情面板。
pub struct DetailPane {
    pub root: gtk::Box,
    stack: gtk::Stack,
    body: gtk::Box,
    icon: gtk::Image,
    title: gtk::Label,
    subtitle: gtk::Label,
    source_badge: gtk::Label,
    protected_badge: gtk::Label,
    description: gtk::Label,
    facts: gtk::Box,
    extras: gtk::Box,
    install_button: gtk::Button,
    reveal_button: gtk::Button,
    remove_button: gtk::Button,
    current: RefCell<Option<PackageRef>>,
}

impl DetailPane {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("detail-pane");
        root.set_width_request(360);

        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);

        // ---- 空状态 ----
        let empty_title = gtk::Label::new(Some("没有选中程序"));
        empty_title.add_css_class("pm-empty-title");
        let empty_sub = gtk::Label::new(Some("从左侧选择一个程序，查看它的信息并卸载"));
        empty_sub.add_css_class("pm-empty-sub");
        empty_sub.set_wrap(true);
        empty_sub.set_justify(gtk::Justification::Center);
        empty_sub.set_max_width_chars(28);

        let empty_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        empty_box.set_valign(gtk::Align::Center);
        empty_box.set_halign(gtk::Align::Center);
        empty_box.set_margin_start(24);
        empty_box.set_margin_end(24);
        let empty_icon = gtk::Image::from_icon_name("package-x-generic-symbolic");
        empty_icon.set_pixel_size(56);
        empty_icon.add_css_class("dim-label");
        empty_icon.set_margin_bottom(6);
        empty_box.append(&empty_icon);
        empty_box.append(&empty_title);
        empty_box.append(&empty_sub);
        stack.add_named(&empty_box, Some("empty"));

        // ---- 详情 ----
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_margin_top(16);
        body.set_margin_bottom(16);
        body.set_margin_start(18);
        body.set_margin_end(18);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = gtk::Image::new();
        icon.set_pixel_size(48);
        icon.set_valign(gtk::Align::Start);
        let head_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head_text.set_hexpand(true);
        let title = gtk::Label::new(None);
        title.add_css_class("detail-title");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("detail-sub");
        subtitle.set_xalign(0.0);
        subtitle.set_selectable(true);
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

        let badges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        badges.set_margin_top(4);
        let source_badge = gtk::Label::new(None);
        source_badge.add_css_class("pm-source-badge");
        let protected_badge = gtk::Label::new(Some("系统保护"));
        protected_badge.add_css_class("pm-protected-badge");
        protected_badge.set_visible(false);
        badges.append(&source_badge);
        badges.append(&protected_badge);

        head_text.append(&title);
        head_text.append(&subtitle);
        head_text.append(&badges);
        head.append(&icon);
        head.append(&head_text);
        body.append(&head);

        let description = gtk::Label::new(None);
        description.add_css_class("detail-desc");
        description.set_xalign(0.0);
        description.set_wrap(true);
        description.set_margin_top(14);
        description.set_lines(4);
        description.set_ellipsize(gtk::pango::EllipsizeMode::End);
        body.append(&description);

        let facts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        facts.set_margin_top(14);
        body.append(&facts);

        // 依赖与文件清单：默认折叠，点开才去读，避免每次选中都扫磁盘。
        let extras = gtk::Box::new(gtk::Orientation::Vertical, 0);
        extras.set_margin_top(12);
        body.append(&extras);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_propagate_natural_width(true);
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scrolled.set_child(Some(&body));
        stack.add_named(&scrolled, Some("detail"));
        stack.set_visible_child_name("empty");

        // ---- 底部操作条 ----
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_margin_top(10);
        actions.set_margin_bottom(12);
        actions.set_margin_start(18);
        actions.set_margin_end(18);

        let install_button = gtk::Button::with_label("打开");
        install_button.add_css_class("suggested-action");
        install_button.set_visible(false);
        install_button.set_tooltip_text(Some("启动这个程序"));

        let reveal_button = gtk::Button::with_label("位置");
        reveal_button.add_css_class("flat");
        reveal_button.set_tooltip_text(Some("在文件管理器中打开安装位置"));

        let remove_button = gtk::Button::with_label("卸载…");
        remove_button.add_css_class("destructive-action");
        remove_button.set_sensitive(false);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        actions.append(&install_button);
        actions.append(&reveal_button);
        actions.append(&spacer);
        actions.append(&remove_button);

        root.append(&stack);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        root.append(&actions);

        let pane = Self {
            root,
            stack,
            body,
            icon,
            title,
            subtitle,
            source_badge,
            protected_badge,
            description,
            facts,
            extras,
            install_button,
            reveal_button,
            remove_button,
            current: RefCell::new(None),
        };
        pane.wire();
        pane
    }

    fn wire(&self) {
        self.install_button.connect_clicked(|_| {
            let Some(ui) = super::ui() else { return };
            let Some(package) = ui.detail.current.borrow().clone() else {
                return;
            };
            if let Err(err) = ops::launch(&package) {
                ui.toast.add_toast(adw::Toast::new(&err));
            }
        });

        self.reveal_button.connect_clicked(|_| {
            let Some(ui) = super::ui() else { return };
            let Some(package) = ui.detail.current.borrow().clone() else {
                return;
            };
            if let Err(err) = super::actions::reveal(&package) {
                ui.toast.add_toast(adw::Toast::new(&err));
            }
        });

        self.remove_button.connect_clicked(|button| {
            let Some(ui) = super::ui() else { return };
            let Some(package) = ui.detail.current.borrow().clone() else {
                return;
            };
            super::actions::confirm_remove(&ui, package, button);
        });
    }

    /// 展示一个程序（传 None 回到空状态）。
    pub fn show(&self, package: Option<PackageRef>) {
        let Some(package) = package else {
            self.stack.set_visible_child_name("empty");
            self.install_button.set_visible(false);
            self.remove_button.set_sensitive(false);
            *self.current.borrow_mut() = None;
            return;
        };

        self.stack.set_visible_child_name("detail");
        *self.current.borrow_mut() = Some(package.clone());

        self.title.set_text(&package.display_name);
        if package.display_name != package.name {
            self.subtitle.set_text(&package.name);
        } else {
            self.subtitle
                .set_text(&format!("{} · {}", package.source.label(), package.version));
        }
        self.source_badge.set_text(package.source.label());
        self.protected_badge.set_visible(package.protected);

        let icon = package.icon.clone().unwrap_or_else(|| {
            if package.is_desktop_app() || package.is_snap() {
                "application-x-executable".to_string()
            } else {
                "package-x-generic".to_string()
            }
        });
        if icon.starts_with('/') {
            self.icon.set_from_file(Some(icon));
        } else {
            self.icon.set_icon_name(Some(&icon));
        }

        let text = if package.description.is_empty() {
            package.summary.clone()
        } else {
            format!("{}\n\n{}", package.summary, util::ellipsize(&package.description, 480))
        };
        self.description.set_text(text.trim());

        populate_facts(&self.facts, &package);
        populate_extras(&self.extras, &package);

        // 有 .desktop 但可执行文件已不在（卸载不干净的常见症状）时不提供「打开」。
        let runnable = package
            .desktop_file
            .as_deref()
            .map(crate::system::desktop::desktop_entry_runnable)
            .unwrap_or(false);
        self.install_button
            .set_visible(package.is_desktop_app() && !package.leftover && runnable);
        self.install_button.set_tooltip_text(Some(if runnable {
            "启动这个程序"
        } else {
            "桌面入口存在，但程序文件缺失"
        }));
        self.reveal_button.set_sensitive(!package.leftover);
        self.remove_button.set_sensitive(!package.protected);
        if package.protected {
            self.remove_button
                .set_tooltip_text(Some("这是系统必需组件，卸载会破坏系统，已禁用"));
        } else if package.leftover {
            self.remove_button.set_label("清理配置…");
            self.remove_button
                .set_tooltip_text(Some("删除这个包残留的配置文件"));
        } else {
            self.remove_button.set_label("卸载…");
            self.remove_button
                .set_tooltip_text(Some("卸载这个程序，可选是否同时清除配置"));
        }
    }

    /// 用于自动化测试：当前展示的包名。
    pub fn current_name(&self) -> Option<String> {
        self.current.borrow().as_ref().map(|p| p.name.clone())
    }

    /// 卸载按钮：既是确认对话框的锚点，也是执行期间的禁用目标。
    pub fn remove_button(&self) -> gtk::Button {
        self.remove_button.clone()
    }

    /// 详情正文容器，自动化验证会读它的文本。
    pub fn body(&self) -> &gtk::Box {
        &self.body
    }
}

/// 生成「详情」里的键值行。
fn populate_facts(container: &gtk::Box, package: &Package) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push(("版本", package.version.clone()));
    rows.push(("来源", package.source.label().to_string()));
    if package.is_snap() {
        if !package.publisher.is_empty() {
            rows.push(("发布者", package.publisher.clone()));
        }
        if !package.confinement.is_empty() {
            rows.push((
                "沙盒",
                if package.confinement == "classic" {
                    "classic（无沙盒限制）".to_string()
                } else {
                    "strict（受限沙盒）".to_string()
                },
            ));
        }
        if !package.section.is_empty() {
            rows.push(("类型", snap_type_label(&package.section)));
        }
    } else {
        rows.push(("分类", section_label(&package.section).to_string()));
        if package.auto_installed {
            rows.push(("安装方式", "作为依赖自动安装".to_string()));
        }
        if !package.maintainer.is_empty() {
            rows.push(("维护者", package.maintainer.clone()));
        }
    }
    rows.push(("占用空间", package.size_human()));
    rows.push(("安装时间", package.installed_text()));
    if package.essential {
        rows.push(("标记", "Essential（系统必需）".to_string()));
    }

    for (key, value) in rows {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(5);
        row.set_margin_bottom(5);
        let key_label = gtk::Label::new(Some(key));
        key_label.add_css_class("detail-key");
        key_label.set_xalign(0.0);
        key_label.set_width_chars(6);
        key_label.set_valign(gtk::Align::Start);
        let value_label = gtk::Label::new(Some(&value));
        value_label.add_css_class("detail-value");
        value_label.set_xalign(0.0);
        value_label.set_hexpand(true);
        value_label.set_wrap(true);
        value_label.set_selectable(true);
        row.append(&key_label);
        row.append(&value_label);
        container.append(&row);
    }
}

/// 依赖与文件清单（可折叠，展开时才读取）。
fn populate_extras(container: &gtk::Box, package: &Package) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if package.leftover {
        return;
    }

    if !package.is_snap() {
        let deps = crate::system::apt::reverse_dependencies(&package.name);
        let text = if deps.is_empty() {
            "没有其它已安装的包依赖它".to_string()
        } else {
            format!("{} 个已安装包依赖它：\n{}", deps.len(), deps.join("、"))
        };
        container.append(&expander("谁依赖它", &text));
    }

    if !package.is_snap() {
        let name = package.name.clone();
        container.append(&lazy_expander("文件清单", move || {
            let files = crate::system::apt::file_list(&name);
            if files.is_empty() {
                "没有读取到文件清单（可能需要 root 权限）".to_string()
            } else {
                format!("共 {} 个文件：\n{}", files.len(), util::ellipsize(&files.join("\n"), 4000))
            }
        }));
    }
}

/// 静态文本的折叠区。
fn expander(title: &str, text: &str) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(title);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_selectable(true);
    label.add_css_class("detail-desc");
    label.set_margin_top(6);
    label.set_margin_bottom(10);
    label.set_margin_start(6);
    label.set_margin_end(6);
    row.add_row(&label);
    row
}

/// 展开时才计算内容的折叠区。
fn lazy_expander(title: &str, compute: impl Fn() -> String + 'static) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(title);
    let label = gtk::Label::new(Some("…"));
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.add_css_class("detail-desc");
    label.set_margin_top(6);
    label.set_margin_bottom(10);
    label.set_margin_start(6);
    label.set_margin_end(6);
    // 文件清单可能上千行，用等宽小字并且不自动换行，靠滚动查看。
    label.set_wrap(false);
    label.set_lines(10);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.add_row(&label);
    let compute = std::rc::Rc::new(compute);
    row.connect_expanded_notify(move |row| {
        if row.is_expanded() {
            label.set_text(&compute());
        }
    });
    row
}

fn snap_type_label(snap_type: &str) -> String {
    match snap_type {
        "app" => "应用程序",
        "base" => "基础运行时",
        "kernel" => "内核",
        "gadget" => "设备适配",
        "snapd" => "snapd 本体",
        other => other,
    }
    .to_string()
}
