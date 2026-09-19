//! Equal-size metric tiles with line charts for the KPI board.

use gpui::{
    canvas, div, point, prelude::*, px, rgb, Bounds, IntoElement, PathBuilder, Pixels,
    SharedString, Window,
};

const TILE_MIN_H: f32 = 200.0;
const CHART_H: f32 = 110.0;

/// One board cell: title, optional note, line chart, latest value.
pub fn metric_tile(
    title: &str,
    unit: &str,
    values: &[f32],
    color: u32,
    note: Option<&str>,
) -> impl IntoElement {
    let title = SharedString::from(title.to_owned());
    let unit = SharedString::from(unit.to_owned());
    let note = note.map(|n| SharedString::from(n.to_owned()));
    let series = values.to_vec();
    let latest = series.last().copied();
    let latest_label = SharedString::from(match latest {
        Some(v) => format!("{v:.0}"),
        None => "-".into(),
    });

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x2a3a2a))
        .bg(rgb(0x121812))
        .min_h(px(TILE_MIN_H))
        .flex_1()
        .min_w(px(0.0))
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0xb8d4b8))
                        .child(title),
                )
                .child(div().text_xs().text_color(rgb(0x6a8a6a)).child(unit)),
        )
        .when_some(note, |this, n| {
            this.child(div().text_xs().text_color(rgb(0xe0b040)).child(n))
        })
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(color))
                .child(latest_label),
        )
        .child({
            let paint = series.clone();
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_grid(window, bounds);
                    paint_line(window, bounds, &paint, color);
                },
            )
            .w_full()
            .h(px(CHART_H))
            .flex_1()
        })
}

fn paint_grid(window: &mut Window, bounds: Bounds<Pixels>) {
    let mut grid = PathBuilder::stroke(px(1.0));
    let rows = 4;
    for i in 0..=rows {
        let y = bounds.origin.y + bounds.size.height * (i as f32 / rows as f32);
        grid.move_to(point(bounds.origin.x, y));
        grid.line_to(point(bounds.origin.x + bounds.size.width, y));
    }
    if let Ok(path) = grid.build() {
        window.paint_path(path, rgb(0x1e2e1e));
    }
}

fn paint_line(window: &mut Window, bounds: Bounds<Pixels>, values: &[f32], color: u32) {
    if values.is_empty() {
        return;
    }
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min).abs().max(1.0);
    let w = bounds.size.width;
    let h = bounds.size.height;
    let n = (values.len().saturating_sub(1).max(1)) as f32;

    let mut stroke = PathBuilder::stroke(px(2.0));
    let mut fill = PathBuilder::fill();
    fill.move_to(point(bounds.origin.x, bounds.origin.y + h));

    for (i, v) in values.iter().enumerate() {
        let t = (*v - min) / span;
        let x = if values.len() == 1 {
            bounds.origin.x + w * 0.5
        } else {
            bounds.origin.x + w * (i as f32 / n)
        };
        let y = bounds.origin.y + h * (1.0 - t);
        let p = point(x, y);
        if i == 0 {
            stroke.move_to(p);
        } else {
            stroke.line_to(p);
        }
        fill.line_to(p);
    }
    fill.line_to(point(bounds.origin.x + w, bounds.origin.y + h));
    fill.close();

    if let Ok(path) = fill.build() {
        window.paint_path(path, rgb(darken(color, 0.55)));
    }
    if let Ok(path) = stroke.build() {
        window.paint_path(path, rgb(color));
    }
}

fn darken(color: u32, factor: f32) -> u32 {
    let r = ((color >> 16) & 0xff) as f32 * factor;
    let g = ((color >> 8) & 0xff) as f32 * factor;
    let b = (color & 0xff) as f32 * factor;
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
