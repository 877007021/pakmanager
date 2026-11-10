//! 视觉层：设计令牌与样式表。
//!
//! 设计系统来自 ui-ux-pro-max 的 `--design-system` 结果（扁平风格 / 青绿主色 /
//! 密集表格密度 8 / 克制动效），并映射到 libadwaita 的命名色上，
//! 这样跟随系统明暗主题、也保留可访问的对比度。

use gtk::gdk;

/// 设计令牌（与 ui-ux-pro-max 生成的 MASTER.md 对齐）。
///
/// 之所以把颜色写死在样式表里而不是引用 `@accent_bg_color`：
/// 这台机器的系统强调色是橙色，直接继承会与青绿主色冲突。
pub mod tokens {
    /// 主色：青绿 600，用于选中态、主按钮、聚焦描边
    pub const PRIMARY: &str = "#0d9488";
    /// 主色按下/悬停
    pub const PRIMARY_DARK: &str = "#0f766e";
    /// 危险操作（卸载）
    pub const DESTRUCTIVE: &str = "#dc2626";
    /// 紧凑密度（8px 网格），列表行 44px
    pub const DENSITY: u8 = 8;
}

/// 安装一次全局样式。
pub fn install() {
    let provider = gtk::CssProvider::new();
    // 颜色集中写在 tokens 里，运行时替换进样式表，避免两处维护。
    let sheet = STYLESHEET
        .replace("@ROW_HEIGHT@", &(tokens::DENSITY as i32 * 5 + 4).to_string())
        .replace("@PRIMARY_DARK@", tokens::PRIMARY_DARK)
        .replace("@PRIMARY@", tokens::PRIMARY)
        .replace("@DESTRUCTIVE@", tokens::DESTRUCTIVE);
    provider.load_from_string(&sheet);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// 全局样式表。刻意保持扁平：没有阴影、没有渐变，只用间距和层级区分信息。
const STYLESHEET: &str = r#"
/* 主按钮用青绿 */
button.suggested-action {
  background-color: @PRIMARY@;
  color: #ffffff;
  border-radius: 8px;
}
button.suggested-action:hover {
  background-color: @PRIMARY_DARK@;
}
button.suggested-action:active {
  background-color: @PRIMARY_DARK@;
}

/* 破坏性操作：橙红，但默认是扁平描边，降低误触 */
button.destructive-action {
  background-color: @DESTRUCTIVE@;
  color: #ffffff;
  border-radius: 8px;
}
button.destructive-action:hover {
  background-color: @DESTRUCTIVE@;
}

/* ---------- 侧栏：来源与分类 ---------- */
.sidebar {
  background-color: alpha(@PRIMARY@, 0.05);
}
.sidebar row {
  border-radius: 8px;
  min-height: 34px;
  padding: 0 8px;
  margin: 1px 6px;
}
.sidebar row:selected {
  background-color: @PRIMARY@;
  color: #ffffff;
}

.pm-section-title {
  font-size: 0.8em;
  font-weight: 700;
  letter-spacing: 0.04em;
  opacity: 0.55;
  margin: 14px 14px 4px 14px;
}

.rows-group {
  margin: 2px 8px 10px 8px;
  border-radius: 10px;
}
.rows-group row {
  min-height: 40px;
  padding: 2px 12px;
}

/* ---------- 中间列表 ---------- */
.pm-list-header {
  border-bottom: 1px solid alpha(currentColor, 0.10);
}
.pm-row {
  min-height: @ROW_HEIGHT@px;
  padding: 0 12px;
  border-bottom: 1px solid alpha(currentColor, 0.06);
}
.pm-row:hover {
  background-color: alpha(currentColor, 0.04);
}
.pm-row:selected {
  background-color: alpha(@PRIMARY@, 0.14);
}

/* GtkListView 的选中态挂在生成的 row 节点上，不是我们自己的 .pm-row 上 */
.pm-list > row:selected,
.pm-list > row:selected .pm-row {
  background-color: alpha(@PRIMARY@, 0.16);
}

.pm-list > row {
  border-radius: 8px;
  margin: 0 6px;
}
.pm-row-name {
  font-weight: 600;
}
.pm-row-sub {
  font-size: 0.85em;
  opacity: 0.62;
}
.pm-row-meta {
  font-size: 0.85em;
  opacity: 0.72;
  font-variant-numeric: tabular-nums;
}
.pm-source-badge {
  font-size: 0.72em;
  font-weight: 700;
  border-radius: 4px;
  padding: 1px 5px;
  background-color: alpha(currentColor, 0.08);
  opacity: 0.85;
}
.pm-protected-badge {
  font-size: 0.72em;
  font-weight: 700;
  border-radius: 4px;
  padding: 1px 5px;
  background-color: alpha(@warning_bg_color, 0.85);
  color: @warning_fg_color;
}

/* ---------- 详情面板 ---------- */
.detail-pane {
  background-color: @card_bg_color;
}
.detail-title {
  font-size: 1.35em;
  font-weight: 800;
}
.detail-sub {
  opacity: 0.6;
  font-size: 0.92em;
}
.detail-key {
  opacity: 0.55;
  font-size: 0.86em;
}
.detail-value {
  font-size: 0.92em;
}
.detail-desc {
  font-size: 0.95em;
  line-height: 1.5;
}

/* 统计条：一行小字，等宽数字，不抢注意力 */
/* 搜索框：聚焦时用主色描边，和系统橙色强调区分开 */
entry.search:focus-within,
entry.search:focus {
  outline-color: alpha(@PRIMARY@, 0.75);
  border-color: @PRIMARY@;
  box-shadow: inset 0 0 0 1px alpha(@PRIMARY@, 0.65);
}

.stat-pill {
  font-size: 0.82em;
  padding: 2px 10px;
  border-radius: 999px;
  background-color: alpha(currentColor, 0.06);
  font-variant-numeric: tabular-nums;
}

.pm-empty-title {
  font-size: 1.1em;
  font-weight: 700;
}
.pm-empty-sub {
  opacity: 0.6;
}

.log-view {
  font-family: monospace;
  font-size: 0.85em;
}
"#;
