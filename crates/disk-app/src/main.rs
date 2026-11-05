#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cleanup_view;
mod logging;
mod treemap_view;

use std::sync::Arc;

use app::DiskApp;
use eframe::egui;

fn main() {
    logging::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--scan" {
        #[cfg(windows)]
        attach_parent_console();
        log_start("headless");
        run_headless(&args[2]);
        return;
    }
    let initial = args.get(1).filter(|arg| !arg.starts_with("--")).cloned();
    log_start("gui");
    if let Err(error) = run_gui(initial) {
        logging::event(
            logging::Level::Error,
            "startup_error",
            &[("error", logging::Value::Str(&error.to_string()))],
        );
        eprintln!("启动失败：{error}");
        std::process::exit(1);
    }
}

fn log_start(mode: &str) {
    logging::event(
        logging::Level::Info,
        "app_start",
        &[
            ("mode", logging::Value::Str(mode)),
            ("version", logging::Value::Str(env!("CARGO_PKG_VERSION"))),
        ],
    );
}

/// 发布版是 GUI 子系统，默认没有控制台，`println!` 会无处可写。
/// 命令行执行 `--scan` 时先附着到父进程控制台，结果才能显示在同一个窗口里。
#[cfg(windows)]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// 无界面扫描，便于在没有图形环境时验证核心流程。
fn run_headless(path: &str) {
    logging::event(
        logging::Level::Info,
        "scan_start",
        &[("root", logging::Value::Str(path))],
    );
    let progress = Arc::new(disk_core::ScanProgress::new());
    match disk_core::scan(std::path::Path::new(path), progress) {
        Ok(tree) => {
            logging::event(
                logging::Level::Info,
                "scan_done",
                &[
                    ("root", logging::Value::Str(&tree.root_path)),
                    ("files", logging::Value::Num(tree.total_files())),
                    ("dirs", logging::Value::Num(tree.dir_count())),
                    ("bytes", logging::Value::Num(tree.total_size())),
                    ("skipped_links", logging::Value::Num(tree.skipped_links)),
                    ("read_errors", logging::Value::Num(tree.read_error_count)),
                    (
                        "elapsed_ms",
                        logging::Value::Num(tree.elapsed.as_millis() as u64),
                    ),
                ],
            );
            println!("根目录: {}", tree.root_path);
            println!("总大小: {}", disk_core::format_size(tree.total_size()));
            println!("文件数: {}", tree.total_files());
            println!("文件夹数: {}", tree.dir_count());
            println!("跳过链接: {}", tree.skipped_links);
            println!("读取失败: {}", tree.read_error_count);
            println!("用时: {:.2}s", tree.elapsed.as_secs_f64());
            for id in tree.top_files(0, 10) {
                let node = tree.node(id);
                println!(
                    "  {:>10}  {}",
                    disk_core::format_size(node.size),
                    tree.path_of(id)
                );
            }
        }
        Err(error) => {
            logging::event(
                logging::Level::Error,
                "scan_error",
                &[
                    ("root", logging::Value::Str(path)),
                    ("error", logging::Value::Str(&error.to_string())),
                ],
            );
            eprintln!("扫描失败：{error}");
            std::process::exit(1);
        }
    }
}

fn run_gui(initial: Option<String>) -> eframe::Result<()> {
    let (width, height) = default_window_size();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width, height])
            .with_min_inner_size([860.0, 560.0])
            .with_title("磁盘空间分析器"),
        ..Default::default()
    };
    eframe::run_native(
        "disk-cleaner",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            setup_style(&cc.egui_ctx);
            Ok(Box::new(DiskApp::new(initial)))
        }),
    )
}

/// 按主屏幕尺寸给出合适的初始窗口大小，避免在小屏上超出可视区域。
fn default_window_size() -> (f32, f32) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetSystemMetrics(index: i32) -> i32;
        }
        let screen_width = unsafe { GetSystemMetrics(0) };
        let screen_height = unsafe { GetSystemMetrics(1) };
        if screen_width > 0 && screen_height > 0 {
            return (
                (screen_width as f32 * 0.9).clamp(860.0, 1280.0),
                (screen_height as f32 * 0.8).clamp(560.0, 820.0),
            );
        }
    }
    (1280.0, 820.0)
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for candidate in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(candidate) {
            fonts.font_data.insert(
                "cjk".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "cjk".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("cjk".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = app::BG_PANEL;
    visuals.window_fill = app::BG_CARD;
    visuals.extreme_bg_color = app::BG_WINDOW;
    visuals.override_text_color = Some(app::TEXT);
    visuals.selection.bg_fill = app::SELECTION_BG;
    // 不跟随系统主题：系统若是浅色，egui 会切到浅色主题并丢弃自定义配色，
    // 导致浅底 + 浅色文字几乎看不见。两个主题都钉死为同一套深色视觉。
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, visuals.clone());
    ctx.set_visuals_of(egui::Theme::Light, visuals);

    ctx.all_styles_mut(|style| {
        // hover / selected 等微交互统一 150ms，由 ctx.animate_bool 使用。
        style.animation_time = app::ANIM_FAST;
        // 8pt 间距节奏：只出现 4 / 8 / 12 / 16 / 24。
        style.spacing.item_spacing = egui::vec2(app::SPACE_SM, app::SPACE_XS);
        style.spacing.button_padding = egui::vec2(app::SPACE_MD, app::SPACE_XS);
        style.spacing.menu_margin = egui::Margin::same(app::SPACE_MD as i8);
        style.spacing.window_margin = egui::Margin::same(app::SPACE_MD as i8);
        // 3 档字号：辅助 11 / 正文 13 / 标题 20。
        style.text_styles = [
            (
                egui::TextStyle::Small,
                egui::FontId::proportional(app::FONT_SMALL),
            ),
            (
                egui::TextStyle::Body,
                egui::FontId::proportional(app::FONT_BODY),
            ),
            (
                egui::TextStyle::Button,
                egui::FontId::proportional(app::FONT_BODY),
            ),
            (
                egui::TextStyle::Heading,
                egui::FontId::proportional(app::FONT_TITLE),
            ),
            (
                egui::TextStyle::Monospace,
                egui::FontId::monospace(app::FONT_BODY),
            ),
        ]
        .into();
    });
}
