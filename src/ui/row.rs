//! 列表行：图标 + 名称 + 描述 + 来源/大小/日期。
//!
//! 行的控件树直接以 `gtk::Box` 为根，`SignalListItemFactory` 回收复用它，
//! 绑定数据时沿着固定的子节点顺序取回各个标签。

use gtk::prelude::*;

use crate::model::Package;

/// 行的控件引用集合。
pub struct RowWidgets {
    pub root: gtk::Box,
}

/// 子控件在 Box 中的位置（顺序与 [`bind_new`] 里 append 的顺序一致）。
const ICON: i32 = 0;
const TEXT_BOX: i32 = 1;
const SIZE: i32 = 2;
const DATE: i32 = 3;

/// 建一行空壳，返回根节点。
pub fn bind_new() -> RowWidgets {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    root.add_css_class("pm-row");

    let icon = gtk::Image::new();
    icon.set_pixel_size(28);
    icon.set_valign(gtk::Align::Center);
    icon.set_icon_name(Some("package-x-generic"));

    let text_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text_box.set_valign(gtk::Align::Center);
    text_box.set_hexpand(true);

    let name_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Label::new(None);
    name.add_css_class("pm-row-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let source = gtk::Label::new(None);
    source.add_css_class("pm-source-badge");
    source.set_valign(gtk::Align::Center);
    let protected = gtk::Label::new(Some("系统"));
    protected.add_css_class("pm-protected-badge");
    protected.set_valign(gtk::Align::Center);
    protected.set_visible(false);
    name_box.append(&name);
    name_box.append(&source);
    name_box.append(&protected);

    let summary = gtk::Label::new(None);
    summary.add_css_class("pm-row-sub");
    summary.set_xalign(0.0);
    summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
    summary.set_lines(1);

    text_box.append(&name_box);
    text_box.append(&summary);

    let size = gtk::Label::new(None);
    size.add_css_class("pm-row-meta");
    size.set_width_chars(9);
    size.set_xalign(1.0);
    size.set_valign(gtk::Align::Center);

    let date = gtk::Label::new(None);
    date.add_css_class("pm-row-meta");
    date.set_width_chars(10);
    date.set_xalign(1.0);
    date.set_valign(gtk::Align::Center);

    root.append(&icon);
    root.append(&text_box);
    root.append(&size);
    root.append(&date);

    RowWidgets { root }
}

fn child_at<T: IsA<gtk::Widget>>(root: &gtk::Box, index: i32) -> Option<T> {
    let widget = root.first_child()?;
    let mut current = Some(widget);
    let mut seen = 0;
    while let Some(node) = current {
        if seen == index {
            return node.downcast::<T>().ok();
        }
        seen += 1;
        current = node.next_sibling();
    }
    None
}

/// 把数据写进这一行。
pub fn bind_to(root: &gtk::Box, package: &Package) {
    let Some(icon) = child_at::<gtk::Image>(root, ICON) else {
        return;
    };
    let Some(text_box) = child_at::<gtk::Box>(root, TEXT_BOX) else {
        return;
    };
    let Some(size) = child_at::<gtk::Label>(root, SIZE) else {
        return;
    };
    let Some(date) = child_at::<gtk::Label>(root, DATE) else {
        return;
    };

    // text_box: [name_box, summary]
    let name_box = text_box
        .first_child()
        .and_then(|widget| widget.downcast::<gtk::Box>().ok());
    if let Some(name_box) = &name_box {
        if let (Some(name), Some(source), Some(protected)) = (
            child_at::<gtk::Label>(name_box, 0),
            child_at::<gtk::Label>(name_box, 1),
            child_at::<gtk::Label>(name_box, 2),
        ) {
            name.set_text(&package.display_name);
            source.set_text(package.source.label());
            protected.set_visible(package.protected);
        }
    }
    if let Some(summary) = text_box
        .last_child()
        .and_then(|widget| widget.downcast::<gtk::Label>().ok())
    {
        let text = if package.summary.is_empty() {
            package.name.clone()
        } else {
            format!("{} · {}", package.name, package.summary)
        };
        summary.set_text(&text);
    }

    size.set_text(&package.size_human());
    date.set_text(&package.installed_text());

    let icon_name = package.icon.clone().unwrap_or_else(|| {
        if package.is_desktop_app() || package.is_snap() {
            "application-x-executable".to_string()
        } else {
            "package-x-generic".to_string()
        }
    });
    if icon_name.starts_with('/') {
        icon.set_from_file(Some(icon_name));
    } else {
        icon.set_icon_name(Some(&icon_name));
    }
}
