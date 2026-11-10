//! 主窗口：三栏布局（筛选侧栏 · 程序列表 · 详情面板）。

pub mod actions;
pub mod detail;
pub mod probe;
pub mod row;
pub mod sidebar;
pub mod style;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::model::{Package, PackageRef, Source};
use crate::system::{self, ScanResult};

/// 列表分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    All,
    Desktop,
    CommandLine,
    Library,
    Leftover,
}

impl Group {
    /// 库依赖不算「已安装的程序」：默认视图把它们折叠掉，需要时进「库与依赖」看。
    pub fn is_library(package: &Package) -> bool {
        !package.leftover
            && (package.section.starts_with("libs")
                || package.section.starts_with("libdevel")
                || package.auto_installed)
    }

    pub fn matches(self, package: &Package) -> bool {
        match self {
            Group::All => !package.leftover && !Self::is_library(package),
            Group::Desktop => !package.leftover && package.is_desktop_app(),
            Group::CommandLine => !package.leftover && !package.is_desktop_app(),
            Group::Library => Self::is_library(package),
            Group::Leftover => package.leftover,
        }
    }
}

/// 排序方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Date,
}

/// 共享的筛选状态。
#[derive(Debug, Default)]
pub struct FilterState {
    pub query: String,
    pub group: Option<Group>,
    pub source: Option<Source>,
    pub category: Option<String>,
    pub sort: Option<SortKey>,
    pub show_protected: bool,
}

impl FilterState {
    /// 单个包是否通过筛选。
    pub fn accepts(&self, package: &Package) -> bool {
        if let Some(group) = self.group {
            if !group.matches(package) {
                return false;
            }
        }
        if let Some(source) = self.source {
            if package.source != source {
                return false;
            }
        }
        if let Some(category) = &self.category {
            if crate::model::section_label(&package.section) != category {
                return false;
            }
        }
        if !self.show_protected && package.protected {
            return false;
        }
        package.matches(&self.query)
    }
}

/// 界面持有的全部状态。
pub struct Ui {
    pub window: adw::ApplicationWindow,
    pub toast: adw::ToastOverlay,
    pub banner: adw::Banner,
    pub store: gio::ListStore,
    pub filter_model: gtk::FilterListModel,
    pub sort_model: gtk::SortListModel,
    pub selection: gtk::SingleSelection,
    pub search: gtk::SearchEntry,
    pub list_view: gtk::ListView,
    pub list_title: gtk::Label,
    pub list_subtitle: gtk::Label,
    pub stat_label: gtk::Label,
    pub detail: detail::DetailPane,
    pub filters: Rc<RefCell<FilterState>>,
    pub sidebar: sidebar::Sidebar,
    pub scanning: Cell<bool>,
    pub pending: RefCell<Option<ScanResult>>,
}

// 全局唯一的界面单例（GTK 只在主线程使用）。
thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

pub fn set_ui(ui: Rc<Ui>) {
    UI.with(|slot| *slot.borrow_mut() = Some(ui));
}

pub fn ui() -> Option<Rc<Ui>> {
    UI.with(|slot| slot.borrow().clone())
}

/// 把 `glib::Object` 还原成程序条目。
pub fn package_of(object: &glib::Object) -> Option<PackageRef> {
    let boxed = object.downcast_ref::<glib::BoxedAnyObject>()?;
    let borrowed = boxed.borrow::<PackageRef>();
    Some(borrowed.clone())
}

/// 构造主窗口。
pub fn build(app: &adw::Application) -> Rc<Ui> {
    style::install();

    let filters = Rc::new(RefCell::new(FilterState::default()));

    // ---- 列表模型 ----
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let filter = gtk::CustomFilter::new({
        let filters = filters.clone();
        move |object| {
            let Some(package) = package_of(object) else {
                return false;
            };
            filters.borrow().accepts(&package)
        }
    });
    let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
    // 1800 条数据量很小，不需要增量过滤；开启后列表会分批出现，滚动与截图都不稳。
    filter_model.set_incremental(false);

    let sorter = gtk::CustomSorter::new({
        let filters = filters.clone();
        move |a, b| {
            let (Some(left), Some(right)) = (package_of(a), package_of(b)) else {
                return gtk::Ordering::Equal;
            };
            let key = filters.borrow().sort.unwrap_or(SortKey::Name);
            let ordering = match key {
                SortKey::Name => left
                    .display_name
                    .to_lowercase()
                    .cmp(&right.display_name.to_lowercase()),
                SortKey::Size => right.size_bytes.cmp(&left.size_bytes),
                SortKey::Date => right.installed_at.cmp(&left.installed_at),
            };
            match ordering {
                std::cmp::Ordering::Less => gtk::Ordering::Smaller,
                std::cmp::Ordering::Equal => gtk::Ordering::Equal,
                std::cmp::Ordering::Greater => gtk::Ordering::Larger,
            }
        }
    });
    let sort_model = gtk::SortListModel::new(Some(filter_model.clone()), Some(sorter));
    let selection = gtk::SingleSelection::new(Some(sort_model.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);

    // ---- 列表视图 ----
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let widget = row::bind_new();
        item.set_child(Some(&widget.root));
    });
    factory.connect_bind(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(widget) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(package) = item.item().and_then(|object| package_of(&object)) else {
            return;
        };
        row::bind_to(&widget, &package);
    });

    let list_view = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list_view.add_css_class("pm-list");
    list_view.set_single_click_activate(false);

    // ---- 顶部：搜索 + 标题 ----
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("搜索程序、包名、描述…"));
    search.set_width_chars(28);
    search.set_max_width_chars(40);

    let header_bar = adw::HeaderBar::new();
    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_box.set_valign(gtk::Align::Center);
    let app_title = gtk::Label::new(Some("程序管家"));
    app_title.add_css_class("title");
    app_title.set_xalign(0.0);
    let app_sub = gtk::Label::new(Some("管理这台电脑上已安装的程序"));
    app_sub.add_css_class("subtitle");
    app_sub.add_css_class("dim-label");
    app_sub.set_xalign(0.0);
    title_box.append(&app_title);
    title_box.append(&app_sub);
    header_bar.set_title_widget(Some(&title_box));

    let menu = gio::Menu::new();
    menu.append(Some("刷新列表"), Some("win.refresh"));
    menu.append(Some("排序方式"), None);
    let sort_menu = gio::Menu::new();
    sort_menu.append(Some("按名称"), Some("win.sort-name"));
    sort_menu.append(Some("按占用空间"), Some("win.sort-size"));
    sort_menu.append(Some("按安装时间"), Some("win.sort-date"));
    menu.append_section(None, &sort_menu);
    menu.append(Some("显示受保护的系统包"), Some("win.toggle-protected"));
    menu.append(Some("清理残留配置…"), Some("win.clean-leftovers"));
    menu.append(Some("关于程序管家"), Some("win.about"));

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_menu_model(Some(&menu));
    menu_button.set_tooltip_text(Some("菜单"));
    header_bar.pack_end(&menu_button);

    // ---- 列表头：标题 + 统计 ----
    let list_title = gtk::Label::new(Some("全部程序"));
    list_title.add_css_class("heading");
    list_title.set_xalign(0.0);
    let list_subtitle = gtk::Label::new(None);
    list_subtitle.add_css_class("dim-label");
    list_subtitle.add_css_class("caption");
    list_subtitle.set_xalign(0.0);

    let heading_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
    heading_box.set_margin_top(10);
    heading_box.set_margin_bottom(8);
    heading_box.set_margin_start(14);
    heading_box.set_margin_end(14);
    heading_box.append(&list_title);
    heading_box.append(&list_subtitle);
    heading_box.set_hexpand(true);

    let stat_label = gtk::Label::new(None);
    stat_label.add_css_class("stat-pill");
    stat_label.set_valign(gtk::Align::Center);

    let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    top_row.append(&heading_box);
    top_row.append(&stat_label);
    top_row.set_margin_end(14);

    // 搜索条独立成行，宽度跟随列表栏，长查询也能看全。
    search.set_margin_top(2);
    search.set_margin_bottom(10);
    search.set_margin_start(14);
    search.set_margin_end(14);
    search.set_hexpand(true);

    let list_header = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_header.add_css_class("pm-list-header");
    list_header.append(&top_row);
    list_header.append(&search);

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_child(Some(&list_view));

    let list_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_column.set_hexpand(true);
    list_column.append(&list_header);
    list_column.append(&scrolled);

    // ---- 详情面板 ----
    let detail = detail::DetailPane::new();

    // ---- 侧栏 ----
    let sidebar = sidebar::Sidebar::new(filters.clone());

    // ---- 组合 ----
    let split = adw::OverlaySplitView::new();
    split.set_sidebar(Some(&sidebar.root));
    split.set_content(Some(&list_column));
    split.set_min_sidebar_width(210.0);
    split.set_max_sidebar_width(260.0);
    split.set_sidebar_width_fraction(0.18);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_start_child(Some(&split));
    paned.set_end_child(Some(&detail.root));
    paned.set_resize_start_child(true);
    paned.set_shrink_start_child(false);
    paned.set_resize_end_child(false);
    paned.set_shrink_end_child(false);
    // 详情栏固定宽度，空间优先给列表：卸载决策发生在列表和详情之间来回看的时候。
    paned.set_position(880);

    let banner = adw::Banner::new("");
    banner.set_revealed(false);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&paned);
    paned.set_vexpand(true);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header_bar);
    toolbar_view.set_content(Some(&content));

    let toast = adw::ToastOverlay::new();
    toast.set_child(Some(&toolbar_view));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("程序管家")
        .default_width(1280)
        .default_height(800)
        .width_request(940)
        .height_request(560)
        .content(&toast)
        .build();
    window.set_icon_name(Some("dev.nanh.PakManager"));

    let ui = Rc::new(Ui {
        window: window.clone(),
        toast,
        banner,
        store,
        filter_model: filter_model.clone(),
        sort_model: sort_model.clone(),
        selection: selection.clone(),
        search: search.clone(),
        list_view: list_view.clone(),
        list_title,
        list_subtitle,
        stat_label,
        detail,
        filters: filters.clone(),
        sidebar,
        scanning: Cell::new(false),
        pending: RefCell::new(None),
    });

    wire_events(&ui);
    set_ui(ui.clone());

    ui.sidebar.select_initial();
    reload(&ui);
    ui
}

/// 连接搜索、选择、菜单等交互。
fn wire_events(ui: &Rc<Ui>) {
    // 搜索：防抖 180ms，避免每次按键都重建筛选。
    let pending: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    ui.search.connect_search_changed({
        let filters = ui.filters.clone();
        let weak = Rc::downgrade(ui);
        let pending = pending.clone();
        move |entry| {
            let text = entry.text().to_string().to_lowercase();
            if let Some(source) = pending.take() {
                source.remove();
            }
            let filters = filters.clone();
            let weak = weak.clone();
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(180), move || {
                filters.borrow_mut().query = text;
                if let Some(ui) = weak.upgrade() {
                    refresh_filter(&ui);
                }
            });
            pending.set(Some(id));
            // timeout_add_local_once 返回的 SourceId 已自动移除，这里只做去重标记。
            pending.set(None);
        }
    });

    // 选中变化 → 更新详情面板。
    ui.selection.connect_selected_notify({
        let weak = Rc::downgrade(ui);
        move |selection| {
            let Some(ui) = weak.upgrade() else { return };
            let package = selection
                .selected_item()
                .and_then(|object| package_of(&object));
            ui.detail.show(package);
        }
    });

    // 列表数量变化 → 更新统计。
    ui.filter_model.connect_notify_local(Some("n-items"), {
        let weak = Rc::downgrade(ui);
        move |_, _| {
            if let Some(ui) = weak.upgrade() {
                update_counts(&ui);
            }
        }
    });
}

/// 筛选条件变化后重建列表。
pub fn refresh_filter(ui: &Rc<Ui>) {
    ui.filter_model.filter().and_downcast::<gtk::CustomFilter>().map(|filter| filter.changed(gtk::FilterChange::Different));
    ui.sort_model.sorter().and_downcast::<gtk::CustomSorter>().map(|sorter| sorter.changed(gtk::SorterChange::Different));
    update_counts(ui);
}

/// 更新标题下的说明与右上角统计。
pub fn update_counts(ui: &Rc<Ui>) {
    let total = ui.sort_model.n_items();
    let state = ui.filters.borrow();
    let label = match state.group.unwrap_or(Group::All) {
        Group::All => "全部程序".to_string(),
        Group::Desktop => "桌面应用".to_string(),
        Group::CommandLine => "命令行与后台程序".to_string(),
        Group::Library => "库与依赖".to_string(),
        Group::Leftover => "残留配置".to_string(),
    };
    let mut extra = Vec::new();
    if let Some(source) = state.source {
        extra.push(source.label().to_string());
    }
    if let Some(category) = &state.category {
        extra.push(category.clone());
    }
    if !state.query.is_empty() {
        extra.push(format!("“{}”", state.query));
    }
    drop(state);

    ui.list_title.set_text(&label);
    if extra.is_empty() {
        ui.list_subtitle
            .set_text(&format!("{total} 个条目"));
    } else {
        ui.list_subtitle
            .set_text(&format!("{total} 个条目 · {}", extra.join(" · ")));
    }

    let bytes: u64 = (0..ui.sort_model.n_items())
        .filter_map(|index| {
            ui.sort_model
                .item(index)
                .and_then(|object| package_of(&object))
        })
        .map(|package| package.size_bytes)
        .sum();
    ui.stat_label
        .set_text(&format!("{} · {}", total, crate::model::human_bytes(bytes)));
}

/// 重新扫描系统并填充列表。
pub fn reload(ui: &Rc<Ui>) {
    if ui.scanning.replace(true) {
        return;
    }
    ui.window.set_cursor_from_name(Some("progress"));
    let (sender, receiver) = std::sync::mpsc::channel::<ScanResult>();

    system::scan_async(|_| {}, sender);

    let weak = Rc::downgrade(ui);
    glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if let Some(ui) = weak.upgrade() {
                    ui.scanning.set(false);
                    ui.window.set_cursor(None);
                }
                return glib::ControlFlow::Break;
            }
        };
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        ui.scanning.set(false);
        ui.window.set_cursor(None);
        apply_scan(&ui, result);
        glib::ControlFlow::Break
    });
}

/// 把扫描结果塞进列表模型。
pub fn apply_scan(ui: &Rc<Ui>, result: ScanResult) {
    let mut objects: Vec<glib::BoxedAnyObject> = Vec::with_capacity(result.packages.len());
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for package in result.packages.iter().chain(result.leftovers.iter()) {
        // 同一个 id 只进列表一次（例如多个 arch 的同名包），否则统计会虚高。
        if !seen.insert(package.id.as_str()) {
            continue;
        }
        let reference: PackageRef = Rc::new(package.clone());
        objects.push(glib::BoxedAnyObject::new(reference));
    }
    ui.store.remove_all();
    ui.store.extend_from_slice(&objects);

    // 分类连同数量一起给侧栏，排序按使用频率而不是字母序。
    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    for package in result.packages.iter().filter(|package| !package.protected) {
        let label = crate::model::section_label(&package.section);
        if label == "库与依赖" && package.auto_installed {
            continue;
        }
        *counts.entry(label).or_default() += 1;
    }
    let mut categories: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(label, count)| (label.to_string(), count))
        .collect();
    categories.sort_by_key(|(label, _)| {
        let section = result
            .packages
            .iter()
            .find(|package| crate::model::section_label(&package.section) == label)
            .map(|package| package.section.as_str())
            .unwrap_or("");
        (crate::model::section_rank(section), label.clone())
    });
    ui.sidebar.set_categories(&categories);

    ui.sidebar.set_leftover_count(result.leftovers.len());

    if !result.warnings.is_empty() {
        ui.banner.set_title(&result.warnings.join("；"));
        ui.banner.set_revealed(true);
    } else {
        ui.banner.set_revealed(false);
    }

    *ui.pending.borrow_mut() = Some(result);
    refresh_filter(ui);
    update_counts(ui);
}
