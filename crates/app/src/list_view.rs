//! The browsing screen: header, the sorted list, the selection's details, and the visual
//! (spec §3.8).

use crate::browser::Browser;
use crate::colour;
use crate::scanning::count;
use crate::theme::MONO;
use crate::view::{ROW_HEIGHT, RsDisk, grantee};
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::tree::flag;
use std::ops::Range;
use std::sync::atomic::Ordering::Relaxed;

const BAR_WIDTH: f32 = 80.;
/// Breadcrumbs beyond the root and this many of the last ones collapse to an ellipsis.
const CRUMBS_KEPT: usize = 3;

const HELP: &[(&str, &str)] = &[
    ("j / k", "down / up"),
    ("l / Enter", "open folder"),
    ("h / Backspace", "up a folder"),
    ("gg / G", "top / bottom"),
    ("Ctrl-d / Ctrl-u", "half a page down / up"),
    ("s", "sort by size, name, files"),
    ("Space", "mark / unmark, then down"),
    ("u", "clear marks"),
    ("/", "search the whole scan"),
    ("r", "rescan this folder"),
    ("c", "scan the selected cloud folder"),
    ("n / N", "next / previous search result"),
    ("e", "list the unreadable folders"),
    ("d", "review marked and move to the Trash"),
    ("v", "treemap / sunburst"),
    ("a", "colour by kind / age"),
    ("o", "reveal in Finder"),
    ("p", "preview with Quick Look"),
    ("Esc", "stop a rescan, close the preview"),
    ("?", "this help"),
    ("Cmd-,", "settings and permissions"),
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
        let list = if rows == 0 {
            div()
                .px_4()
                .py_3()
                .text_color(rgb(t.dim))
                .child("This folder is empty")
                .into_any_element()
        } else {
            uniform_list(
                "rows",
                rows,
                cx.processor(|view, range: Range<usize>, _window, cx| view.render_rows(range, cx)),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        };
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
                        div()
                            .w(relative(0.55))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(div().flex_1().min_h_0().child(list))
                            .children(self.render_details(b)),
                    )
                    .child(self.render_visual(b, cx)),
            )
            .children(self.render_marked_pane())
            .when(self.help, |d| d.child(self.render_help(cx)))
            .children(self.render_find(cx))
            .children(self.render_dialog(cx))
    }

    fn render_header(&self, b: &Browser, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let tree = b.tree();
        let result = b.result();
        let root = tree.node(tree.root());
        let mut crumbs = div()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .font_family(MONO);
        let chain = b.breadcrumb();
        let last = chain.len() - 1;
        // The root, an ellipsis, and the last few: one line however deep.
        let hidden = 1..chain.len().saturating_sub(CRUMBS_KEPT).max(1);
        for (i, id) in chain.into_iter().enumerate() {
            if hidden.contains(&i) {
                if i == hidden.start {
                    crumbs = crumbs
                        .child(div().text_color(rgb(t.dim)).child("/"))
                        .child(div().text_color(rgb(t.dim)).child("\u{2026}"));
                }
                continue;
            }
            if i > 0 {
                crumbs = crumbs.child(div().text_color(rgb(t.dim)).child("/"));
            }
            let name = tree.node(id).name.to_string();
            crumbs = crumbs.child(
                div()
                    .id(("crumb", id as usize))
                    .flex_shrink_0()
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
        let at_least = if errors > 0 || result.cancelled {
            "at least "
        } else {
            ""
        };
        let folder = tree.node(b.folder());
        let mut totals = vec![
            format!("{} files", count(folder.files as u64)),
            format!("sorted by {}", b.sort().label()),
        ];
        if b.folder() == tree.root() {
            totals.insert(0, format!("{at_least}{}", format::bytes(root.size)));
        } else {
            totals.insert(
                0,
                format!(
                    "{} of {at_least}{}",
                    format::bytes(folder.size),
                    format::bytes(root.size)
                ),
            );
        }
        if result.cancelled {
            totals.push("scan stopped early".into());
        }
        if self.locating() {
            totals.push(format!("{errors} unreadable, finding which"));
        } else if errors > 0 {
            totals.push(format!("{errors} unreadable (e lists them)"));
        }
        let (found, at) = &self.last_found;
        if !found.is_empty() {
            totals.push(format!("result {} of {}, n / N", at + 1, found.len()));
        }
        let rescan = self.rescan.as_ref().map(|r| {
            format!(
                "Scanning {}: {}, {} files. Esc stops it.",
                self.shown(&r.path),
                format::bytes(r.progress.bytes.load(Relaxed)),
                count(r.progress.files.load(Relaxed))
            )
        });
        let home = &self.home;
        let private = ["Library", "Desktop", "Documents", "Downloads"].map(|d| home.join(d));
        let needs_access = result
            .errors
            .iter()
            .any(|e| private.iter().any(|p| e.path.starts_with(p)));

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
            .children(rescan.map(|r| div().text_color(rgb(t.text)).child(r)))
            .when(needs_access, |d| {
                d.child(div().text_color(rgb(t.warning)).child(format!(
                    "Some private folders are unreadable. Give {} Full Disk Access in System \
                     Settings > Privacy & Security for a complete scan.",
                    grantee()
                )))
            })
            .children(
                self.note
                    .clone()
                    .map(|n| div().text_color(rgb(t.warning)).child(n)),
            )
    }

    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(b) = self.browser() else {
            return Vec::new();
        };
        let t = &self.theme;
        let tree = b.tree();
        let folder_size = tree.node(b.folder()).size.max(1);
        let selected = b.selected();
        let now = crate::visual::now();
        b.rows()[range]
            .iter()
            .map(|&id| {
                let n = tree.node(id);
                let share = n.size as f32 / folder_size as f32;
                // The same colour as the item's cell in the visual, in either colour mode.
                let colour = colour::fill(Some(n), self.settings.colour, t, now);
                let name = if n.is_dir() {
                    format!("{}/", n.name)
                } else {
                    n.name.to_string()
                };
                let marked = self.marks.covered(tree, id);
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
                            .when(marked, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(name),
                    )
                    .when(marked, |d| {
                        // Inverted, so it reads as a mark in any theme and never as a kind.
                        d.child(
                            div()
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(t.text))
                                .text_color(rgb(t.bg))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("marked"),
                        )
                    })
                    .children(
                        marker(n.flags)
                            .filter(|_| !marked)
                            .map(|m| div().text_color(rgb(t.dim)).child(m)),
                    )
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
                            .font_family(MONO)
                            .child(format::bytes(n.size)),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .text_color(rgb(t.dim))
                            .child(count(n.files as u64)),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// One line under the list about the selected item: what the tooltip says, for the
    /// keyboard.
    fn render_details(&self, b: &Browser) -> Option<impl IntoElement + use<>> {
        let t = &self.theme;
        let id = b.selected()?;
        let (title, detail) = self.describe(b.tree(), id);
        Some(
            div()
                .px_4()
                .py_1()
                .border_t_1()
                .border_color(rgb(t.faint))
                .text_size(px(12.))
                .flex()
                .gap_3()
                .child(
                    div()
                        .min_w_0()
                        .font_family(MONO)
                        .text_ellipsis_start()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(div().flex_shrink_0().text_color(rgb(t.dim)).child(detail)),
        )
    }

    fn render_help(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let mut list = div().flex().flex_col().gap_1();
        for (keys, what) in HELP {
            list = list.child(
                div()
                    .flex()
                    .gap_4()
                    .child(div().w(px(140.)).font_family(MONO).child(*keys))
                    .child(div().text_color(rgb(t.dim)).child(*what)),
            );
        }
        self.scrim()
            .id("help")
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.help = false;
                cx.notify();
            }))
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
                    .child(
                        div()
                            .text_color(rgb(t.dim))
                            .child("? or Esc closes this, or click anywhere"),
                    ),
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
