//! The search box: `/` to open, type to filter, Enter to jump, `n`/`N` afterwards (§3.10).
//! `e` reuses it to list the unreadable folders.

use crate::search::search;
use crate::theme::MONO;
use crate::view::{RsDisk, Stage};
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::tree::{NodeId, flag};

/// How many results a search returns, and how many rows the box shows at once.
const SHOWN: usize = 20;

#[derive(Default)]
pub struct Find {
    pub query: String,
    pub results: Vec<NodeId>,
    pub cursor: usize,
    /// A fixed list (the unreadable folders) instead of a search; typing does nothing.
    pub title: Option<String>,
}

impl RsDisk {
    pub fn open_find(&mut self) {
        if self.browser().is_some() {
            self.find = Some(Find::default());
        }
    }

    /// `e`: the unreadable folders, by path, to step through like search results.
    pub fn open_unreadable(&mut self) {
        let Some(b) = self.browser() else { return };
        let tree = b.tree();
        let mut ids: Vec<NodeId> = (0..tree.len() as NodeId)
            .filter(|&id| {
                let n = tree.node(id);
                n.has(flag::UNREADABLE) && !n.has(flag::REMOVED)
            })
            .collect();
        if ids.is_empty() {
            self.note = Some(if self.locating() {
                "Still finding which folders are unreadable".into()
            } else {
                "Every folder was readable".into()
            });
            return;
        }
        ids.sort_by_cached_key(|&id| tree.path(id));
        let title = format!("{} unreadable folders", ids.len());
        self.find = Some(Find {
            results: ids,
            title: Some(title),
            ..Find::default()
        });
    }

    /// Keys while the search box is open. Returns whether the key was used.
    pub fn find_key(&mut self, key: &str, typed: Option<&str>, ctrl: bool) -> bool {
        let Some(find) = &mut self.find else {
            return false;
        };
        let fixed = find.title.is_some();
        match (key, ctrl) {
            ("escape", _) => self.find = None,
            ("enter", _) => self.pick(None),
            ("down", _) | ("n", true) => {
                find.cursor = (find.cursor + 1).min(find.results.len().saturating_sub(1));
            }
            ("up", _) | ("p", true) => find.cursor = find.cursor.saturating_sub(1),
            ("j", false) if fixed => {
                find.cursor = (find.cursor + 1).min(find.results.len().saturating_sub(1));
            }
            ("k", false) if fixed => find.cursor = find.cursor.saturating_sub(1),
            ("backspace", _) if !fixed => {
                find.query.pop();
                self.refind();
            }
            (_, false) if !fixed => {
                if let Some(text) = typed.filter(|t| !t.chars().any(char::is_control)) {
                    find.query.push_str(text);
                    self.refind();
                }
            }
            _ => {}
        }
        true
    }

    /// Jumps to a result (the cursor's, or `at`) and keeps the list for `n`/`N`.
    fn pick(&mut self, at: Option<usize>) {
        let find = self.find.take().unwrap_or_default();
        let at = at.unwrap_or(find.cursor);
        if let Some(&id) = find.results.get(at) {
            self.last_found = (find.results, at);
            self.jump(id);
        }
    }

    fn refind(&mut self) {
        let (Some(find), Stage::Browsing(b)) = (&mut self.find, &self.stage) else {
            return;
        };
        find.results = search(b.tree(), &find.query, SHOWN);
        find.cursor = 0;
    }

    /// `n` / `N`: the next or previous result of the last search.
    pub fn step_found(&mut self, forward: bool) {
        let (results, at) = &mut self.last_found;
        if results.is_empty() {
            return;
        }
        *at = if forward {
            (*at + 1) % results.len()
        } else {
            (*at + results.len() - 1) % results.len()
        };
        let id = results[*at];
        self.jump(id);
    }

    fn jump(&mut self, id: NodeId) {
        if let Stage::Browsing(b) = &mut self.stage
            && !b.tree().node(id).has(flag::REMOVED)
        {
            b.jump_to(id);
            self.hover = None;
            if let Some(i) = b.selected_index() {
                self.scroll.scroll_to_item(i, ScrollStrategy::Center);
            }
        }
    }

    pub fn render_find(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let find = self.find.as_ref()?;
        let b = self.browser()?;
        let t = &self.theme;
        let tree = b.tree();
        // A window of rows that keeps the cursor in view on long lists.
        let first = find.cursor.saturating_sub(SHOWN - 1);
        let mut list = div().flex().flex_col();
        for (i, &id) in find.results.iter().enumerate().skip(first).take(SHOWN) {
            let n = tree.node(id);
            let mut shown = self.shown(&tree.path(id));
            if n.is_dir() {
                shown.push('/');
            }
            list = list.child(
                div()
                    .id(("found", i))
                    .flex()
                    .justify_between()
                    .gap_4()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(i == find.cursor, |d| d.bg(rgb(t.raised)))
                    .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                        view.pick(Some(i));
                        cx.notify();
                    }))
                    .child(div().min_w_0().truncate().font_family(MONO).child(shown))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_color(rgb(t.dim))
                            .child(format::bytes(n.size)),
                    ),
            );
        }
        let (top, hint) = match &find.title {
            Some(title) => (
                div()
                    .px_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.clone()),
                "Enter jumps there and n / N step through the rest; j / k move, Esc closes",
            ),
            None => (
                div()
                    .font_family(MONO)
                    .px_2()
                    .child(format!("/{}\u{258f}", find.query)),
                if find.query.is_empty() {
                    "Type part of a name; use / for folders above it, like lib/cach"
                } else if find.results.is_empty() {
                    "Nothing matches"
                } else {
                    "Enter jumps there, arrows or Ctrl-n / Ctrl-p move, Esc closes"
                },
            ),
        };
        Some(
            self.scrim()
                .id("find-scrim")
                .on_click(cx.listener(|view, _: &ClickEvent, _, cx| {
                    view.find = None;
                    cx.notify();
                }))
                .items_start()
                .pt(px(60.))
                .child(
                    div()
                        .id("find")
                        .occlude()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .w(px(640.))
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.faint))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(top)
                        .child(list)
                        .child(
                            div()
                                .px_2()
                                .text_color(rgb(t.dim))
                                .text_size(px(12.))
                                .child(hint),
                        ),
                ),
        )
    }
}
