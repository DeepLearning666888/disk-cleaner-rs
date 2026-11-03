//! 清理候选面板：按规则筛选、勾选、批量移入回收站并给出结果报告。
//!
//! 删除始终在后台线程执行，界面只通过 channel 收结果，避免大批量删除时卡住渲染。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use disk_core::{format_size, CandidateSet, RuleOptions, Tree};
use eframe::egui::{self, RichText};

use crate::app::{
    shorten_path, truncate_chars, with_alpha, ACCENT, ANIM_SLOW, BG_CARD, DANGER, MUTED, SPACE_SM,
    SPACE_XS, TEXT, WARN,
};

/// 单次筛选最多展示多少项，避免极端情况下列表过长。
const CANDIDATE_LIMIT: usize = 2000;

pub struct DeleteReport {
    /// 已成功移入回收站的节点 id。
    pub removed: Vec<u32>,
    pub freed: u64,
    pub failures: Vec<(String, String)>,
    pub cancelled: bool,
}

struct DeleteJob {
    rx: mpsc::Receiver<DeleteReport>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize, String)>>,
    started: Instant,
}

/// 列表里的一行，筛完就固定下来，避免每帧重新分配字符串。
struct Row {
    id: u32,
    name: String,
    path: String,
    size: u64,
    reason: &'static str,
    protected: Option<String>,
}

pub struct CleanupPanel {
    pub options: RuleOptions,
    set: CandidateSet,
    rows: Vec<Row>,
    selected: HashSet<u32>,
    job: Option<DeleteJob>,
    confirm_open: bool,
    report: Option<DeleteReport>,
    stale: bool,
    threshold_mb: u64,
}

impl Default for CleanupPanel {
    fn default() -> Self {
        let options = RuleOptions::default();
        Self {
            threshold_mb: options.large_threshold / (1024 * 1024),
            options,
            set: CandidateSet::default(),
            rows: Vec::new(),
            selected: HashSet::new(),
            job: None,
            confirm_open: false,
            report: None,
            stale: true,
        }
    }
}

impl CleanupPanel {
    /// 目录树发生变化（重新扫描或删除后）时调用，下次进入面板重新筛选。
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    pub fn is_stale(&self) -> bool {
        self.stale
    }

    pub fn refresh(&mut self, tree: &Tree) {
        self.options.large_threshold = self.threshold_mb.saturating_mul(1024 * 1024);
        self.set = disk_core::collect_candidates(tree, &self.options, CANDIDATE_LIMIT);
        self.rows = self
            .set
            .items
            .iter()
            .map(|item| Row {
                id: item.id,
                name: std::path::Path::new(&item.path)
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| item.path.clone()),
                path: item.path.clone(),
                size: item.size,
                reason: item.reason,
                protected: item.protected.clone(),
            })
            .collect();
        self.selected.clear();
        self.stale = false;
        self.report = None;
    }

    /// 收删除结果；返回本次移除的节点 id 与释放的字节数，交给调用方更新目录树。
    pub fn poll(&mut self) -> Option<(Vec<u32>, u64)> {
        let job = self.job.as_ref()?;
        let report = match job.rx.try_recv() {
            Ok(report) => report,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                return None;
            }
        };
        self.job = None;
        let removed = report.removed.clone();
        let freed = report.freed;
        self.report = Some(report);
        self.stale = true;
        Some((removed, freed))
    }

    fn start_delete(&mut self, targets: Vec<(u32, String, u64)>) {
        if targets.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new((0usize, targets.len(), String::new())));
        let thread_cancel = Arc::clone(&cancel);
        let thread_progress = Arc::clone(&progress);
        let total = targets.len();
        std::thread::spawn(move || {
            let mut removed = Vec::new();
            let mut freed = 0u64;
            let mut failures = Vec::new();
            for (index, (id, path, size)) in targets.into_iter().enumerate() {
                if thread_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(DeleteReport {
                        removed,
                        freed,
                        failures,
                        cancelled: true,
                    });
                    return;
                }
                if let Ok(mut slot) = thread_progress.lock() {
                    *slot = (index + 1, total, path.clone());
                }
                match disk_core::recycle(std::path::Path::new(&path)) {
                    Ok(()) => {
                        removed.push(id);
                        freed = freed.saturating_add(size);
                    }
                    Err(error) => failures.push((path, error.to_string())),
                }
            }
            let _ = tx.send(DeleteReport {
                removed,
                freed,
                failures,
                cancelled: false,
            });
        });
        self.job = Some(DeleteJob {
            rx,
            cancel,
            progress,
            started: Instant::now(),
        });
        self.report = None;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, tree: &Tree, scan_running: bool) -> Option<u32> {
        ui.label(RichText::new("按规则筛选的清理候选").strong());
        ui.label(
            RichText::new("规则只负责筛选，是否可删由你判断；删除统一走回收站。")
                .small()
                .color(MUTED),
        );
        ui.add_space(SPACE_XS);

        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui
                .checkbox(&mut self.options.temporary, "临时/备份文件")
                .changed();
            changed |= ui
                .checkbox(&mut self.options.installers, "安装包")
                .changed();
            changed |= ui
                .checkbox(&mut self.options.empty_dirs, "空目录")
                .changed();
        });
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut self.options.large_files, "大文件")
                .changed();
            if self.options.large_files {
                ui.label(RichText::new("阈值").small().color(MUTED));
                if ui
                    .add(
                        egui::DragValue::new(&mut self.threshold_mb)
                            .range(1..=102_400)
                            .suffix(" MB"),
                    )
                    .changed()
                {
                    changed = true;
                }
            }
        });
        if changed {
            self.stale = true;
        }

        ui.add_space(SPACE_XS);
        ui.horizontal(|ui| {
            if ui.button("重新筛选").clicked() {
                self.refresh(tree);
            }
            let selectable: Vec<u32> = self
                .rows
                .iter()
                .filter(|row| row.protected.is_none())
                .map(|row| row.id)
                .collect();
            if ui.button("全选可删项").clicked() {
                self.selected.extend(selectable.iter().copied());
            }
            if ui.button("清空选择").clicked() {
                self.selected.clear();
            }
        });

        let selected_size: u64 = self
            .rows
            .iter()
            .filter(|row| self.selected.contains(&row.id))
            .map(|row| row.size)
            .sum();
        ui.label(
            RichText::new(format!(
                "共 {} 项 · 合计 {} ｜ 已选 {} 项 · {}",
                self.set.items.len(),
                format_size(self.set.total_size),
                self.selected.len(),
                format_size(selected_size)
            ))
            .small(),
        );
        if self.set.protected_count > 0 {
            ui.label(
                RichText::new(format!(
                    "{} 项落在系统保护范围，已禁止勾选",
                    self.set.protected_count
                ))
                .small()
                .color(WARN),
            );
        }
        if self.set.truncated > 0 {
            ui.label(
                RichText::new(format!("另有 {} 项超出展示上限未列出", self.set.truncated))
                    .small()
                    .color(MUTED),
            );
        }

        ui.add_space(SPACE_XS);
        self.draw_job_state(ui);
        self.draw_report(ui);

        let can_delete = !self.selected.is_empty() && !scan_running && self.job.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    can_delete,
                    egui::Button::new(RichText::new("移入回收站（已选）").strong()).fill(DANGER),
                )
                .clicked()
            {
                self.confirm_open = true;
            }
            if ui
                .add_enabled(self.job.is_some(), egui::Button::new("取消"))
                .clicked()
            {
                if let Some(job) = self.job.as_ref() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        if scan_running {
            ui.label(
                RichText::new("扫描进行中，完成后才能执行删除")
                    .small()
                    .color(MUTED),
            );
        }

        self.confirm_window(ui.ctx(), tree);

        ui.separator();
        if self.rows.is_empty() {
            ui.label(
                RichText::new("当前没有命中规则的项。换个目录或调整规则后再试。")
                    .small()
                    .color(MUTED),
            );
            return None;
        }

        // 用 take 把列表挪出来，闭包里才能同时改 self.selected。
        let rows = std::mem::take(&mut self.rows);
        let mut jump = None;
        let mut toggled: Vec<(u32, bool)> = Vec::new();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in &rows {
                    let protected = row.protected.is_some();
                    let mut checked = self.selected.contains(&row.id);
                    ui.horizontal(|ui| {
                        let response =
                            ui.add_enabled(!protected, egui::Checkbox::without_text(&mut checked));
                        if protected {
                            response
                                .on_hover_text(row.protected.as_deref().unwrap_or("受系统保护"));
                        } else if response.changed() {
                            toggled.push((row.id, checked));
                        }
                        let text = RichText::new(truncate_chars(&row.name, 26));
                        let text = if protected { text.color(MUTED) } else { text };
                        if ui
                            .add(egui::Label::new(text).sense(egui::Sense::click()))
                            .clicked()
                        {
                            jump = Some(row.id);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let color = if protected { MUTED } else { TEXT };
                            ui.label(RichText::new(format_size(row.size)).color(color));
                        });
                    });
                    ui.label(
                        RichText::new(format!("{} · {}", row.reason, shorten_path(&row.path, 56)))
                            .small()
                            .color(MUTED),
                    );
                    ui.add_space(SPACE_XS);
                }
            });
        self.rows = rows;
        for (id, checked) in toggled {
            if checked {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        jump
    }

    fn draw_job_state(&mut self, ui: &mut egui::Ui) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        let (done, total, current) =
            job.progress
                .lock()
                .map(|slot| slot.clone())
                .unwrap_or((0, 0, String::new()));
        let ratio = if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        };
        ui.add(egui::ProgressBar::new(ratio).text(format!("{}/{}", done, total)));
        ui.label(
            RichText::new(shorten_path(&current, 70))
                .small()
                .color(MUTED),
        );
        ui.label(
            RichText::new(format!(
                "已用时 {:.1} 秒",
                job.started.elapsed().as_secs_f32()
            ))
            .small()
            .color(MUTED),
        );
        ui.ctx().request_repaint_after(Duration::from_millis(150));
        ui.add_space(SPACE_XS);
    }

    fn draw_report(&mut self, ui: &mut egui::Ui) {
        let Some(report) = self.report.as_ref() else {
            return;
        };
        ui.add_space(SPACE_XS);
        let color = if report.failures.is_empty() {
            ACCENT
        } else {
            WARN
        };
        ui.label(
            RichText::new(format!(
                "{}：成功 {} 项，释放 {}，失败 {} 项",
                if report.cancelled {
                    "已取消"
                } else {
                    "清理完成"
                },
                report.removed.len(),
                format_size(report.freed),
                report.failures.len()
            ))
            .color(color)
            .strong(),
        );
        for (path, reason) in report.failures.iter().take(10) {
            ui.label(
                RichText::new(format!("· {}：{}", shorten_path(path, 46), reason))
                    .small()
                    .color(MUTED),
            );
        }
        if report.failures.len() > 10 {
            ui.label(
                RichText::new(format!("……另有 {} 项失败", report.failures.len() - 10))
                    .small()
                    .color(MUTED),
            );
        }
        ui.label(
            RichText::new("删除的项都在系统回收站，可随时还原。")
                .small()
                .color(MUTED),
        );
        ui.add_space(SPACE_XS);
    }

    /// 计算真正要删除的目标：跳过受保护项与已被选中父目录覆盖的子项。
    fn collect_targets(&self, tree: &Tree) -> (Vec<(u32, String, u64)>, usize) {
        let mut targets = Vec::new();
        let mut blocked = 0usize;
        for row in &self.rows {
            if !self.selected.contains(&row.id) {
                continue;
            }
            if row.protected.is_some()
                || disk_core::is_protected(std::path::Path::new(&row.path)).is_some()
            {
                blocked += 1;
                continue;
            }
            // ancestors 含自身，先排除自己再判断父目录是否也被选中。
            if tree
                .ancestors(row.id)
                .iter()
                .any(|ancestor| *ancestor != row.id && self.selected.contains(ancestor))
            {
                continue;
            }
            targets.push((row.id, row.path.clone(), row.size));
        }
        (targets, blocked)
    }

    fn confirm_window(&mut self, ctx: &egui::Context, tree: &Tree) {
        if !self.confirm_open {
            // 关闭状态下把动画归零，下次弹出才有 200ms 淡入。
            ctx.animate_bool_with_time(
                egui::Id::new("confirm_batch_delete_fade"),
                false,
                ANIM_SLOW,
            );
            return;
        }
        let (targets, blocked) = self.collect_targets(tree);
        let total: u64 = targets.iter().map(|entry| entry.2).sum();
        let blocked_hint = if blocked > 0 {
            format!("（{} 项受系统保护，将跳过）", blocked)
        } else {
            String::new()
        };

        let fade = ctx.animate_bool_with_time_and_easing(
            egui::Id::new("confirm_batch_delete_fade"),
            true,
            ANIM_SLOW,
            egui::emath::easing::cubic_out,
        );
        let modal = egui::Modal::new(egui::Id::new("confirm_batch_delete"))
            .backdrop_color(egui::Color32::from_black_alpha((100.0 * fade) as u8))
            .frame(egui::Frame::popup(&ctx.style()).fill(with_alpha(BG_CARD, fade)))
            .show(ctx, |ui| {
                ui.set_opacity(fade);
                ui.set_max_width(480.0);
                ui.heading("确认批量移入回收站");
                ui.add_space(SPACE_XS);
                ui.label(format!(
                    "将删除 {} 项，合计 {}{}",
                    targets.len(),
                    format_size(total),
                    blocked_hint
                ));
                ui.add_space(SPACE_XS);
                for (_, path, size) in targets.iter().take(12) {
                    ui.label(
                        RichText::new(format!(
                            "· {}（{}）",
                            shorten_path(path, 56),
                            format_size(*size)
                        ))
                        .small()
                        .color(MUTED),
                    );
                }
                if targets.len() > 12 {
                    ui.label(
                        RichText::new(format!("……另有 {} 项", targets.len() - 12))
                            .small()
                            .color(MUTED),
                    );
                }
                ui.add_space(SPACE_SM);
                ui.label(
                    RichText::new("删除后可在系统回收站还原，但不会自动恢复统计。").color(WARN),
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

        let mut decision = modal.inner;
        if modal.backdrop_response.clicked() {
            decision = Some(false);
        }
        match decision {
            Some(true) => {
                self.confirm_open = false;
                self.start_delete(targets);
            }
            Some(false) => self.confirm_open = false,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use disk_core::NodeKind;

    fn row(id: u32, name: &str, size: u64, protected: Option<&str>) -> Row {
        Row {
            id,
            name: name.to_string(),
            path: format!("C:\\demo\\{}", name),
            size,
            reason: "测试",
            protected: protected.map(|value| value.to_string()),
        }
    }

    fn tree() -> Tree {
        let mut tree = Tree::new("C:\\demo".to_string());
        let root = tree.add_node("C:\\demo".into(), disk_core::NO_PARENT, NodeKind::Dir);
        let folder = tree.add_node("旧安装包".into(), root, NodeKind::Dir);
        tree.add_node("setup-old.exe".into(), folder, NodeKind::File);
        tree.add_node("app.log".into(), root, NodeKind::File);
        tree.add_node("空目录".into(), root, NodeKind::Dir);
        tree
    }

    #[test]
    fn collect_targets_keeps_selected_leaves() {
        // 回归：ancestors 含自身，曾把所有被选中项都当成「父目录已选中」而全部跳过。
        let tree = tree();
        let panel = CleanupPanel {
            rows: vec![
                row(2, "setup-old.exe", 10, None),
                row(3, "app.log", 5, None),
                row(4, "空目录", 0, None),
            ],
            selected: [2u32, 3, 4].into_iter().collect(),
            ..CleanupPanel::default()
        };
        let (targets, blocked) = panel.collect_targets(&tree);
        assert_eq!(targets.len(), 3);
        assert_eq!(blocked, 0);
    }

    #[test]
    fn collect_targets_skips_children_of_selected_parent() {
        let tree = tree();
        let panel = CleanupPanel {
            rows: vec![
                row(1, "旧安装包", 10, None),
                row(2, "setup-old.exe", 10, None),
            ],
            selected: [1u32, 2].into_iter().collect(),
            ..CleanupPanel::default()
        };
        let (targets, _) = panel.collect_targets(&tree);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, 1);
    }

    #[test]
    fn collect_targets_skips_protected_rows() {
        let tree = tree();
        let panel = CleanupPanel {
            rows: vec![
                row(2, "setup-old.exe", 10, Some("系统目录")),
                row(3, "app.log", 5, None),
            ],
            selected: [2u32, 3].into_iter().collect(),
            ..CleanupPanel::default()
        };
        let (targets, blocked) = panel.collect_targets(&tree);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, 3);
        assert_eq!(blocked, 1);
    }

    #[cfg(windows)]
    #[test]
    fn batch_delete_pipeline_moves_selected_items_to_recycle_bin() {
        // 端到端：真实目录 → 扫描 → 筛选 → 勾选 → 收集目标 → 后台删除 → 结果报告 → 校验文件已移出。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(".testtmp")
            .join(format!("disk_app_cleanup_{}_{}", std::process::id(), nanos));
        let folder = root.join("旧安装包");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("setup-old.exe"), b"installer").unwrap();
        std::fs::write(root.join("app.log"), b"log").unwrap();
        std::fs::create_dir_all(root.join("空目录")).unwrap();

        let progress = Arc::new(disk_core::ScanProgress::new());
        let tree = disk_core::scan(&root, progress).unwrap();

        let mut panel = CleanupPanel::default();
        panel.refresh(&tree);
        assert_eq!(panel.rows.len(), 3, "应命中安装包、临时文件与空目录三项");
        assert_eq!(panel.set.protected_count, 0, "测试目录不应落入系统保护范围");

        panel.selected = panel.rows.iter().map(|row| row.id).collect();
        let (targets, blocked) = panel.collect_targets(&tree);
        assert_eq!(blocked, 0);
        assert_eq!(targets.len(), 3, "三项都应成为删除目标");

        panel.start_delete(targets);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut outcome = None;
        while std::time::Instant::now() < deadline {
            if let Some(result) = panel.poll() {
                outcome = Some(result);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let (removed, freed) = outcome.expect("删除任务应在 30 秒内结束");
        assert_eq!(removed.len(), 3);

        let report = panel.report.as_ref().expect("应留下结果报告");
        assert!(
            report.failures.is_empty(),
            "不应有失败项：{:?}",
            report.failures
        );
        assert!(!report.cancelled);
        assert_eq!(report.removed.len(), 3);
        assert_eq!(report.freed, freed, "报告释放量与返回值应一致");

        assert!(!root.join("app.log").exists(), "app.log 应已移出原位置");
        assert!(
            !folder.join("setup-old.exe").exists(),
            "setup-old.exe 应已移出原位置"
        );
        assert!(!root.join("空目录").exists(), "空目录应已移出原位置");

        let _ = std::fs::remove_dir_all(&root);
    }
}
