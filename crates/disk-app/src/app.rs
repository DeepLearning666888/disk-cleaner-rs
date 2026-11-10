//! 主界面：顶部目录栏、中央树图、右侧详情/清单、底部状态。

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use disk_core::{format_size, NodeKind, ScanError, ScanProgress, Tree, NO_PARENT};
use eframe::egui::{self, Color32, RichText};

use crate::cleanup_view::CleanupPanel;
use crate::logging::{self, Level, Value};
use crate::treemap_view;

// 设计令牌：背景自深到浅分三层（窗口 → 面板 → 卡片），前景分正文与次要两层。
pub(crate) const BG_WINDOW: Color32 = Color32::from_rgb(14, 16, 20);
pub(crate) const BG_PANEL: Color32 = Color32::from_rgb(21, 24, 29);
pub(crate) const BG_CARD: Color32 = Color32::from_rgb(30, 34, 42);

pub(crate) const TEXT: Color32 = Color32::from_rgb(228, 232, 238);
pub(crate) const MUTED: Color32 = Color32::from_rgb(146, 154, 166);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(64, 132, 255);
pub(crate) const SELECTION_BG: Color32 = Color32::from_rgb(45, 82, 140);
pub(crate) const WARN: Color32 = Color32::from_rgb(232, 178, 92);
pub(crate) const DANGER: Color32 = Color32::from_rgb(186, 66, 66);

// 8pt 间距节奏：只使用这五档。
pub(crate) const SPACE_XS: f32 = 4.0;
pub(crate) const SPACE_SM: f32 = 8.0;
pub(crate) const SPACE_MD: f32 = 12.0;
pub(crate) const SPACE_LG: f32 = 16.0;
pub(crate) const SPACE_XL: f32 = 24.0;

// 3 档字号：辅助信息 / 正文 / 标题。
pub(crate) const FONT_SMALL: f32 = 11.0;
pub(crate) const FONT_BODY: f32 = 13.0;
pub(crate) const FONT_TITLE: f32 = 20.0;

/// hover / selected 等微交互时长。
pub(crate) const ANIM_FAST: f32 = 0.15;
/// 侧栏切换、弹窗、详情等面板级过渡时长。
pub(crate) const ANIM_SLOW: f32 = 0.20;

/// 按透明度系数给颜色加 alpha，用于淡入淡出。
pub(crate) fn with_alpha(color: Color32, t: f32) -> Color32 {
    let alpha = (255.0 * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Top,
    Search,
    Cleanup,
}

impl SideTab {
    fn index(self) -> usize {
        match self {
            SideTab::Top => 0,
            SideTab::Search => 1,
            SideTab::Cleanup => 2,
        }
    }
}

struct ScanHandle {
    progress: Arc<ScanProgress>,
    rx: mpsc::Receiver<Result<Tree, ScanError>>,
    started: Instant,
}

pub struct DiskApp {
    path_input: String,
    drives: Vec<String>,
    tree: Option<Tree>,
    current: u32,
    selected: Option<u32>,
    hovered: Option<u32>,
    /// 树图的缩放/平移状态。
    treemap: treemap_view::ViewState,
    /// 目录导航历史：后退 / 前进。
    back_stack: Vec<u32>,
    forward_stack: Vec<u32>,
    scan: Option<ScanHandle>,
    side_tab: SideTab,
    query: String,
    results: Vec<u32>,
    top_files: Vec<u32>,
    message: Option<(String, Instant)>,
    show_help: bool,
    /// 等待用户二次确认的删除目标。
    pending_delete: Option<u32>,
    cleanup: CleanupPanel,
}

impl DiskApp {
    pub fn new(initial: Option<String>) -> Self {
        let drives = available_drives();
        let path_input = if drives.iter().any(|d| d.starts_with("C:")) {
            "C:\\".to_string()
        } else {
            drives
                .first()
                .cloned()
                .unwrap_or_else(|| "C:\\".to_string())
        };
        let mut app = Self {
            path_input,
            drives,
            tree: None,
            current: 0,
            selected: None,
            hovered: None,
            treemap: treemap_view::ViewState::default(),
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            scan: None,
            side_tab: SideTab::Top,
            query: String::new(),
            results: Vec::new(),
            top_files: Vec::new(),
            message: None,
            show_help: false,
            pending_delete: None,
            cleanup: CleanupPanel::default(),
        };
        if let Some(path) = initial {
            let path = path.trim().trim_matches('"').trim().to_string();
            if !path.is_empty() {
                app.path_input = path;
                app.start_scan();
            }
        }
        app
    }

    fn toast(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }

    fn browse(&mut self) {
        if let Some(dir) = rfd::FileDialog::new()
            .set_title("选择要分析的文件夹")
            .pick_folder()
        {
            self.path_input = dir.display().to_string();
        }
    }

    fn cancel_scan(&mut self) {
        if let Some(handle) = &self.scan {
            handle.progress.request_cancel();
            self.toast("正在取消扫描…");
        }
    }

    fn start_scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let raw = self.path_input.trim().trim_matches('"').trim().to_string();
        if raw.is_empty() {
            self.toast("请先填写要分析的目录");
            return;
        }
        let path = PathBuf::from(&raw);
        if !path.exists() {
            self.toast(format!("目录不存在：{raw}"));
            return;
        }
        if !path.is_dir() {
            self.toast("请选择文件夹，而不是文件");
            return;
        }

        logging::event(Level::Info, "scan_start", &[("root", Value::Str(&raw))]);
        let progress = Arc::new(ScanProgress::new());
        let (tx, rx) = mpsc::channel();
        let worker_progress = Arc::clone(&progress);
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let result = disk_core::scan(&worker_path, worker_progress);
            let _ = tx.send(result);
        });
        self.scan = Some(ScanHandle {
            progress,
            rx,
            started: Instant::now(),
        });
        self.message = None;
        self.pending_delete = None;
    }

    fn poll_scan(&mut self) {
        let mut received = None;
        if let Some(handle) = &self.scan {
            match handle.rx.try_recv() {
                Ok(result) => received = Some(result),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    received = Some(Err(ScanError::Io("扫描线程意外结束".to_string())))
                }
            }
        }
        let Some(result) = received else {
            return;
        };
        self.scan = None;
        match result {
            Ok(tree) => {
                let top = tree.top_files(0, 80);
                let summary = format!(
                    "扫描完成：{} 个文件，用时 {:.1} 秒",
                    tree.total_files(),
                    tree.elapsed.as_secs_f64()
                );
                logging::event(
                    Level::Info,
                    "scan_done",
                    &[
                        ("root", Value::Str(&tree.root_path)),
                        ("files", Value::Num(tree.total_files())),
                        ("dirs", Value::Num(tree.dir_count())),
                        ("bytes", Value::Num(tree.total_size())),
                        ("skipped_links", Value::Num(tree.skipped_links)),
                        ("read_errors", Value::Num(tree.read_error_count)),
                        ("elapsed_ms", Value::Num(tree.elapsed.as_millis() as u64)),
                    ],
                );
                self.path_input = tree.root_path.clone();
                self.current = 0;
                self.selected = None;
                self.hovered = None;
                self.back_stack.clear();
                self.forward_stack.clear();
                self.treemap.reset();
                self.results.clear();
                self.query.clear();
                self.top_files = top;
                self.tree = Some(tree);
                self.cleanup.invalidate();
                self.toast(summary);
            }
            Err(ScanError::Cancelled) => {
                logging::event(Level::Warn, "scan_cancel", &[]);
                self.toast("扫描已取消");
            }
            Err(error) => {
                logging::event(
                    Level::Error,
                    "scan_error",
                    &[("error", Value::Str(&error.to_string()))],
                );
                self.toast(format!("扫描失败：{error}"));
            }
        }
    }

    fn set_current(&mut self, id: u32) {
        if id != self.current {
            self.back_stack.push(self.current);
            self.forward_stack.clear();
            self.treemap.reset();
        }
        self.current = id;
        self.selected = Some(id);
        self.hovered = None;
        self.results.clear();
        self.query.clear();
        if let Some(tree) = &self.tree {
            self.top_files = tree.top_files(id, 80);
        }
    }

    fn go_back(&mut self) {
        if let Some(previous) = self.back_stack.pop() {
            self.forward_stack.push(self.current);
            self.current = previous;
            self.selected = None;
            self.hovered = None;
            self.results.clear();
            self.query.clear();
            if let Some(tree) = &self.tree {
                self.top_files = tree.top_files(previous, 80);
            }
            self.treemap.reset();
        }
    }

    fn go_forward(&mut self) {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(self.current);
            self.current = next;
            self.selected = None;
            self.hovered = None;
            self.results.clear();
            self.query.clear();
            if let Some(tree) = &self.tree {
                self.top_files = tree.top_files(next, 80);
            }
            self.treemap.reset();
        }
    }

    fn go_up(&mut self) {
        if self.current == 0 {
            return;
        }
        if let Some(tree) = &self.tree {
            let parent = tree.node(self.current).parent;
            self.set_current(parent);
        }
    }

    fn run_search(&mut self) {
        let Some(tree) = &self.tree else {
            return;
        };
        self.results = tree.search(self.current, &self.query, 200);
    }

    fn export_csv(&mut self) {
        let payload = self.tree.as_ref().map(|tree| {
            (
                tree.children_csv(self.current),
                tree.node(self.current).name.to_string(),
            )
        });
        let Some((csv, node_name)) = payload else {
            return;
        };
        let default_name = format!("目录清单_{}.csv", sanitize_file_name(&node_name));
        let Some(path) = rfd::FileDialog::new()
            .set_title("导出当前目录清单")
            .set_file_name(default_name)
            .add_filter("CSV 文件", &["csv"])
            .save_file()
        else {
            return;
        };
        let mut bytes = Vec::with_capacity(csv.len() + 3);
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        bytes.extend_from_slice(csv.as_bytes());
        match std::fs::write(&path, bytes) {
            Ok(_) => self.toast(format!("已导出：{}", path.display())),
            Err(error) => self.toast(format!("导出失败：{error}")),
        }
    }

    /// 批量删除结束后把已移除的节点从树上摘掉，并刷新受影响的视图数据。
    fn apply_cleanup_result(&mut self) {
        let Some((removed, freed)) = self.cleanup.poll() else {
            return;
        };
        let count = removed.len();
        let mut current_removed = false;
        if let Some(tree) = self.tree.as_mut() {
            current_removed = removed.contains(&self.current);
            for id in removed {
                tree.detach(id);
            }
        }
        if current_removed {
            self.current = 0;
        }
        if let Some(tree) = self.tree.as_ref() {
            self.top_files = tree.top_files(self.current, 80);
        }
        self.selected = None;
        self.hovered = None;
        self.results.clear();
        logging::event(
            Level::Info,
            "delete_batch",
            &[
                ("count", Value::Num(count as u64)),
                ("freed_bytes", Value::Num(freed)),
            ],
        );
        self.toast(format!(
            "已移入回收站 {} 项，释放 {}",
            count,
            format_size(freed)
        ));
    }

    fn delete_node(&mut self, id: u32) {
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        if id == 0 || id as usize >= tree.nodes.len() || tree.node(id).parent == NO_PARENT {
            return;
        }
        let path = tree.path_of(id);
        let name = tree.node(id).name.to_string();
        let new_current = if tree.ancestors(self.current).contains(&id) {
            tree.node(id).parent
        } else {
            self.current
        };

        match disk_core::recycle(std::path::Path::new(&path)) {
            Ok(()) => {
                logging::event(
                    Level::Info,
                    "delete_ok",
                    &[("name", Value::Str(&name)), ("path", Value::Str(&path))],
                );
                if let Some(tree) = self.tree.as_mut() {
                    tree.detach(id);
                }
                self.current = new_current;
                self.selected = None;
                self.hovered = None;
                self.back_stack.clear();
                self.forward_stack.clear();
                self.treemap.reset();
                self.results.clear();
                self.query.clear();
                if let Some(tree) = self.tree.as_ref() {
                    self.top_files = tree.top_files(new_current, 80);
                }
                self.cleanup.invalidate();
                self.toast(format!("已移入回收站：{}", name));
            }
            Err(error) => {
                logging::event(
                    Level::Warn,
                    "delete_error",
                    &[
                        ("path", Value::Str(&path)),
                        ("error", Value::Str(&error.to_string())),
                    ],
                );
                self.toast(format!("删除失败：{}", error))
            }
        }
    }

    fn confirm_delete_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pending_delete else {
            // 关闭状态下把动画归零，下次弹出才有 200ms 淡入。
            ctx.animate_bool_with_time(egui::Id::new("confirm_delete_fade"), false, ANIM_SLOW);
            return;
        };
        let Some(tree) = self.tree.as_ref() else {
            self.pending_delete = None;
            return;
        };
        if id == 0 || id as usize >= tree.nodes.len() || tree.node(id).parent == NO_PARENT {
            self.pending_delete = None;
            return;
        }
        let node = tree.node(id);
        let kind = node.kind;
        let name = node.name.to_string();
        let size = node.size;
        let files = node.files;
        let path = tree.path_of(id);

        let fade = ctx.animate_bool_with_time_and_easing(
            egui::Id::new("confirm_delete_fade"),
            true,
            ANIM_SLOW,
            egui::emath::easing::cubic_out,
        );
        let modal = egui::Modal::new(egui::Id::new("confirm_delete"))
            .backdrop_color(Color32::from_black_alpha((100.0 * fade) as u8))
            .frame(egui::Frame::popup(&ctx.style()).fill(with_alpha(BG_CARD, fade)))
            .show(ctx, |ui| {
                ui.set_opacity(fade);
                ui.set_max_width(460.0);
                ui.heading("确认移入回收站");
                ui.add_space(SPACE_XS);
                ui.label(
                    RichText::new(format!("{}：{}", kind.label(), truncate_chars(&name, 60)))
                        .strong(),
                );
                ui.label(format!("大小：{} · 文件数：{}", format_size(size), files));
                ui.label(RichText::new(shorten_path(&path, 80)).small().color(MUTED));
                ui.add_space(SPACE_SM);
                ui.label(
                    RichText::new("删除后会从当前统计中移除，可在系统回收站中还原。").color(WARN),
                );
                ui.add_space(SPACE_SM);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        Some(false)
                    } else if ui
                        .add(
                            egui::Button::new(RichText::new("确认删除").strong())
                                .fill(DANGER)
                                .min_size(egui::vec2(96.0, 28.0)),
                        )
                        .clicked()
                    {
                        Some(true)
                    } else {
                        None
                    }
                })
                .inner
            });

        let backdrop = modal.backdrop_response;
        let mut decision = modal.inner;
        if backdrop.clicked() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                self.pending_delete = None;
                self.delete_node(id);
            }
            Some(false) => self.pending_delete = None,
            None => {}
        }
    }

    fn top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(SPACE_MD as i8, SPACE_SM as i8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("磁盘空间分析器")
                            .heading()
                            .strong()
                            .color(TEXT),
                    );
                    ui.label(
                        RichText::new("扫描只读 · 删除走回收站，可还原")
                            .small()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("帮助").clicked() {
                            self.show_help = true;
                        }
                        let can_export = self.tree.is_some() && self.scan.is_none();
                        if ui
                            .add_enabled(can_export, egui::Button::new("导出当前目录 CSV"))
                            .clicked()
                        {
                            self.export_csv();
                        }
                        if can_export && ui.button("重新扫描").clicked() {
                            self.start_scan();
                        }
                    });
                });
                ui.add_space(SPACE_XS);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("目录").color(MUTED));
                    let width = (ui.available_width() - 230.0).max(200.0);
                    let path_response = ui.add_sized(
                        [width, 28.0],
                        egui::TextEdit::singleline(&mut self.path_input)
                            .hint_text("例如 C:\\ 或 D:\\资料"),
                    );
                    if path_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && self.scan.is_none()
                    {
                        self.start_scan();
                    }
                    if ui.button("浏览…").clicked() {
                        self.browse();
                    }
                    if self.scan.is_some() {
                        if ui.button("取消扫描").clicked() {
                            self.cancel_scan();
                        }
                    } else if ui
                        .add(egui::Button::new(RichText::new("开始扫描").strong()).fill(ACCENT))
                        .clicked()
                    {
                        self.start_scan();
                    }
                });
                ui.add_space(SPACE_XS);
                let drives = self.drives.clone();
                let mut jump: Option<String> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("快速选择").small().color(MUTED));
                    for drive in drives {
                        if ui.small_button(&drive).clicked() {
                            self.path_input = drive;
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new("常用位置").small().color(MUTED));
                    for (label, path) in cleanup_locations() {
                        if ui.small_button(label).on_hover_text(&path).clicked() {
                            jump = Some(path);
                        }
                    }
                });
                if let Some(path) = jump {
                    if self.scan.is_none() {
                        self.path_input = path;
                        self.start_scan();
                    }
                }
            });
    }

    fn bottom_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(SPACE_MD as i8, SPACE_XS as i8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(handle) = &self.scan {
                        ui.spinner();
                        let snapshot = handle.progress.snapshot();
                        let state = if handle.progress.is_cancelled() {
                            "正在取消"
                        } else {
                            "扫描中"
                        };
                        ui.label(format!(
                            "{} · 已扫描 {} 个文件 · {} 个文件夹 · {} · {}s",
                            state,
                            snapshot.files,
                            snapshot.dirs,
                            format_size(snapshot.bytes),
                            handle.started.elapsed().as_secs()
                        ));
                        ui.label(
                            RichText::new(shorten_path(&snapshot.current, 70))
                                .small()
                                .color(MUTED),
                        );
                    } else if let Some(tree) = &self.tree {
                        ui.label(format!(
                            "{} · {} 个文件 · {} 个文件夹 · 跳过链接 {} · 读取失败 {}",
                            format_size(tree.total_size()),
                            tree.total_files(),
                            tree.dir_count(),
                            tree.skipped_links,
                            tree.read_error_count
                        ));
                        if let Some(id) = self.hovered {
                            let node = tree.node(id);
                            let base = tree.node(self.current).size;
                            let pct = if base > 0 {
                                node.size as f64 * 100.0 / base as f64
                            } else {
                                0.0
                            };
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} · {:.1}%",
                                    node.name,
                                    format_size(node.size),
                                    pct
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                    } else {
                        ui.label(RichText::new("就绪 · 选择目录后开始扫描").color(MUTED));
                    }
                    if let Some((text, _)) = &self.message {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(text).small().color(ACCENT));
                        });
                    }
                });
            });
    }

    fn side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(360.0)
            .min_width(300.0)
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .inner_margin(egui::Margin::symmetric(SPACE_MD as i8, SPACE_MD as i8)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("详情与清单").strong());
                ui.add_space(SPACE_XS);

                // 详情卡片：选中项出现时 200ms 淡入。
                let detail_open = self.selected.is_some();
                let detail_t = ctx.animate_bool_with_time_and_easing(
                    egui::Id::new("detail_open"),
                    detail_open,
                    ANIM_SLOW,
                    egui::emath::easing::cubic_out,
                );
                if detail_open {
                    // 背景与内容一起淡入，避免卡片底色先整块弹出造成硬切观感。
                    egui::Frame::new()
                        .fill(with_alpha(BG_CARD, detail_t))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::same(SPACE_SM as i8))
                        .show(ui, |ui| {
                            ui.set_opacity(detail_t);
                            self.draw_selected(ui);
                        });
                } else {
                    self.draw_selected(ui);
                }

                ui.add_space(SPACE_LG);
                ui.separator();
                ui.add_space(SPACE_SM);

                // 侧栏页签：指示条与内容切换统一 200ms 减速过渡。
                let mut tab_t = [0.0_f32; 3];
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (SideTab::Top, "大文件"),
                        (SideTab::Search, "搜索"),
                        (SideTab::Cleanup, "清理候选"),
                    ] {
                        let active = self.side_tab == tab;
                        let t = ctx.animate_bool_with_time_and_easing(
                            egui::Id::new(("side_tab", tab.index())),
                            active,
                            ANIM_SLOW,
                            egui::emath::easing::cubic_out,
                        );
                        tab_t[tab.index()] = t;
                        let response = ui.selectable_label(active, label);
                        if response.clicked() {
                            self.side_tab = tab;
                        }
                        if t > 0.0 {
                            let rect = response.rect;
                            let underline = egui::Rect::from_min_max(
                                egui::pos2(rect.left() + SPACE_XS, rect.bottom() - 2.0),
                                egui::pos2(rect.right() - SPACE_XS, rect.bottom()),
                            );
                            ui.painter().rect_filled(
                                underline,
                                egui::CornerRadius::ZERO,
                                with_alpha(ACCENT, t),
                            );
                        }
                    }
                });
                ui.add_space(SPACE_XS);
                let content_t = tab_t[self.side_tab.index()];
                ui.scope(|ui| {
                    ui.set_opacity(content_t);
                    match self.side_tab {
                        SideTab::Top => {
                            ui.label(
                                RichText::new("当前目录范围内最大的文件")
                                    .small()
                                    .color(MUTED),
                            );
                            let ids = self.top_files.clone();
                            if ids.is_empty() {
                                ui.label(RichText::new("暂无数据").color(MUTED));
                            } else {
                                self.draw_file_list(ui, ids);
                            }
                        }
                        SideTab::Search => {
                            let mut do_search = false;
                            ui.horizontal(|ui| {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut self.query)
                                        .desired_width(200.0)
                                        .hint_text("文件名关键字"),
                                );
                                if response.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    do_search = true;
                                }
                                if ui.button("搜索").clicked() {
                                    do_search = true;
                                }
                            });
                            if do_search {
                                self.run_search();
                            }
                            ui.label(
                                RichText::new(format!(
                                    "在当前目录内搜索，命中 {} 个",
                                    self.results.len()
                                ))
                                .small()
                                .color(MUTED),
                            );
                            let ids = self.results.clone();
                            if ids.is_empty() {
                                ui.label(
                                    RichText::new("输入关键字后按回车开始搜索")
                                        .small()
                                        .color(MUTED),
                                );
                            } else {
                                self.draw_file_list(ui, ids);
                            }
                        }
                        SideTab::Cleanup => {
                            let scan_running = self.scan.is_some();
                            let mut jump = None;
                            match (&self.tree, &mut self.cleanup) {
                                (Some(tree), cleanup) => {
                                    if cleanup.is_stale() {
                                        cleanup.refresh(tree);
                                    }
                                    jump = cleanup.ui(ui, tree, scan_running);
                                }
                                (None, _) => {
                                    ui.label(
                                        RichText::new("先扫描一个目录，再来看清理候选")
                                            .small()
                                            .color(MUTED),
                                    );
                                }
                            }
                            if let Some(id) = jump {
                                self.selected = Some(id);
                            }
                        }
                    }
                });
            });
    }

    fn draw_selected(&mut self, ui: &mut egui::Ui) {
        let mut goto = None;
        let mut copy_path = None;
        let mut request_delete = None;
        if let (Some(tree), Some(id)) = (&self.tree, self.selected) {
            let node = tree.node(id);
            let path = tree.path_of(id);
            ui.horizontal(|ui| {
                ui.label(RichText::new(truncate_chars(&node.name, 40)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(node.kind.label()).small().color(MUTED));
                });
            });
            ui.label(format!(
                "大小：{}（{} 字节）",
                format_size(node.size),
                node.size
            ));
            ui.label(format!("文件数：{}", node.files));
            ui.label(RichText::new(&path).small().color(MUTED));
            let protected = disk_core::is_protected(std::path::Path::new(&path));
            ui.horizontal(|ui| {
                if node.kind == NodeKind::Dir && ui.button("进入此文件夹").clicked() {
                    goto = Some(id);
                }
                if ui.button("复制路径").clicked() {
                    copy_path = Some(path.clone());
                }
                if self.scan.is_some() {
                    ui.add_enabled(false, egui::Button::new("移入回收站"))
                        .on_disabled_hover_text("扫描进行中，完成后再操作");
                } else if let Some(reason) = &protected {
                    ui.add_enabled(false, egui::Button::new("移入回收站"))
                        .on_disabled_hover_text(reason);
                } else if ui
                    .add(egui::Button::new(RichText::new("移入回收站")).fill(DANGER))
                    .clicked()
                {
                    request_delete = Some(id);
                }
            });
            ui.label(
                RichText::new("移入回收站后可还原；系统目录受保护。")
                    .small()
                    .color(MUTED),
            );
            if let Some(error) = &node.error {
                ui.label(
                    RichText::new(format!("读取失败：{error}"))
                        .small()
                        .color(WARN),
                );
            }
            ui.add_space(SPACE_XS);
        } else {
            ui.label(
                RichText::new("点击树图中的方块查看详情")
                    .small()
                    .color(MUTED),
            );
        }
        if let Some(id) = goto {
            self.set_current(id);
        }
        if let Some(path) = copy_path {
            ui.ctx().copy_text(path);
            self.toast("路径已复制");
        }
        if let Some(id) = request_delete {
            self.pending_delete = Some(id);
        }
    }

    fn draw_file_list(&mut self, ui: &mut egui::Ui, ids: Vec<u32>) {
        let selected = self.selected;
        let mut clicked = None;
        if let Some(tree) = &self.tree {
            let rows: Vec<(u32, String, u64, String, bool)> = ids
                .iter()
                .map(|id| {
                    let node = tree.node(*id);
                    (
                        *id,
                        node.name.to_string(),
                        node.size,
                        tree.path_of(*id),
                        node.kind == NodeKind::Dir,
                    )
                })
                .collect();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (id, name, size, path, is_dir) in &rows {
                        ui.horizontal(|ui| {
                            let marker = if *is_dir { "▸" } else { "·" };
                            let label = format!("{} {}", marker, truncate_chars(name, 34));
                            let response =
                                ui.selectable_label(selected == Some(*id), RichText::new(label));
                            if response.clicked() {
                                clicked = Some(*id);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_size(*size)).small().color(MUTED),
                                    );
                                },
                            );
                            response.on_hover_text(format!("{}\n{}", path, format_size(*size)));
                        });
                    }
                });
        }
        if let Some(id) = clicked {
            self.selected = Some(id);
        }
    }

    fn central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(BG_WINDOW)
                    .inner_margin(egui::Margin::same(SPACE_MD as i8)),
            )
            .show(ctx, |ui| {
                if self.tree.is_none() {
                    self.draw_welcome(ui);
                    return;
                }

                let mut goto = None;
                let mut go_back = false;
                let mut go_forward = false;
                {
                    let tree = self.tree.as_ref().unwrap();
                    let ancestors = tree.ancestors(self.current);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(!self.back_stack.is_empty(), egui::Button::new("◀"))
                            .on_hover_text("后退 (Alt+←)")
                            .clicked()
                        {
                            go_back = true;
                        }
                        if ui
                            .add_enabled(!self.forward_stack.is_empty(), egui::Button::new("▶"))
                            .on_hover_text("前进 (Alt+→)")
                            .clicked()
                        {
                            go_forward = true;
                        }
                        ui.separator();
                        if ui
                            .add_enabled(self.current != 0, egui::Button::new("上一级"))
                            .clicked()
                        {
                            goto = Some(tree.node(self.current).parent);
                        }
                        ui.separator();
                        for (index, id) in ancestors.iter().enumerate() {
                            if index > 0 {
                                ui.label(RichText::new("›").color(MUTED));
                            }
                            let name = if *id == 0 {
                                tree.root_path.clone()
                            } else {
                                tree.node(*id).name.to_string()
                            };
                            let active = *id == self.current;
                            if ui.selectable_label(active, RichText::new(name)).clicked() {
                                goto = Some(*id);
                            }
                        }
                    });
                    let current = tree.node(self.current);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} 个直属条目 · {} 个文件 · {}",
                                current.children.len(),
                                current.files,
                                format_size(current.size)
                            ))
                            .small()
                            .color(MUTED),
                        );
                    });
                }
                if let Some(id) = goto {
                    self.set_current(id);
                }
                if go_back {
                    self.go_back();
                }
                if go_forward {
                    self.go_forward();
                }
                ui.add_space(SPACE_XS);

                let mut hovered = self.hovered;
                let clicked = {
                    let tree = self.tree.as_ref().unwrap();
                    treemap_view::draw(
                        ui,
                        tree,
                        self.current,
                        self.selected,
                        &mut hovered,
                        &mut self.treemap,
                    )
                };
                self.hovered = hovered;
                if let Some(interaction) = clicked {
                    let is_dir = self
                        .tree
                        .as_ref()
                        .map(|tree| tree.node(interaction.id).kind == NodeKind::Dir)
                        .unwrap_or(false);
                    if interaction.double && is_dir {
                        self.set_current(interaction.id);
                    } else {
                        self.selected = Some(interaction.id);
                    }
                }
            });
    }

    fn draw_welcome(&self, ui: &mut egui::Ui) {
        ui.add_space(SPACE_XL);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("磁盘空间分析器")
                    .heading()
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(SPACE_SM);
            ui.label(
                RichText::new("按面积展示每个文件夹与文件占用的空间，点击方块逐层下钻")
                    .color(MUTED),
            );
            ui.add_space(SPACE_XL);
            ui.label("1. 在上方输入或选择要分析的目录（例如 C:\\）");
            ui.label("2. 点击「开始扫描」，完成后自动生成矩形树图");
            ui.label("3. 点击文件夹方块下钻，点击文件方块查看详情");
            ui.add_space(SPACE_XL);
            ui.label(
                RichText::new("删除会二次确认并移入回收站，可随时从回收站还原。")
                    .color(WARN)
                    .strong(),
            );
        });
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut open = self.show_help;
        egui::Window::new("使用说明")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("· 选择目录后点击「开始扫描」，左侧会按占用面积生成矩形树图。");
                ui.label("· 方块面积与目录/文件大小成正比，越大越接近正方形。");
                ui.label("· 双击文件夹方块进入下一层，单击为选中；滚轮放大、拖动平移。");
                ui.label("· 「◀」「▶」或 Alt+← / Alt+→ 在浏览历史中后退、前进。");
                ui.label("· 右侧可查看选中项详情、当前目录的大文件，并支持按名称搜索。");
                ui.label("· 「导出当前目录 CSV」会把直属子项清单保存为 UTF-8 BOM 文件。");
                ui.label("· 详情中的「移入回收站」会先弹出确认框，成功后从当前统计里移除该项。");
                ui.add_space(SPACE_SM);
                ui.label(RichText::new("扫描只读；删除只走回收站，不会永久抹除。").color(WARN));
                ui.label(
                    RichText::new(
                        "磁盘根目录、Windows、Program Files、ProgramData、用户目录根、\
                         回收站等系统位置禁止删除；符号链接与目录联接也不会被删除。",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.label(
                    RichText::new("符号链接与目录联接在扫描时默认跳过，避免重复统计和循环。")
                        .small()
                        .color(MUTED),
                );
                ui.add_space(SPACE_SM);
                ui.label(
                    RichText::new(format!("版本 v{}", env!("CARGO_PKG_VERSION")))
                        .small()
                        .color(MUTED),
                );
            });
        self.show_help = open;
    }
}

impl eframe::App for DiskApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let expired = self
            .message
            .as_ref()
            .map(|(_, at)| at.elapsed() > Duration::from_secs(8))
            .unwrap_or(false);
        if expired {
            self.message = None;
        }

        if !self.show_help
            && self.pending_delete.is_none()
            && !ctx.wants_keyboard_input()
            && ctx
                .input(|i| i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Escape))
        {
            self.go_up();
        }
        if !self.show_help && self.pending_delete.is_none() && !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowLeft)) {
                self.go_back();
            }
            if ctx.input(|i| i.modifiers.alt && i.key_pressed(egui::Key::ArrowRight)) {
                self.go_forward();
            }
        }

        self.poll_scan();
        self.apply_cleanup_result();
        self.top_panel(ctx);
        self.bottom_panel(ctx);
        self.side_panel(ctx);
        self.central_panel(ctx);
        self.help_window(ctx);
        self.confirm_delete_window(ctx);

        if self.scan.is_some() {
            // 100ms 刷新一次足够让进度条平滑推进，又不会持续满帧重绘。
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn available_drives() -> Vec<String> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if std::path::Path::new(&root).is_dir() {
            out.push(root);
        }
    }
    out
}

/// 常见的可清理目录，点击后直接扫描。
fn cleanup_locations() -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (label, key, suffix) in [
        ("用户临时目录", "TEMP", ""),
        ("Windows 临时", "SystemRoot", "\\Temp"),
        ("下载目录", "USERPROFILE", "\\Downloads"),
    ] {
        let Ok(base) = std::env::var(key) else {
            continue;
        };
        let path = if suffix.is_empty() {
            PathBuf::from(&base)
        } else {
            PathBuf::from(format!("{}{}", base.trim_end_matches(['\\', '/']), suffix))
        };
        if path.is_dir() {
            out.push((label, path.display().to_string()));
        }
    }
    out
}

pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(crate) fn shorten_path(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let tail: String = chars[chars.len() - (max.saturating_sub(1))..]
        .iter()
        .collect();
    format!("…{tail}")
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "目录清单".to_string()
    } else {
        trimmed
    }
}
