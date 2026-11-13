//! The header while the scan runs (spec §2.6): live counters over the browser's own list and
//! visual, which show the items under the root as they fill in, like dua.

use crate::theme::MONO;
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
    pub fn render_scan_header(&self, progress: &Progress, started: Instant) -> impl IntoElement {
        let t = &self.theme;
        let latest = progress.latest();
        let shown = latest
            .strip_prefix(&self.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| latest.display().to_string());
        let counters = format!(
            "Scanning: {}, {} files, {} folders, {:.1} s. Esc stops it and shows what it found \
             so far.",
            format::bytes(progress.bytes.load(Relaxed)),
            count(progress.files.load(Relaxed)),
            count(progress.dirs.load(Relaxed)),
            started.elapsed().as_secs_f32()
        );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .pt_3()
            .pb_2p5()
            .border_b_1()
            .border_color(rgb(t.faint))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.root.display().to_string()),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(t.dim))
                    .child(counters),
            )
            .child(
                div()
                    .text_color(rgb(t.dim))
                    .font_family(MONO)
                    .text_size(px(12.))
                    // Cut from the start: the end of a path says what it is.
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    .child(shown),
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
