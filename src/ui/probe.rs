//! 自动化探针：用于开发期验证界面。
//!
//! 设置环境变量 `PAKMANAGER_SHOT=<目录>` 时，程序会在主窗口渲染完成后
//! 依次把若干界面状态存成 PNG，然后退出。它不会改动系统上任何东西，
//! 只做「截图 + 读一次筛选结果」这类只读操作。

use std::path::PathBuf;

use adw::prelude::*;
use gtk::gsk::prelude::GskRendererExt;

use super::Ui;

/// 是否启用了探针模式。
pub fn enabled() -> bool {
    std::env::var_os("PAKMANAGER_SHOT").is_some()
}

fn output_dir() -> PathBuf {
    std::env::var_os("PAKMANAGER_SHOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// 把当前窗口渲染成 PNG。
pub fn capture(widget: &impl IsA<gtk::Widget>, name: &str) -> Result<PathBuf, String> {
    let width = widget.width() as f64;
    let height = widget.height() as f64;
    if width < 1.0 || height < 1.0 {
        return Err(format!("窗口尺寸无效：{width}x{height}"));
    }
    if !widget.is_mapped() {
        return Err("窗口还没有映射到屏幕".to_string());
    }
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width, height);
    // 子部件是异步布局的：第一帧常常还是空的，多试几次。
    let node = match snapshot.to_node() {
        Some(node) if node.bounds().width() > 1.0 => node,
        _ => {
            let retry = gtk::Snapshot::new();
            paintable.snapshot(&retry, width, height);
            retry.to_node().ok_or("没有生成渲染节点")?
        }
    };

    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize_for_display(&widget.display())
        .map_err(|err| format!("渲染器初始化失败：{err}"))?;
    let texture = renderer.render_texture(&node, None);
    let path = output_dir().join(format!("{name}.png"));
    let ok = texture.save_to_png(&path);
    renderer.unrealize();
    if ok.is_ok() {
        Ok(path)
    } else {
        Err(format!("写入 {} 失败", path.display()))
    }
}

/// 依次抓取几个界面状态。
///
/// 每次「改状态」和「截图」分在两个主循环节拍里完成：行视图是异步惰性布局的，
/// 同一帧里改完模型立刻 snapshot 会拿到空节点。
pub fn run(ui: &std::rc::Rc<Ui>) {
    let ui = ui.clone();
    // 截图统一用浅色主题，便于对比设计稿；正常运行时跟随系统设置。
    if let Some(manager) = adw::StyleManager::default()
        .downcast::<adw::StyleManager>()
        .ok()
    {
        manager.set_color_scheme(adw::ColorScheme::ForceLight);
    }
    let mut phase = 0usize;
    let mut pending_action: Option<Box<dyn Fn(&Ui)>> = Some(Box::new(|ui| {
        ui.selection.set_selected(0);
    }));
    let mut report: Vec<String> = Vec::new();
    let mut action_result: Option<String> = None;

    let names = [
        "shot-1-overview",
        "shot-2-detail",
        "shot-3-search",
        "shot-4-snap",
        "shot-5-sizes",
    ];

    gtk::glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
        let ui = &ui;
        if let Some(action) = pending_action.take() {
            action(ui);
            action_result = Some(format!("n={}", ui.sort_model.n_items()));
            return gtk::glib::ControlFlow::Continue;
        }

        // 截图节拍
        if phase >= names.len() {
            report.push(detail_dump(ui));
            report.push(metrics(ui));
            report.push(geometry(ui));
            println!("PROBE {}", report.join(" | "));
            if let Some(app) = gtk::gio::Application::default() {
                app.quit();
            }
            return gtk::glib::ControlFlow::Break;
        }
        if phase == 0 {
            report.push(dump_rows(ui));
        }
        let name = names[phase];
        match capture(&ui.window, name) {
            Ok(_) => report.push(format!("{name} ok")),
            Err(err) => report.push(format!("{name} ERR {err}")),
        }
        if let Some(info) = action_result.take() {
            report.push(info);
        }
        report.push(format!("selected={:?}", ui.detail.current_name()));
        phase += 1;

        pending_action = match phase {
            1 => {
                ui.filters.borrow_mut().query = "firefox".to_string();
                super::refresh_filter(ui);
                Some(Box::new(|_| {}))
            }
            2 => {
                ui.filters.borrow_mut().query.clear();
                ui.filters.borrow_mut().source = Some(crate::model::Source::Snap);
                super::refresh_filter(ui);
                Some(Box::new(|ui| ui.selection.set_selected(0)))
            }
            3 => {
                ui.filters.borrow_mut().source = None;
                ui.filters.borrow_mut().sort = Some(super::SortKey::Size);
                ui.filters.borrow_mut().group = Some(super::Group::Desktop);
                super::refresh_filter(ui);
                Some(Box::new(|ui| ui.selection.set_selected(0)))
            }
            4 => Some(Box::new(|ui| ui.selection.set_selected(0))),
            _ => None,
        };
        gtk::glib::ControlFlow::Continue
    });
}

/// 读出整个窗口里所有非空标签，用来确认绑定是否生效。
fn dump_rows(ui: &Ui) -> String {
    let mut texts = Vec::new();
    collect_labels(ui.window.upcast_ref::<gtk::Widget>(), &mut texts, 0);
    let total = texts.len();
    let sample: Vec<String> = texts.iter().take(24).cloned().collect();
    format!("labels({total}): {}", sample.join(" / "))
}

fn collect_labels(widget: &gtk::Widget, out: &mut Vec<String>, depth: usize) {
    if depth > 30 || out.len() > 200 {
        return;
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        let text = label.text().to_string();
        // 只统计真正显示出来的文本，隐藏的徽标不算。
        if !text.trim().is_empty() && label.is_visible() && label.is_mapped() {
            out.push(text);
        }
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        child = node.next_sibling();
        collect_labels(&node, out, depth + 1);
    }
}

/// 读出详情面板里的键值文本，确认信息完整。
fn detail_dump(ui: &Ui) -> String {
    let mut texts = Vec::new();
    collect_labels(ui.detail.body().upcast_ref::<gtk::Widget>(), &mut texts, 0);
    format!("detail: {}", texts.join(" | "))
}

/// 布局度量：用来核对行高、栏宽这些数值是否符合设计密度。
pub fn metrics(ui: &Ui) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("window={}x{}", ui.window.width(), ui.window.height()));
    parts.push(format!("rows={}", ui.sort_model.n_items()));
    let mut child = ui.list_view.first_child();
    let mut rows = 0;
    while let Some(node) = child {
        child = node.next_sibling();
        if node.height() > 10 {
            rows += 1;
            if rows == 1 {
                parts.push(format!("first_row={}x{}", node.width(), node.height()));
            }
        }
    }
    parts.push(format!("visible_rows={rows}"));
    parts.push(format!("detail_w={}", ui.detail.root.width()));
    parts.push(format!("sidebar_w={}", ui.sidebar.root.width()));
    parts.push(format!("selected_index={}", ui.selection.selected()));
    parts.join(" ")
}

/// 打印关键控件的实际分配区域，用来核对三栏布局。
pub fn geometry(ui: &Ui) -> String {
    fn rect(widget: &gtk::Widget) -> String {
        #[allow(deprecated)]
        let alloc = widget.allocation();
        format!("{}x{}+{}+{}", alloc.width(), alloc.height(), alloc.x(), alloc.y())
    }
    let mut parts = vec![
        format!("window={}", rect(ui.window.upcast_ref::<gtk::Widget>())),
        format!("sidebar={}", rect(ui.sidebar.root.upcast_ref::<gtk::Widget>())),
        format!("list_view={}", rect(ui.list_view.upcast_ref::<gtk::Widget>())),
        format!("detail={}", rect(ui.detail.root.upcast_ref::<gtk::Widget>())),
        format!("search={}", rect(ui.search.upcast_ref::<gtk::Widget>())),
    ];
    // 列表里第一行的实际位置
    let mut child = ui.list_view.first_child();
    let mut index = 0;
    while let Some(node) = child {
        child = node.next_sibling();
        if node.height() > 10 {
            parts.push(format!("row{index}={}", rect(&node)));
            index += 1;
            if index >= 2 {
                break;
            }
        }
    }
    parts.join(" ")
}
