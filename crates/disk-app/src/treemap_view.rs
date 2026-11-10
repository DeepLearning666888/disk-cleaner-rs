//! 矩形树图绘制（嵌套式）。
//!
//! 参考 SpaceSniffer：目录框内留出标题行，子项继续在框内按面积分割，
//! 逐层嵌套。面积严格按真实大小占比分配；小格子靠滚轮放大或双击下钻看清。
//! 文字统一 11px 并严格裁剪在框内，不随缩放变大。

use disk_core::{format_size, NodeKind, Tree};
use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, StrokeKind};

use crate::app::{with_alpha, ACCENT, BG_WINDOW, FONT_BODY, FONT_SMALL, MUTED, SPACE_XS, TEXT};

const DIR_COLORS: [Color32; 3] = [
    Color32::from_rgb(54, 64, 82),
    Color32::from_rgb(48, 71, 88),
    Color32::from_rgb(61, 72, 95),
];
/// 物理下限：小于该尺寸的框无法显示，不再继续嵌套。
const MIN_VISIBLE: f32 = 1.0;
/// 目录标题行高度。
const TITLE_H: f32 = 15.0;
const BORDER: f32 = 1.0;
const HOVER_LIFT: u8 = 24;
const SELECT_LIFT: u8 = 38;
/// 最大放大倍数（滚轮缩放上限）。
const MAX_ZOOM: f32 = 8.0;
/// 同层条目达到该数量时，单个文件的最大面积占比才生效（条目太少时强行压缩会颠倒大小关系）。
const CAP_MIN_ITEMS: usize = 8;
/// 单个文件最多占据父级面积的比例，避免一个 exe/zip 铺满整屏。
const FILE_MAX_SHARE: f64 = 0.05;

/// 树图视图状态：滚轮缩放与拖动平移。
pub struct ViewState {
    zoom: f32,
    pan: egui::Vec2,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

impl ViewState {
    /// 切换目录时回到默认视图。
    pub fn reset(&mut self) {
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
    }

    /// 平移范围限制在当前画布内，避免把内容拖出视野。
    fn clamp_pan(&mut self, size: egui::Vec2) {
        if self.zoom <= 1.0 {
            self.pan = egui::Vec2::ZERO;
            return;
        }
        self.pan.x = self.pan.x.clamp(size.x * (1.0 - self.zoom), 0.0);
        self.pan.y = self.pan.y.clamp(size.y * (1.0 - self.zoom), 0.0);
    }
}

/// 点击结果：`double` 为真表示双击（用于进入目录）。
pub struct Interaction {
    pub id: u32,
    pub double: bool,
}

struct Tile {
    id: u32,
    /// 逻辑坐标（与画布同尺寸，不受缩放影响）。
    rect: Rect,
    /// 应用缩放/平移后的屏幕坐标，用于绘制与命中测试。
    screen: Rect,
    depth: usize,
    /// 该目录框内是否已展开子项（展开后不再重复绘制大小标签）。
    expanded: bool,
}

/// 绘制嵌套树图，返回点击结果。
pub fn draw(
    ui: &mut egui::Ui,
    tree: &Tree,
    current: u32,
    selected: Option<u32>,
    hovered: &mut Option<u32>,
    view: &mut ViewState,
) -> Option<Interaction> {
    *hovered = None;

    let available = ui.available_size();
    if available.x < 40.0 || available.y < 40.0 {
        return None;
    }
    let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
    let canvas = response.rect;
    painter.rect_filled(canvas, egui::CornerRadius::ZERO, BG_WINDOW);

    // 滚轮缩放：以光标为锚点，只放大不缩小。
    if response.hovered() {
        let scroll = ui.input(|input| input.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let old_zoom = view.zoom;
            let new_zoom = (old_zoom * (scroll * 0.0025).exp()).clamp(1.0, MAX_ZOOM);
            if let Some(cursor) = response.hover_pos() {
                let offset = cursor - canvas.min;
                view.pan = offset - (offset - view.pan) * (new_zoom / old_zoom);
            }
            view.zoom = new_zoom;
        }
    }
    // 拖动平移。
    if response.dragged() {
        view.pan += response.drag_delta();
    }
    view.clamp_pan(canvas.size());

    let items = area_weights(tree, current);

    if items.is_empty() {
        let message = if tree.node(current).children.is_empty() {
            "此目录为空"
        } else {
            "此目录下没有可统计大小的条目"
        };
        painter.text(
            canvas.center(),
            Align2::CENTER_CENTER,
            message,
            FontId::proportional(FONT_BODY),
            MUTED,
        );
        return None;
    }

    // 先完成整棵可见子树的布局，再统一绘制：命中测试取最深层，绘制顺序保证子项覆盖父项。
    let mut tiles: Vec<Tile> = Vec::new();
    for (id, rect) in disk_core::squarify(&items, canvas.width() as f64, canvas.height() as f64) {
        let tile = Rect::from_min_max(
            canvas.min + egui::vec2(rect.x as f32, rect.y as f32),
            canvas.min + egui::vec2((rect.x + rect.w) as f32, (rect.y + rect.h) as f32),
        );
        push_tile(tree, id, tile, 0, &mut tiles);
    }

    let zoom = view.zoom;
    let pan = view.pan;
    let scale = |rect: Rect| {
        Rect::from_min_max(
            canvas.min + (rect.min - canvas.min) * zoom + pan,
            canvas.min + (rect.max - canvas.min) * zoom + pan,
        )
    };
    for tile in &mut tiles {
        tile.screen = scale(tile.rect);
    }

    if let Some(position) = response.hover_pos() {
        for tile in tiles.iter().rev() {
            if tile.screen.contains(position) {
                *hovered = Some(tile.id);
                break;
            }
        }
    }

    let painter = painter.with_clip_rect(canvas);
    let ctx = ui.ctx().clone();
    for tile in &tiles {
        let node = tree.node(tile.id);
        let base = match node.kind {
            NodeKind::Dir => lighten(
                DIR_COLORS[tile.id as usize % DIR_COLORS.len()],
                (tile.depth as u8).saturating_mul(8),
            ),
            NodeKind::File => category_color(&node.name),
        };
        let is_hovered = *hovered == Some(tile.id);
        let is_selected = selected == Some(tile.id);
        // hover / selected 都走 150ms 动画，鼠标扫过或切换选中时不再瞬变。
        let hover_t = ctx.animate_bool(Id::new(("tile_hover", tile.id)), is_hovered);
        let select_t = ctx.animate_bool(Id::new(("tile_select", tile.id)), is_selected);
        let mut fill = base;
        if hover_t > 0.0 {
            fill = blend(fill, lighten(base, HOVER_LIFT), hover_t);
        }
        if select_t > 0.0 {
            fill = blend(fill, lighten(base, SELECT_LIFT), select_t);
        }
        painter.rect_filled(tile.screen, egui::CornerRadius::ZERO, fill);
        painter.rect_stroke(
            tile.screen,
            egui::CornerRadius::ZERO,
            Stroke::new(BORDER, BG_WINDOW),
            StrokeKind::Inside,
        );
        if select_t > 0.0 {
            painter.rect_stroke(
                tile.screen,
                egui::CornerRadius::ZERO,
                Stroke::new(2.0_f32, with_alpha(ACCENT, select_t)),
                StrokeKind::Inside,
            );
        }

        // 标签严格裁剪在框内，避免溢出到相邻格子。
        if tile.screen.width() > 30.0 && tile.screen.height() > 14.0 {
            let text_painter = painter.with_clip_rect(tile.screen.shrink(BORDER));
            let max_width = (tile.screen.width() - SPACE_XS * 2.0).max(4.0);
            let label = truncate_to_width(&node.name, max_width, FONT_SMALL);
            if !label.is_empty() {
                text_painter.text(
                    tile.screen.min + egui::vec2(SPACE_XS, SPACE_XS),
                    Align2::LEFT_TOP,
                    label,
                    FontId::proportional(FONT_SMALL),
                    TEXT,
                );
            }
            if !tile.expanded && tile.screen.height() > 32.0 {
                let size_label = truncate_to_width(&format_size(node.size), max_width, FONT_SMALL);
                if !size_label.is_empty() {
                    text_painter.text(
                        tile.screen.min + egui::vec2(SPACE_XS, SPACE_XS + FONT_SMALL + 2.0),
                        Align2::LEFT_TOP,
                        size_label,
                        FontId::proportional(FONT_SMALL),
                        MUTED,
                    );
                }
            }
        }
    }

    if response.double_clicked() {
        return (*hovered).map(|id| Interaction { id, double: true });
    }
    if response.clicked() {
        return (*hovered).map(|id| Interaction { id, double: false });
    }
    None
}

/// 计算同层条目的面积权重。
///
/// 默认按真实字节数占比；当同层条目不少于 [`CAP_MIN_ITEMS`] 时，
/// 真实占比超过 [`FILE_MAX_SHARE`] 的单个文件会被封顶到该比例，
/// 释放出的面积按原始大小分给其余条目，避免一个 exe/zip 铺满整屏。
/// 目录不设上限；条目太少时退回真实比例，防止大小关系被颠倒。
fn area_weights(tree: &Tree, parent: u32) -> Vec<(u32, f64)> {
    let items: Vec<(u32, f64, bool)> = tree
        .sorted_children(parent)
        .into_iter()
        .filter_map(|id| {
            let node = tree.node(id);
            (node.size > 0).then_some((id, node.size as f64, node.kind == NodeKind::File))
        })
        .collect();
    let count = items.len();
    let proportional = |items: Vec<(u32, f64, bool)>| -> Vec<(u32, f64)> {
        items.into_iter().map(|(id, size, _)| (id, size)).collect()
    };
    if count < CAP_MIN_ITEMS {
        return proportional(items);
    }

    let total: f64 = items.iter().map(|(_, size, _)| *size).sum();
    let capped: Vec<bool> = items
        .iter()
        .map(|(_, size, is_file)| *is_file && *size / total > FILE_MAX_SHARE)
        .collect();
    let capped_count = capped.iter().filter(|flag| **flag).count();
    if capped_count == 0 || capped_count == count {
        return proportional(items);
    }
    let free_share = 1.0 - capped_count as f64 * FILE_MAX_SHARE;
    let free_total: f64 = items
        .iter()
        .enumerate()
        .filter(|(index, _)| !capped[*index])
        .map(|(_, (_, size, _))| *size)
        .sum();
    if free_total <= 0.0 {
        return proportional(items);
    }
    items
        .into_iter()
        .enumerate()
        .map(|(index, (id, size, _))| {
            let weight = if capped[index] {
                FILE_MAX_SHARE * total
            } else {
                size / free_total * free_share * total
            };
            (id, weight)
        })
        .collect()
}

/// 递归布局一个节点：先登记自身，再在框内展开子项。
fn push_tile(tree: &Tree, id: u32, rect: Rect, depth: usize, out: &mut Vec<Tile>) {
    if rect.width() < MIN_VISIBLE || rect.height() < MIN_VISIBLE {
        return;
    }

    let node = tree.node(id);
    let items: Vec<(u32, f64)> = if node.kind == NodeKind::Dir {
        area_weights(tree, id)
    } else {
        Vec::new()
    };
    let expanded = !items.is_empty();

    out.push(Tile {
        id,
        rect,
        screen: rect,
        depth,
        expanded,
    });

    if !expanded {
        return;
    }

    let inner = Rect::from_min_max(
        rect.min + egui::vec2(BORDER, BORDER + TITLE_H),
        rect.max - egui::vec2(BORDER, BORDER),
    );
    if inner.width() < MIN_VISIBLE || inner.height() < MIN_VISIBLE {
        return;
    }
    for (child, layout) in disk_core::squarify(&items, inner.width() as f64, inner.height() as f64)
    {
        let child_rect = Rect::from_min_max(
            inner.min + egui::vec2(layout.x as f32, layout.y as f32),
            inner.min + egui::vec2((layout.x + layout.w) as f32, (layout.y + layout.h) as f32),
        );
        push_tile(tree, child, child_rect, depth + 1, out);
    }
}

/// 按估算文本宽度截断：ASCII 约 0.55 字宽，其他字符（含中文）按 1 字宽。
fn truncate_to_width(text: &str, max_width: f32, font_size: f32) -> String {
    let mut out = String::new();
    let mut width = 0.0;
    for ch in text.chars() {
        let char_width = if ch.is_ascii() {
            font_size * 0.55
        } else {
            font_size
        };
        if width + char_width > max_width {
            if !out.is_empty() {
                out.push('…');
            }
            return out;
        }
        out.push(ch);
        width += char_width;
    }
    out
}

fn lighten(color: Color32, amount: u8) -> Color32 {
    Color32::from_rgb(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
    )
}

/// 在两色之间按 `t` 线性插值，用于 hover / selected 的过渡。
fn blend(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (from.r() as f32 + (to.r() as f32 - from.r() as f32) * t) as u8,
        (from.g() as f32 + (to.g() as f32 - from.g() as f32) * t) as u8,
        (from.b() as f32 + (to.b() as f32 - from.b() as f32) * t) as u8,
    )
}

fn category_color(name: &str) -> Color32 {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    let category = match extension.as_str() {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "ts" | "mpg" => 0,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "svg"
        | "ico" => 1,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" => 2,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "iso" => 3,
        "rs" | "py" | "js" | "java" | "c" | "cpp" | "h" | "go" | "json" | "toml" | "yaml"
        | "html" | "css" | "sh" | "bat" | "ps1" => 4,
        "doc" | "docx" | "pdf" | "txt" | "md" | "xls" | "xlsx" | "ppt" | "pptx" | "csv" => 5,
        _ => 6,
    };
    const COLORS: [Color32; 7] = [
        Color32::from_rgb(58, 88, 134),  // 视频
        Color32::from_rgb(42, 108, 101), // 图片
        Color32::from_rgb(92, 70, 127),  // 音频
        Color32::from_rgb(126, 84, 46),  // 压缩包
        Color32::from_rgb(50, 100, 70),  // 代码
        Color32::from_rgb(116, 98, 48),  // 文档
        Color32::from_rgb(84, 90, 101),  // 其他
    ];
    COLORS[category]
}
