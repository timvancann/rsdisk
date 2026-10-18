//! The browsing screen: header, the sorted list, and the pane plan 04 fills (spec §3.8).

use crate::browser::Browser;
use crate::scanning::count;
use crate::view::{ROW_HEIGHT, RsDisk};
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::tree::flag;
use std::ops::Range;

const BAR_WIDTH: f32 = 80.;

const HELP: &[(&str, &str)] = &[
    ("j / k", "down / up"),
    ("l / Enter", "open folder"),
    ("h / Backspace", "up a folder"),
    ("gg / G", "top / bottom"),
    ("Ctrl-d / Ctrl-u", "half a page down / up"),
    ("s", "sort by size, name, files"),
    ("Space", "mark / unmark, then down"),
    ("u", "clear marks"),
    ("d", "review marked and move to the Trash"),
    ("v", "treemap / sunburst"),
    ("a", "colour by kind / age"),
    ("o", "reveal in Finder"),
    ("?", "this help"),
    ("q", "quit"),
];

impl RsDisk {
    pub fn render_browser(
        &self,
        b: &Browser,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let rows = b.rows().len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_header(b, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div().w(relative(0.55)).h_full().child(
                            uniform_list(
                                "rows",
                                rows,
                                cx.processor(|view, range: Range<usize>, _window, cx| {
                                    view.render_rows(range, cx)
                                }),
                            )
                            .track_scroll(&self.scroll)
                            .size_full(),
                        ),
                    )
                    .child(self.render_visual(b, cx)),
            )
            .children(self.render_marked_pane())
            .when(rows == 0, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(90.))
                        .left(px(16.))
                        .text_color(rgb(t.dim))
                        .child("This folder is empty"),
                )
            })
            .when(self.help, |d| d.child(self.render_help()))
            .children(self.render_dialog())
    }

    fn render_header(&self, b: &Browser, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let tree = b.tree();
        let result = b.result();
        let root = tree.node(tree.root());
        let mut crumbs = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .font_family("Menlo");
        let chain = b.breadcrumb();
        let last = chain.len() - 1;
        for (i, id) in chain.into_iter().enumerate() {
            if i > 0 {
                crumbs = crumbs.child(div().text_color(rgb(t.dim)).child("/"));
            }
            let name = tree.node(id).name.to_string();
            crumbs = crumbs.child(
                div()
                    .id(("crumb", id as usize))
                    .text_color(rgb(if i == last { t.text } else { t.dim }))
                    .when(i == last, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |view, _: &ClickEvent, _, cx| view.click_crumb(id, cx)),
                    )
                    .child(name),
            );
        }

        let errors = result.error_count();
        let at_least = if errors > 0 { "at least " } else { "" };
        let mut totals = vec![
            format!("{at_least}{}", format::bytes(root.size)),
            format!("{} files", count(root.files as u64)),
            format!("sorted by {}", b.sort().label()),
        ];
        if result.cancelled {
            totals.push("scan stopped early".into());
        }
        if self.locating() {
            totals.push(format!("{errors} unreadable, finding which"));
        } else if errors > 0 {
            totals.push(format!("{errors} unreadable"));
        }
        let library = self.home.join("Library");
        let needs_access = result.errors.iter().any(|e| e.path.starts_with(&library));

        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(rgb(t.faint))
            .child(crumbs)
            .child(div().text_color(rgb(t.dim)).child(totals.join(", ")))
            .when(needs_access, |d| {
                d.child(div().text_color(rgb(t.accent)).child(
                    "Parts of ~/Library are unreadable. Give your terminal Full Disk Access in \
                     System Settings > Privacy & Security for a complete scan.",
                ))
            })
    }

    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(b) = self.browser() else {
            return Vec::new();
        };
        let t = &self.theme;
        let tree = b.tree();
        let folder_size = tree.node(b.folder()).size.max(1);
        let selected = b.selected();
        b.rows()[range]
            .iter()
            .map(|&id| {
                let n = tree.node(id);
                let share = n.size as f32 / folder_size as f32;
                let colour = if n.is_dir() { t.accent } else { t.kind(n.kind) };
                let name = if n.is_dir() {
                    format!("{}/", n.name)
                } else {
                    n.name.to_string()
                };
                let marked = self.marks.covered(tree, id);
                let marker = if marked {
                    Some("marked")
                } else {
                    marker(n.flags)
                };
                div()
                    .id(("row", id as usize))
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .when(selected == Some(id), |d| d.bg(rgb(t.raised)))
                    .on_click(cx.listener(move |view, ev: &ClickEvent, _, cx| {
                        view.click_row(id, ev.click_count() >= 2, cx)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(marked, |d| {
                                d.text_color(rgb(t.accent))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(name),
                    )
                    .children(marker.map(|m| {
                        div()
                            .text_color(rgb(if marked { t.accent } else { t.dim }))
                            .child(m)
                    }))
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
                                    .bg(rgb(colour)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_end()
                            .font_family("Menlo")
                            .child(format::bytes(n.size)),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_end()
                            .text_color(rgb(t.dim))
                            .child(count(n.files as u64)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_help(&self) -> impl IntoElement {
        let t = &self.theme;
        let mut list = div().flex().flex_col().gap_1();
        for (keys, what) in HELP {
            list = list.child(
                div()
                    .flex()
                    .gap_4()
                    .child(div().w(px(140.)).font_family("Menlo").child(*keys))
                    .child(div().text_color(rgb(t.dim)).child(*what)),
            );
        }
        div()
            .absolute()
            .size_full()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .p_5()
                    .rounded_lg()
                    .bg(rgb(t.surface))
                    .border_1()
                    .border_color(rgb(t.faint))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Keys"))
                    .child(list)
                    .child(div().text_color(rgb(t.dim)).child("? or Esc closes this")),
            )
    }
}

fn marker(flags: u8) -> Option<&'static str> {
    if flags & flag::UNREADABLE != 0 {
        Some("unreadable")
    } else if flags & flag::NOT_SCANNED != 0 {
        Some("not scanned")
    } else if flags & flag::OTHER_VOLUME != 0 {
        Some("other volume")
    } else {
        None
    }
}
