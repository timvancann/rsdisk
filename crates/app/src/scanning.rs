//! The scan screen: live counters while the walk runs (spec §2.6).

use crate::view::RsDisk;
use gpui_kit::*;
use rsdisk_core::format;
use rsdisk_core::scan::Progress;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

/// `3906896` as `3,906,896`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl RsDisk {
    pub fn render_scanning(&self, progress: &Progress, started: Instant) -> impl IntoElement {
        let t = &self.theme;
        let stat = |label: &str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgb(t.dim)).child(label.to_string()))
                .child(div().font_family("Menlo").text_size(px(22.)).child(value))
        };
        let latest = progress.latest();
        let shown = latest
            .strip_prefix(&self.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| latest.display().to_string());
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .child(
                div()
                    .text_size(px(16.))
                    .child(format!("Scanning {}", self.root.display())),
            )
            .child(
                div()
                    .flex()
                    .gap_10()
                    .child(stat("Size", format::bytes(progress.bytes.load(Relaxed))))
                    .child(stat("Files", count(progress.files.load(Relaxed))))
                    .child(stat("Folders", count(progress.dirs.load(Relaxed))))
                    .child(stat(
                        "Time",
                        format!("{:.1} s", started.elapsed().as_secs_f32()),
                    )),
            )
            .child(
                div()
                    .w(px(640.))
                    .text_color(rgb(t.dim))
                    .font_family("Menlo")
                    .text_size(px(12.))
                    .text_center()
                    .truncate()
                    .child(shown),
            )
            .child(
                div()
                    .text_color(rgb(t.dim))
                    .child("Esc stops the scan and shows what it found so far"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn thousands_get_commas() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(3_906_896), "3,906,896");
    }
}
