//! Canvas line and bar charts for the control-room dashboard.

use gpui::{
    canvas, div, point, prelude::*, px, rgb, Bounds, IntoElement, PathBuilder, Pixels,
    SharedString, Window,
};

/// Multi-series line chart panel (control-room style).
pub fn line_panel(title: &str, series: &[(&str, &[f32], u32)], unit: &str) -> impl IntoElement {
    let title = SharedString::from(title.to_owned());
    let unit = SharedString::from(unit.to_owned());
    let owned: Vec<(String, Vec<f32>, u32)> = series
        .iter()
        .map(|(l, vals, c)| ((*l).to_owned(), vals.to_vec(), *c))
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x2a3a2a))
        .bg(rgb(0x121812))
        .min_h(px(220.0))
        .flex_1()
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0xb8d4b8))
                        .child(title),
                )
                .child(div().text_xs().text_color(rgb(0x6a8a6a)).child(unit)),
        )
        .child(
            div()
                .flex()
                .gap_3()
                .children(owned.iter().map(|(label, _, color)| {
                    let label = SharedString::from(label.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().w(px(10.0)).h(px(3.0)).bg(rgb(*color)))
                        .child(div().text_xs().text_color(rgb(0x8aaa8a)).child(label))
                })),
        )
        .child({
            let paint_data = owned.clone();
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_grid(window, bounds);
                    for (_, values, color) in &paint_data {
                        paint_line(window, bounds, values, *color);
                    }
                },
            )
            .w_full()
            .h(px(140.0))
        })
        .child(latest_readout(&owned))
}

fn latest_readout(owned: &[(String, Vec<f32>, u32)]) -> impl IntoElement {
    div()
        .flex()
        .gap_4()
        .children(owned.iter().map(|(label, vals, color)| {
            let last = vals.last().copied().unwrap_or(0.0);
            let text = SharedString::from(format!("{label}  {last:.0}"));
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(*color))
                .child(text)
        }))
}

/// Horizontal readiness bars (percent series as single current values).
pub fn bar_panel(title: &str, bars: &[(&str, f32, u32)]) -> impl IntoElement {
    let title = SharedString::from(title.to_owned());
    let owned: Vec<(String, f32, u32)> = bars
        .iter()
        .map(|(l, v, c)| ((*l).to_owned(), *v, *c))
        .collect();

    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(0x2a3a2a))
        .bg(rgb(0x121812))
        .min_h(px(220.0))
        .flex_1()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(0xb8d4b8))
                .child(title),
        )
        .children(owned.into_iter().map(|(label, value, color)| {
            let pct = value.clamp(0.0, 100.0);
            let label_s = SharedString::from(format!("{label}  {pct:.0}%"));
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(0x8aaa8a)).child(label_s))
                .child(
                    canvas(
                        move |bounds, _, _| bounds.size.width,
                        move |bounds, _width, window, _| {
                            paint_bar(window, bounds, pct, color);
                        },
                    )
                    .w_full()
                    .h(px(18.0)),
                )
        }))
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
    if values.len() < 2 {
        return;
    }
    let min = values.iter().copied().fold(f32::INFINITY, f32::min);
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let span = (max - min).abs().max(1.0);
    let w = bounds.size.width;
    let h = bounds.size.height;
    let n = (values.len() - 1) as f32;

    let mut stroke = PathBuilder::stroke(px(2.0));
    let mut fill = PathBuilder::fill();
    fill.move_to(point(bounds.origin.x, bounds.origin.y + h));

    for (i, v) in values.iter().enumerate() {
        let t = (*v - min) / span;
        let x = bounds.origin.x + w * (i as f32 / n);
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

fn paint_bar(window: &mut Window, bounds: Bounds<Pixels>, pct: f32, color: u32) {
    let mut track = PathBuilder::fill();
    track.move_to(bounds.origin);
    track.line_to(point(bounds.origin.x + bounds.size.width, bounds.origin.y));
    track.line_to(point(
        bounds.origin.x + bounds.size.width,
        bounds.origin.y + bounds.size.height,
    ));
    track.line_to(point(bounds.origin.x, bounds.origin.y + bounds.size.height));
    track.close();
    if let Ok(path) = track.build() {
        window.paint_path(path, rgb(0x1a2a1a));
    }

    let fill_w = bounds.size.width * (pct / 100.0);
    let mut fill = PathBuilder::fill();
    fill.move_to(bounds.origin);
    fill.line_to(point(bounds.origin.x + fill_w, bounds.origin.y));
    fill.line_to(point(
        bounds.origin.x + fill_w,
        bounds.origin.y + bounds.size.height,
    ));
    fill.line_to(point(bounds.origin.x, bounds.origin.y + bounds.size.height));
    fill.close();
    if let Ok(path) = fill.build() {
        window.paint_path(path, rgb(color));
    }
}

fn darken(color: u32, factor: f32) -> u32 {
    let r = ((color >> 16) & 0xff) as f32 * factor;
    let g = ((color >> 8) & 0xff) as f32 * factor;
    let b = (color & 0xff) as f32 * factor;
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}
