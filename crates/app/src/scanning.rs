//! The scan screen: live counters and the items under the root as they fill in, like dua
//! (spec §2.6).

use crate::theme::MONO;
use crate::view::{ROW_HEIGHT, RsDisk};
use gpui_kit::*;
use rsdisk_core::format;
use rsdisk_core::scan::Progress;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

/// The largest items under the root the scan screen lists; the rest are counted.
const TOPS_SHOWN: usize = 12;
const BAR_WIDTH: f32 = 80.;

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
        // Fixed widths, so the columns don't shift as the numbers grow.
        let stat = |label: &str, value: String| {
            div()
                .w(px(160.))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_color(rgb(t.dim)).child(label.to_string()))
                .child(div().font_family(MONO).text_size(px(22.)).child(value))
        };
        let total = progress.bytes.load(Relaxed).max(1);
        let mut tops = progress.tops();
        tops.sort_by_key(|t| std::cmp::Reverse(t.bytes));
        let more = tops.len().saturating_sub(TOPS_SHOWN);
        let mut list = div().w(px(720.)).flex().flex_col();
        for top in tops.iter().take(TOPS_SHOWN) {
            let share = top.bytes as f32 / total as f32;
            let name = if top.dir {
                format!("{}/", top.name)
            } else {
                top.name.to_string()
            };
            list = list.child(
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(
                        div()
                            .w(px(BAR_WIDTH))
                            .h(px(6.))
                            .rounded_sm()
                            .bg(rgb(t.faint))
                            .child(
                                div()
                                    .h_full()
                                    .rounded_sm()
                                    .w(px(BAR_WIDTH * share.clamp(0., 1.)))
                                    .bg(rgb(t.accent)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .child(format::bytes(top.bytes)),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .text_color(rgb(t.dim))
                            .child(count(top.files)),
                    ),
            );
        }
        if more > 0 {
            list = list.child(
                div()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .text_color(rgb(t.dim))
                    .child(format!("and {} more", count(more as u64))),
            );
        }
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
                    .gap_4()
                    .child(stat("Size", format::bytes(progress.bytes.load(Relaxed))))
                    .child(stat("Files", count(progress.files.load(Relaxed))))
                    .child(stat("Folders", count(progress.dirs.load(Relaxed))))
                    .child(stat(
                        "Time",
                        format!("{:.1} s", started.elapsed().as_secs_f32()),
                    )),
            )
            .child(list)
            .child(
                div()
                    .w(px(640.))
                    .text_color(rgb(t.dim))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .text_center()
                    // Cut from the start: the end of a path says what it is.
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
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
