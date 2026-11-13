//! The browsing screen: header, the sorted list, the selection's details, and the visual
//! (spec §3.8).

use crate::browser::{Browser, Sort};
use crate::colour;
use crate::scanning::count;
use crate::settings::{ColourMode, Visual};
use crate::theme::MONO;
use crate::view::{ROW_HEIGHT, RsDisk, grantee};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::scan::Progress;
use rsdisk_core::tree::flag;
use std::ops::Range;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

const BAR_WIDTH: f32 = 80.;
/// The size and files columns.
const NUMBER_WIDTH: f32 = 84.;
/// Breadcrumbs beyond the root and this many of the last ones collapse to an ellipsis.
const CRUMBS_KEPT: usize = 3;

/// The `?` sheet, in groups. Keys in one entry are split on " / ", each drawn as a key.
const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Move",
        &[
            ("j / k", "down / up"),
            ("l / Enter", "open the folder"),
            ("h / Backspace", "up a folder"),
            ("gg / G", "top / bottom"),
            ("Ctrl-d / Ctrl-u", "half a page down / up"),
        ],
    ),
    (
        "Clean up",
        &[
            ("Space", "mark or unmark, then down"),
            (
                "Right-click",
                "mark or unmark the row or cell under the pointer",
            ),
            ("u", "clear the marks"),
            ("d", "review the marked items, move them to the Trash"),
        ],
    ),
    (
        "Find",
        &[
            ("/", "search the whole scan"),
            ("n / N", "next / previous result"),
            ("e", "list the unreadable folders"),
        ],
    ),
    (
        "Look",
        &[
            ("s", "sort by size, name, files (or click a column)"),
            ("v", "treemap / sunburst"),
            ("a", "colour by kind / age"),
            ("o", "reveal in Finder"),
            ("p", "preview with Quick Look"),
        ],
    ),
    (
        "Scan",
        &[
            ("r", "rescan this folder"),
            ("c", "scan the selected cloud folder"),
            ("Esc", "stop a scan, close the preview"),
        ],
    ),
    (
        "rsdisk",
        &[
            ("?", "these keys"),
            ("Cmd-,", "settings and permissions"),
            ("q", "quit"),
        ],
    ),
];

/// The bar along the bottom: the keys used most, the rest behind `?`.
const KEY_BAR: &[(&str, &str)] = &[
    ("j / k", "move"),
    ("l", "open"),
    ("h", "up"),
    ("Space", "mark"),
    ("d", "trash"),
    ("/", "search"),
    ("s", "sort"),
    // `v` and `a` say what's showing now; see `render_key_bar`.
    ("v", "visual"),
    ("a", "colour"),
    ("?", "all keys"),
];

/// While the first scan runs, keys wait for the tree; only these do something.
const SCAN_KEY_BAR: &[(&str, &str)] = &[("Esc", "stop and show what's found"), ("q", "quit")];

impl RsDisk {
    /// The browser, or with `scanning` the scan so far, under a header with its counters.
    pub fn render_browser(
        &self,
        b: &Browser,
        scanning: Option<(&Progress, Instant)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let rows = b.rows().len();
        let list = if rows == 0 {
            div()
                .px_4()
                .py_3()
                .text_color(rgb(t.dim))
                .child(if scanning.is_some() {
                    "Nothing found yet"
                } else {
                    "This folder is empty"
                })
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
            .child(match scanning {
                Some((progress, started)) => self
                    .render_scan_header(progress, started)
                    .into_any_element(),
                None => self.render_header(b, cx).into_any_element(),
            })
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
                            .when(rows > 0, |d| d.child(self.render_columns(b, cx)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .relative()
                                    .child(list)
                                    // Always shown, so you can see how far down the list you are.
                                    .when(rows > 0, |d| d.vertical_scrollbar(&self.scroll)),
                            )
                            .children(self.render_details(b)),
                    )
                    .child(self.render_visual(b, cx)),
            )
            .children(self.render_marked_pane())
            .child(self.render_key_bar(scanning.is_some()))
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
            .gap_1p5()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(16.));
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
                    .when(i == last, |d| d.font_weight(FontWeight::MEDIUM))
                    .when(i != last, |d| d.hover(|s| s.text_color(rgb(t.text))))
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
        // The folder's size leads, in the text colour; the rest is context.
        let (size, of) = if b.folder() == tree.root() {
            (
                format!("{at_least}{}", format::bytes(root.size)),
                String::new(),
            )
        } else {
            (
                format::bytes(folder.size),
                format!(" of {at_least}{}", format::bytes(root.size)),
            )
        };
        let mut totals = vec![format!("{} files", count(folder.files as u64))];
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

        let rest = format!("{of}, {}", totals.join(", "));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .pt_3()
            .pb_2p5()
            .border_b_1()
            .border_color(rgb(t.faint))
            .child(crumbs)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .text_size(px(12.))
                    .text_color(rgb(t.dim))
                    .child(
                        div()
                            .font_family(MONO)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(t.text))
                            .child(size),
                    )
                    .child(div().min_w_0().truncate().child(rest)),
            )
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

    /// The column names over the list, laid out like a row; a click sorts by that column.
    fn render_columns(&self, b: &Browser, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let current = b.sort();
        // While the first scan runs nothing can be sorted yet, so the names don't look clickable.
        let browsing = self.browser().is_some();
        let column = |sort: Sort, text: &'static str| {
            let on = sort == current;
            div()
                .id(text)
                .flex()
                .text_color(rgb(if on { t.text } else { t.dim }))
                .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                .when(browsing, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.text_color(rgb(t.text)))
                        .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                            view.with_browser(cx, |b| b.set_sort(sort))
                        }))
                })
                .child(text)
        };
        div()
            .h(px(28.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_4()
            .px_4()
            .border_b_1()
            .border_color(rgb(t.faint))
            .text_size(px(12.))
            .child(div().flex_1().child(column(Sort::Name, "Name")))
            .child(div().w(px(BAR_WIDTH)))
            .child(
                div()
                    .w(px(NUMBER_WIDTH))
                    .flex()
                    .justify_end()
                    .child(column(Sort::Size, "Size")),
            )
            .child(
                div()
                    .w(px(NUMBER_WIDTH))
                    .flex()
                    .justify_end()
                    .child(column(Sort::Files, "Files")),
            )
    }

    fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let b = self.shown_browser();
        let t = &self.theme;
        let tree = b.tree();
        let folder_size = tree.node(b.folder()).size.max(1);
        let selected = b.selected();
        let now = crate::visual::now();
        // The scan so far can't be marked: its nodes aren't the finished tree's.
        let browsing = self.browser().is_some();
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
                    .gap_4()
                    .px_4()
                    .when(selected == Some(id), |d| d.bg(rgb(t.raised)))
                    .on_click(cx.listener(move |view, ev: &ClickEvent, _, cx| {
                        view.click_row(id, ev.click_count() >= 2, cx)
                    }))
                    .when(browsing, |d| {
                        d.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |view, _: &MouseDownEvent, _, cx| {
                                view.note = None;
                                view.toggle_mark_of(id);
                                cx.notify();
                            }),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.))
                            .when(marked, |d| d.font_weight(FontWeight::MEDIUM))
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
                            .w(px(NUMBER_WIDTH))
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .child(format::bytes(n.size)),
                    )
                    .child(
                        div()
                            .w(px(NUMBER_WIDTH))
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

    /// `j / k` as two keys side by side.
    fn keycaps(&self, keys: &str) -> Div {
        let t = &self.theme;
        div()
            .flex()
            .items_center()
            .gap_1()
            .children(keys.split(" / ").map(|k| {
                div()
                    .px_1()
                    .min_w(px(18.))
                    .flex()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(t.faint))
                    .bg(rgb(t.bg))
                    .font_family(MONO)
                    .text_size(px(11.))
                    .text_color(rgb(t.text))
                    .child(k.to_string())
            }))
    }

    /// The most used keys along the window's bottom edge, so they're there without `?`.
    fn render_key_bar(&self, scanning: bool) -> impl IntoElement {
        let t = &self.theme;
        let keys = if scanning { SCAN_KEY_BAR } else { KEY_BAR };
        div()
            .flex()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_x_4()
            .gap_y_1()
            .px_4()
            .py_1p5()
            .border_t_1()
            .border_color(rgb(t.faint))
            .bg(rgb(t.surface))
            .text_size(px(12.))
            .text_color(rgb(t.dim))
            .children(keys.iter().map(|&(k, what)| {
                let what = match k {
                    "v" => match self.settings.visual {
                        Visual::Treemap => "visual: treemap",
                        Visual::Sunburst => "visual: sunburst",
                    },
                    "a" => match self.settings.colour {
                        ColourMode::Kind => "colour: kind",
                        ColourMode::Age => "colour: age",
                    },
                    _ => what,
                };
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(self.keycaps(k))
                    .child(what)
            }))
    }

    fn render_help(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let group = |title: &'static str, keys: &'static [(&'static str, &'static str)]| {
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(
                    div()
                        .pb_0p5()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(t.dim))
                        .child(title),
                )
                .children(keys.iter().map(|(k, what)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().w(px(132.)).flex_shrink_0().child(self.keycaps(k)))
                        .child(div().child(*what))
                }))
        };
        // Two columns, the groups dealt out in order.
        let half = HELP.len().div_ceil(2);
        let column =
            |groups: &'static [(&'static str, &'static [(&'static str, &'static str)])]| {
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .w(px(400.))
                    .children(groups.iter().map(|(title, keys)| group(title, keys)))
            };
        self.scrim()
            .id("help")
            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                view.help = false;
                cx.notify();
            }))
            .child(
                div()
                    .p_6()
                    .rounded_lg()
                    .bg(rgb(t.surface))
                    .border_1()
                    .border_color(rgb(t.faint))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Keys"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_8()
                            .child(column(&HELP[..half]))
                            .child(column(&HELP[half..])),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
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
