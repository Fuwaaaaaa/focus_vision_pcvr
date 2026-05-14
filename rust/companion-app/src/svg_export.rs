//! SVG renderer for the stats history sparklines.
//!
//! Pure-function (egui-independent) so unit tests can drive it from any
//! shape of input. The companion UI calls `render()` and writes the
//! resulting string to a user-chosen path; the SVG is loaded by any image
//! viewer or browser.

use crate::stats_history::StatsHistory;

/// Output canvas size. 800x600 keeps line weights readable at typical
/// HiDPI zoom while still fitting in a slide deck or bug report screenshot.
pub const VIEWPORT_WIDTH: u32 = 800;
pub const VIEWPORT_HEIGHT: u32 = 600;

const SECTION_HEIGHT: u32 = 180;
const SECTION_GAP: u32 = 10;
const MARGIN_LEFT: u32 = 60;
const MARGIN_RIGHT: u32 = 20;
const MARGIN_TOP: u32 = 30;
const LINE_COLOR: &str = "#34D399"; // DESIGN.md accent
const AXIS_COLOR: &str = "#444";
const TEXT_COLOR: &str = "#aaa";
const BG_COLOR: &str = "#111114";

/// Render the stats history as an SVG document. Always returns valid SVG —
/// an empty history produces an axes-only chart rather than a missing
/// section, so users see *something* when they hit Export before any
/// stats have been collected.
pub fn render(history: &StatsHistory) -> String {
    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}">"#,
        w = VIEWPORT_WIDTH,
        h = VIEWPORT_HEIGHT
    ));
    svg.push_str(&format!(
        r#"<rect width="{w}" height="{h}" fill="{bg}" />"#,
        w = VIEWPORT_WIDTH,
        h = VIEWPORT_HEIGHT,
        bg = BG_COLOR
    ));
    svg.push_str(&format!(
        r#"<text x="{x}" y="20" fill="{tc}" font-family="sans-serif" font-size="14">Focus Vision PCVR — Stats (30s window)</text>"#,
        x = MARGIN_LEFT,
        tc = TEXT_COLOR
    ));

    // VecDeque doesn't expose a contiguous slice for a wrapped buffer, so
    // copy each series into a Vec once for the render pass. The buffer
    // caps at 30 samples so this is trivial.
    let latency: Vec<f32> = history.latency_ms.iter().copied().collect();
    let fps: Vec<f32> = history.fps.iter().copied().collect();
    let loss: Vec<f32> = history.packet_loss.iter().copied().collect();

    let sections: [(&str, &[f32], f32, f32); 3] = [
        ("Latency (ms)", &latency, 0.0, 60.0),
        ("FPS", &fps, 0.0, 120.0),
        ("Packet Loss (%)", &loss, 0.0, 10.0),
    ];

    for (i, (label, series, lo, hi)) in sections.iter().enumerate() {
        let y_offset = MARGIN_TOP + (i as u32) * (SECTION_HEIGHT + SECTION_GAP);
        svg.push_str(&render_section(label, series, *lo, *hi, y_offset));
    }

    svg.push_str("</svg>");
    svg
}

fn render_section(label: &str, series: &[f32], lo: f32, hi: f32, y_offset: u32) -> String {
    let mut s = String::new();
    let plot_width = VIEWPORT_WIDTH - MARGIN_LEFT - MARGIN_RIGHT;
    let plot_height = SECTION_HEIGHT - 20; // leave room for label

    // Label
    s.push_str(&format!(
        r#"<text x="{x}" y="{y}" fill="{tc}" font-family="sans-serif" font-size="12">{label}</text>"#,
        x = MARGIN_LEFT,
        y = y_offset + 12,
        tc = TEXT_COLOR,
        label = label
    ));

    let plot_top = y_offset + 20;
    let plot_bottom = plot_top + plot_height;

    // Y axis baseline + top tick
    s.push_str(&format!(
        r#"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{ac}" stroke-width="1" />"#,
        x1 = MARGIN_LEFT,
        y1 = plot_bottom,
        x2 = VIEWPORT_WIDTH - MARGIN_RIGHT,
        y2 = plot_bottom,
        ac = AXIS_COLOR
    ));
    s.push_str(&format!(
        r#"<text x="{x}" y="{y}" fill="{tc}" font-family="sans-serif" font-size="10" text-anchor="end">{val:.1}</text>"#,
        x = MARGIN_LEFT - 4,
        y = plot_top + 4,
        tc = TEXT_COLOR,
        val = hi
    ));
    s.push_str(&format!(
        r#"<text x="{x}" y="{y}" fill="{tc}" font-family="sans-serif" font-size="10" text-anchor="end">{val:.1}</text>"#,
        x = MARGIN_LEFT - 4,
        y = plot_bottom + 4,
        tc = TEXT_COLOR,
        val = lo
    ));

    // Polyline. Empty series → emit just the axes so the SVG stays
    // syntactically valid.
    if !series.is_empty() {
        let mut points = String::new();
        let span = (hi - lo).max(0.001);
        let n = series.len();
        for (i, v) in series.iter().enumerate() {
            // Guard against NaN/inf from upstream — clamp to range and
            // skip non-finite values rather than emit malformed numbers.
            if !v.is_finite() {
                continue;
            }
            let clamped = v.clamp(lo, hi);
            let x = MARGIN_LEFT as f32
                + (i as f32 / (n.max(2) - 1) as f32) * plot_width as f32;
            let y = plot_bottom as f32
                - ((clamped - lo) / span) * plot_height as f32;
            if !points.is_empty() {
                points.push(' ');
            }
            points.push_str(&format!("{:.1},{:.1}", x, y));
        }
        if !points.is_empty() {
            s.push_str(&format!(
                r#"<polyline fill="none" stroke="{lc}" stroke-width="1.5" points="{pts}" />"#,
                lc = LINE_COLOR,
                pts = points
            ));
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_history() -> StatsHistory {
        StatsHistory::new()
    }

    fn filled_history() -> StatsHistory {
        let mut h = StatsHistory::new();
        for i in 0..30 {
            h.push(13.0 + (i as f32) * 0.1, 90.0, 0.3);
        }
        h
    }

    #[test]
    fn renders_valid_svg_for_empty_history() {
        let svg = render(&empty_history());
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        // No polyline element when there are no points to plot.
        assert!(!svg.contains("polyline"));
    }

    #[test]
    fn renders_polyline_for_filled_history() {
        let svg = render(&filled_history());
        assert!(svg.contains("polyline"));
        assert!(svg.contains("Latency (ms)"));
        assert!(svg.contains("FPS"));
        assert!(svg.contains("Packet Loss (%)"));
    }

    #[test]
    fn renders_valid_svg_for_partial_history() {
        let mut h = StatsHistory::new();
        h.push(15.0, 89.0, 0.2);
        h.push(16.0, 90.0, 0.1);
        let svg = render(&h);
        assert!(svg.starts_with("<svg"));
        // Two-point series still emits a polyline.
        assert!(svg.contains("polyline"));
    }

    #[test]
    fn skips_non_finite_values_without_panic() {
        let mut h = StatsHistory::new();
        h.push(f32::NAN, 90.0, 0.3);
        h.push(15.0, f32::INFINITY, 0.3);
        h.push(15.0, 90.0, f32::NAN);
        // Render must succeed and emit valid SVG even with non-finite inputs.
        let svg = render(&h);
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        // Numeric output must not include "NaN" or "inf" — those would
        // make the SVG fail validation when opened in a viewer.
        assert!(!svg.contains("NaN"));
        assert!(!svg.contains("inf"));
    }

    #[test]
    fn svg_contains_brand_header() {
        let svg = render(&filled_history());
        assert!(svg.contains("Focus Vision PCVR"));
    }
}
