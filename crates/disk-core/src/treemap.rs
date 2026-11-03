//! Squarified treemap 布局（Bruls 等，2000）。
//!
//! 输入为 (节点编号, 权重)，输出为铺满给定矩形的矩形列表，面积与权重成正比。

const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn area(&self) -> f64 {
        self.w * self.h
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x <= self.right() && y <= self.bottom()
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        let overlap_w = self.right().min(other.right()) - self.x.max(other.x);
        let overlap_h = self.bottom().min(other.bottom()) - self.y.max(other.y);
        overlap_w > EPS && overlap_h > EPS
    }
}

/// 计算 squarified 布局。权重 <= 0 或非有限的项会被忽略。
pub fn squarify(items: &[(u32, f64)], width: f64, height: f64) -> Vec<(u32, Rect)> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }

    let mut data: Vec<(u32, f64)> = items
        .iter()
        .copied()
        .filter(|(_, value)| value.is_finite() && *value > 0.0)
        .collect();
    if data.is_empty() {
        return Vec::new();
    }
    data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let total: f64 = data.iter().map(|(_, value)| *value).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let scale = width * height / total;
    let areas: Vec<(u32, f64)> = data
        .iter()
        .map(|(id, value)| (*id, *value * scale))
        .collect();

    let mut out: Vec<(u32, Rect)> = Vec::with_capacity(areas.len());
    let mut free = Rect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
    };
    let mut start = 0usize;

    while start < areas.len() {
        if free.w <= EPS || free.h <= EPS {
            break;
        }
        let short = free.w.min(free.h);

        let mut end = start + 1;
        let mut best = worst_ratio(&areas[start..end], short);
        while end < areas.len() {
            let trial = worst_ratio(&areas[start..=end], short);
            if trial <= best {
                best = trial;
                end += 1;
            } else {
                break;
            }
        }

        let row = &areas[start..end];
        let row_sum: f64 = row.iter().map(|(_, area)| *area).sum();
        if row_sum <= 0.0 {
            start = end;
            continue;
        }

        if free.w >= free.h {
            let col_w = (row_sum / free.h).min(free.w);
            let mut y = free.y;
            for (index, (id, area)) in row.iter().enumerate() {
                let h = if index + 1 == row.len() {
                    (free.y + free.h - y).max(0.0)
                } else {
                    area / col_w
                };
                out.push((
                    *id,
                    Rect {
                        x: free.x,
                        y,
                        w: col_w,
                        h,
                    },
                ));
                y += h;
            }
            free.x += col_w;
            free.w = (free.w - col_w).max(0.0);
        } else {
            let row_h = (row_sum / free.w).min(free.h);
            let mut x = free.x;
            for (index, (id, area)) in row.iter().enumerate() {
                let w = if index + 1 == row.len() {
                    (free.x + free.w - x).max(0.0)
                } else {
                    area / row_h
                };
                out.push((
                    *id,
                    Rect {
                        x,
                        y: free.y,
                        w,
                        h: row_h,
                    },
                ));
                x += w;
            }
            free.y += row_h;
            free.h = (free.h - row_h).max(0.0);
        }

        start = end;
    }

    out
}

/// 最差长宽比，越小越接近正方形。
fn worst_ratio(row: &[(u32, f64)], short: f64) -> f64 {
    let mut sum = 0.0;
    let mut max = 0.0f64;
    let mut min = f64::MAX;
    for (_, area) in row {
        sum += *area;
        max = max.max(*area);
        min = min.min(*area);
    }
    if sum <= 0.0 || min <= 0.0 || short <= 0.0 {
        return f64::INFINITY;
    }
    let short2 = short * short;
    let sum2 = sum * sum;
    (short2 * max / sum2).max(sum2 / (short2 * min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(u32, f64)> {
        vec![
            (0, 6.0),
            (1, 6.0),
            (2, 4.0),
            (3, 3.0),
            (4, 2.0),
            (5, 2.0),
            (6, 1.0),
        ]
    }

    #[test]
    fn places_every_item_inside_bounds() {
        let items = sample();
        let out = squarify(&items, 800.0, 500.0);
        assert_eq!(out.len(), items.len());
        for (_, rect) in &out {
            assert!(rect.x >= -1e-6 && rect.y >= -1e-6);
            assert!(rect.right() <= 800.0 + 1e-6);
            assert!(rect.bottom() <= 500.0 + 1e-6);
            assert!(rect.w > 0.0 && rect.h > 0.0);
        }
    }

    #[test]
    fn areas_are_proportional() {
        let items = sample();
        let total: f64 = items.iter().map(|(_, v)| *v).sum();
        let out = squarify(&items, 640.0, 400.0);
        let canvas = 640.0 * 400.0;
        for (id, rect) in &out {
            let value = items.iter().find(|(i, _)| i == id).unwrap().1;
            let expected = canvas * value / total;
            assert!(
                (rect.area() - expected).abs() <= 1e-3 * canvas,
                "id {} area {} expected {}",
                id,
                rect.area(),
                expected
            );
        }
    }

    #[test]
    fn rectangles_do_not_overlap_and_fill_canvas() {
        let out = squarify(&sample(), 600.0, 300.0);
        let mut area_sum = 0.0;
        for i in 0..out.len() {
            area_sum += out[i].1.area();
            for j in (i + 1)..out.len() {
                assert!(!out[i].1.intersects(&out[j].1), "rect {} overlaps {}", i, j);
            }
        }
        assert!((area_sum - 600.0 * 300.0).abs() < 1e-3);
    }

    #[test]
    fn ignores_invalid_values() {
        assert!(squarify(&[], 100.0, 100.0).is_empty());
        assert!(squarify(&[(0, 0.0), (1, -2.0)], 100.0, 100.0).is_empty());
        assert!(squarify(&[(0, 1.0)], 0.0, 100.0).is_empty());
        let out = squarify(&[(0, 1.0), (1, 0.0), (2, f64::NAN)], 100.0, 100.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 0);
    }

    #[test]
    fn single_item_fills_canvas() {
        let out = squarify(&[(7, 3.0)], 250.0, 120.0);
        assert_eq!(out.len(), 1);
        let rect = out[0].1;
        assert!((rect.w - 250.0).abs() < 1e-9);
        assert!((rect.h - 120.0).abs() < 1e-9);
    }
}
