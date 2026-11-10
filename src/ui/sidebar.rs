//! 左侧筛选栏：分组、来源、分类。

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

use crate::model::Source;

use super::Group;

/// 分组入口（常量表，闭包需要 'static）。
const GROUPS: [(&str, &str, Group); 4] = [
    ("全部程序", "view-grid-symbolic", Group::All),
    ("桌面应用", "window-new-symbolic", Group::Desktop),
    (
        "命令行与后台",
        "utilities-terminal-symbolic",
        Group::CommandLine,
    ),
    ("库与依赖", "package-x-generic-symbolic", Group::Library),
];

/// 来源入口。
const SOURCES: [(&str, &str, Source); 2] = [
    (
        "APT 系统包",
        "application-x-executable-symbolic",
        Source::Apt,
    ),
    ("Snap 应用", "package-x-generic-symbolic", Source::Snap),
];

/// 侧栏里可点击的一项。
struct Entry {
    row: gtk::ListBoxRow,
    apply: Box<dyn Fn(&mut super::FilterState)>,
}

pub struct Sidebar {
    pub root: gtk::Box,
    group_list: gtk::ListBox,
    source_list: gtk::ListBox,
    category_list: gtk::ListBox,
    categories_box: gtk::Box,
    filters: Rc<RefCell<super::FilterState>>,
    entries: Rc<RefCell<Vec<Entry>>>,
    leftover_row: RefCell<Option<gtk::ListBoxRow>>,
}

impl Sidebar {
    pub fn new(filters: Rc<RefCell<super::FilterState>>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("sidebar");
        root.set_width_request(212);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&section_title("查看"));

        let group_list = gtk::ListBox::new();
        group_list.set_selection_mode(gtk::SelectionMode::Single);
        group_list.add_css_class("navigation-sidebar");
        container.append(&group_list);

        container.append(&section_title("来源"));
        let source_list = gtk::ListBox::new();
        source_list.set_selection_mode(gtk::SelectionMode::Single);
        source_list.add_css_class("navigation-sidebar");
        container.append(&source_list);

        let categories_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        categories_box.append(&section_title("分类"));
        let category_list = gtk::ListBox::new();
        category_list.set_selection_mode(gtk::SelectionMode::Single);
        category_list.add_css_class("navigation-sidebar");
        categories_box.append(&category_list);
        container.append(&categories_box);

        scrolled.set_child(Some(&container));
        root.append(&scrolled);

        let sidebar = Self {
            root,
            group_list,
            source_list,
            category_list,
            categories_box,
            filters,
            entries: Rc::new(RefCell::new(Vec::new())),
            leftover_row: RefCell::new(None),
        };
        sidebar.build_entries();
        sidebar
    }

    fn build_entries(&self) {
        // 查看：全部 / 桌面应用 / 命令行 / 库与依赖
        for (label, icon, group) in GROUPS {
            let row = build_row(label, icon);
            self.group_list.append(&row);
            self.entries.borrow_mut().push(Entry {
                row,
                apply: Box::new(move |state| state.group = Some(group)),
            });
        }

        // 来源：APT / Snap
        for (label, icon, source) in SOURCES {
            let row = build_row(label, icon);
            self.source_list.append(&row);
            self.entries.borrow_mut().push(Entry {
                row,
                apply: Box::new(move |state| state.source = Some(source)),
            });
        }

        // 残留配置（默认隐藏，只有存在时才显示）
        let leftover = build_row("残留配置", "user-trash-symbolic");
        leftover.set_visible(false);
        self.group_list.append(&leftover);
        self.entries.borrow_mut().push(Entry {
            row: leftover.clone(),
            apply: Box::new(|state| {
                state.group = Some(Group::Leftover);
                state.source = None;
                state.category = None;
            }),
        });
        *self.leftover_row.borrow_mut() = Some(leftover);

        self.connect_selection(&self.group_list);
        self.connect_selection(&self.source_list);
        self.connect_selection(&self.category_list);
    }

    fn connect_selection(&self, list: &gtk::ListBox) {
        let filters = self.filters.clone();
        let entries = self.entries.clone();
        let all_lists = (
            self.group_list.clone(),
            self.source_list.clone(),
            self.category_list.clone(),
        );
        let this_list = list.clone();
        list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let index = row.index();
            if index < 0 {
                return;
            }
            let entry_index = entries.borrow().iter().position(|entry| entry.row == *row);
            let Some(entry_index) = entry_index else {
                return;
            };
            {
                let entry = &entries.borrow()[entry_index];
                (entry.apply)(&mut filters.borrow_mut());
            }
            // 只有一个列表保持选中：其它列表清空选择。
            let (groups, sources, categories) = &all_lists;
            for other in [groups, sources, categories] {
                if other != &this_list {
                    other.unselect_all();
                }
            }
            if let Some(ui) = super::ui() {
                super::refresh_filter(&ui);
            }
        });
    }

    /// 分类列表随扫描结果变化。
    pub fn set_categories(&self, categories: &[(String, usize)]) {
        self.category_list.remove_all();
        self.categories_box.set_visible(!categories.is_empty());
        let mut entries = self.entries.borrow_mut();
        entries
            .retain(|entry| entry.row.parent().as_ref() != Some(self.category_list.upcast_ref()));
        for (category, count) in categories {
            let row = build_row(&format!("{category} ({count})"), "tag-symbolic");
            self.category_list.append(&row);
            let value = category.clone();
            entries.push(Entry {
                row,
                apply: Box::new(move |state| {
                    state.category = Some(value.clone());
                    state.source = None;
                }),
            });
        }
    }

    pub fn set_leftover_count(&self, count: usize) {
        if let Some(row) = self.leftover_row.borrow().as_ref() {
            row.set_visible(count > 0);
            if let Some(child) = row.child().and_downcast::<gtk::Box>() {
                if let Some(label) = child
                    .last_child()
                    .and_then(|widget| widget.downcast::<gtk::Label>().ok())
                {
                    label.set_text(&format!("残留配置 ({count})"));
                }
            }
        }
    }

    /// 启动时默认选中「全部程序」。
    pub fn select_initial(&self) {
        if let Some(row) = self.group_list.row_at_index(0) {
            self.group_list.select_row(Some(&row));
        }
    }
}

fn section_title(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("pm-section-title");
    label.set_xalign(0.0);
    label
}

fn build_row(label: &str, icon: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(true);
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    box_.set_margin_start(4);
    box_.set_margin_end(4);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(16);
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    box_.append(&image);
    box_.append(&text);
    row.set_child(Some(&box_));
    row
}
