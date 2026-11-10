//! 数据模型：统一的已安装程序条目。

use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Source {
    #[default]
    Apt,
    Snap,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Apt => "APT",
            Source::Snap => "Snap",
        }
    }
}

/// 一个已安装的程序（APT 包或 Snap 应用）。
#[derive(Debug, Clone, Default)]
pub struct Package {
    /// 唯一 id：`apt:bash` / `snap:firefox`
    pub id: String,
    /// 包名 / snap 名
    pub name: String,
    /// 桌面显示名（来自 .desktop 的 Name），没有则为包名
    pub display_name: String,
    pub version: String,
    pub source: Source,
    /// 磁盘占用（字节）
    pub size_bytes: u64,
    /// 安装日期（unix 时间戳），未知为 None
    pub installed_at: Option<i64>,
    pub summary: String,
    pub description: String,
    /// 分类（APT 的 section / snap 的类型）
    pub section: String,
    /// 被 apt 标记为自动安装（通常是依赖）
    pub auto_installed: bool,
    /// Essential / required：卸载会破坏系统
    pub essential: bool,
    /// 受保护，界面上禁止卸载
    pub protected: bool,
    /// 已卸载但残留配置（dpkg 状态 rc）
    pub leftover: bool,
    pub maintainer: String,
    /// 图标名或图标文件路径
    pub icon: Option<String>,
    /// .desktop 文件路径（可用于启动）
    pub desktop_file: Option<String>,
    /// 发布者（snap）
    pub publisher: String,
    /// snap 的 confinement（strict / classic）
    pub confinement: String,
    /// 搜索用的小写关键词
    pub keywords: String,
}

impl Package {
    pub fn is_snap(&self) -> bool {
        self.source == Source::Snap
    }

    /// 是否有桌面入口（能双击启动）。
    pub fn is_desktop_app(&self) -> bool {
        self.desktop_file.is_some()
    }

    /// 搜索匹配：所有词都要命中。
    pub fn matches(&self, needle: &str) -> bool {
        needle
            .split_whitespace()
            .all(|word| self.keywords.contains(word))
    }

    pub fn size_human(&self) -> String {
        human_bytes(self.size_bytes)
    }

    /// 安装时间的本地日期文本。
    pub fn installed_text(&self) -> String {
        match self.installed_at {
            Some(ts) => crate::util::format_date(ts),
            None => "—".to_string(),
        }
    }
}

pub type PackageRef = Rc<Package>;

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// APT section → 中文分类名。
///
/// dpkg 里 40 多个 section 粒度太细（libs 一项就 863 个包），这里收敛成
/// 用户真正会用来筛选的十几个分类。
pub fn section_label(section: &str) -> &'static str {
    match section.split('/').next().unwrap_or(section) {
        // 库和语言绑定占掉七成条目，单独归成一类，默认视图会折叠它们。
        "libs" | "libdevel" | "oldlibs" | "introspection" | "debug" => "库与依赖",
        "perl" | "python" | "ruby" | "rust" | "javascript" | "lisp" | "interpreters" => {
            "语言运行时"
        }
        "devel" | "vcs" => "开发工具",
        "non-free" | "contrib" | "non-free-firmware" | "restricted" | "multiverse" => "第三方软件",
        "localization" | "translations" => "语言包",
        "kde" | "xfce" | "gnome" | "x11" => "桌面环境",
        "admin" | "base" | "kernel" => "系统管理",
        "comm" | "mail" | "news" => "通讯",
        "utils" | "misc" | "otherosfs" => "工具",
        "doc" | "editors" | "text" => "文档与文本",
        "graphics" => "图形图像",
        "video" => "视频",
        "sound" => "音频",
        "net" | "web" | "httpd" => "网络",
        "fonts" => "字体",
        "games" => "游戏",
        "education" | "science" | "math" | "tex" => "教育科研",
        "shells" => "命令行",
        "metapackages" => "元包",
        _ => "其他",
    }
}

/// 分类在侧栏里的排列顺序：常用的在前，库和杂项垫底。
pub fn section_rank(section: &str) -> u8 {
    match section_label(section) {
        "图形图像" => 1,
        "视频" => 2,
        "音频" => 3,
        "网络" => 4,
        "通讯" => 5,
        "开发工具" | "语言运行时" => 6,
        "工具" => 7,
        "系统管理" => 8,
        "文档与文本" => 9,
        "字体" => 10,
        "游戏" | "教育科研" => 11,
        "桌面环境" => 12,
        "第三方软件" | "语言包" | "命令行" => 13,
        "库与依赖" => 90,
        "元包" => 91,
        _ => 50,
    }
}
