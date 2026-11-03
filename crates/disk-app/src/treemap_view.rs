//! 矩形树图绘制。

use disk_core::{format_size, NodeKind, Tree};
use eframe::egui::{self, Align2, Color32, FontId, Id, Sense, Stroke, StrokeKind};

use crate::app::{
    with_alpha, ACCENT, BG_WINDOW, FONT_BODY, FONT_SMALL, MUTED, SPACE_SM, SPACE_XS, TEXT,
};

const DIR_COLORS: [Color32; 3] = [
    Color32::from_rgb(54, 64, 82),
    Color32::from_rgb(48, 71, 88),
    Color32::from_rgb(61, 72, 95),
];
const MAX_TILES: usize = 400;
const HOVER_LIFT: u8 = 24;
const SELECT_LIFT: u8 = 38;

/// 绘制当前目录的树图，返回被点击的节点编号。
pub fn draw(
    ui: &mut egui::Ui,
    tree: &Tree,
    current: u32,
    selected: Option<u32>,
    hovered: &mut Option<u32>,
) -> Option<u32> {
    *hovered = None;

    let available = ui.available_size();
    if available.x < 40.0 || available.y < 40.0 {
        return None;
    }
    let (response, painter) = ui.allocate_painter(available, Sense::click());
    let canvas = response.rect;
    painter.rect_filled(canvas, egui::CornerRadius::ZERO, BG_WINDOW);

    let mut items: Vec<(u32, f64)> = tree
        .sorted_children(current)
        .into_iter()
        .filter_map(|id| {
            let size = tree.node(id).size;
            if size > 0 {
                Some((id, size as f64))
            } else {
                None
            }
        })
        .collect();
    let truncated = items.len() > MAX_TILES;
    items.truncate(MAX_TILES);

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

    let layout = disk_core::squarify(&items, canvas.width() as f64, canvas.height() as f64);
    let pointer = response.hover_pos();
    if let Some(position) = pointer {
        for (id, rect) in &layout {
            let tile = tile_rect(canvas, rect);
            if tile.contains(position) {
                *hovered = Some(*id);
            }
        }
    }

    let ctx = ui.ctx().clone();
    for (id, rect) in &layout {
        let tile = tile_rect(canvas, rect);
        if tile.width() < 1.0 || tile.height() < 1.0 {
            continue;
        }
        let node = tree.node(*id);
        let base = match node.kind {
            NodeKind::Dir => DIR_COLORS[*id as usize % DIR_COLORS.len()],
            NodeKind::File => category_color(&node.name),
        };
        let is_hovered = *hovered == Some(*id);
        let is_selected = selected == Some(*id);
        // hover / selected 都走 150ms 动画，鼠标扫过或切换选中时不再瞬变。
        let hover_t = ctx.animate_bool(Id::new(("tile_hover", *id)), is_hovered);
        let select_t = ctx.animate_bool(Id::new(("tile_select", *id)), is_selected);
        let mut fill = base;
        if hover_t > 0.0 {
            fill = blend(fill, lighten(base, HOVER_LIFT), hover_t);
        }
        if select_t > 0.0 {
            fill = blend(fill, lighten(base, SELECT_LIFT), select_t);
        }
        painter.rect_filled(tile, egui::CornerRadius::ZERO, fill);
        painter.rect_stroke(
            tile,
            egui::CornerRadius::ZERO,
            Stroke::new(1.0_f32, BG_WINDOW),
            StrokeKind::Inside,
        );
        if select_t > 0.0 {
            painter.rect_stroke(
                tile,
                egui::CornerRadius::ZERO,
                Stroke::new(2.0_f32, with_alpha(ACCENT, select_t)),
                StrokeKind::Inside,
            );
        }

        if tile.width() > 42.0 && tile.height() > 16.0 {
            let font_size = if tile.width() > 170.0 && tile.height() > 60.0 {
                FONT_BODY
            } else {
                FONT_SMALL
            };
            let char_width = font_size * 0.62;
            let max_chars = (((tile.width() - 8.0) / char_width).floor() as usize).max(1);
            let label = truncate(&node.name, max_chars);
            painter.text(
                tile.min + egui::vec2(SPACE_XS, SPACE_XS),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(font_size),
                TEXT,
            );
            if tile.height() > 36.0 {
                painter.text(
                    tile.min + egui::vec2(SPACE_XS, SPACE_XS + font_size + SPACE_XS),
                    Align2::LEFT_TOP,
                    format_size(node.size),
                    FontId::proportional(FONT_SMALL),
                    MUTED,
                );
            }
        }
    }

    if truncated {
        painter.text(
            canvas.left_bottom() + egui::vec2(SPACE_SM, -SPACE_SM),
            Align2::LEFT_BOTTOM,
            format!("仅显示前 {} 个直属条目，完整清单请导出 CSV", MAX_TILES),
            FontId::proportional(FONT_SMALL),
            MUTED,
        );
    }

    if response.clicked() {
        return *hovered;
    }
    None
}

fn tile_rect(canvas: egui::Rect, rect: &disk_core::treemap::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(canvas.left() + rect.x as f32, canvas.top() + rect.y as f32),
        egui::pos2(
            canvas.left() + (rect.x + rect.w) as f32,
            canvas.top() + (rect.y + rect.h) as f32,
        ),
    )
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
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
