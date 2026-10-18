//! The search box: `/` to open, type to filter, Enter to jump, `n`/`N` afterwards (§3.10).

use crate::search::search;
use crate::view::{RsDisk, Stage};
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::tree::NodeId;

const SHOWN: usize = 20;

#[derive(Default)]
pub struct Find {
    pub query: String,
    pub results: Vec<NodeId>,
    pub cursor: usize,
}

impl RsDisk {
    pub fn open_find(&mut self) {
        self.find = Some(Find::default());
    }

    /// Keys while the search box is open. Returns whether the key was used.
    pub fn find_key(&mut self, key: &str, typed: Option<&str>, ctrl: bool) -> bool {
        let Some(find) = &mut self.find else {
            return false;
        };
        match (key, ctrl) {
            ("escape", _) => self.find = None,
            ("enter", _) => {
                let find = self.find.take().unwrap_or_default();
                if let Some(&id) = find.results.get(find.cursor) {
                    self.last_found = (find.results, find.cursor);
                    self.jump(id);
                }
            }
            ("down", _) | ("n", true) => {
                find.cursor = (find.cursor + 1).min(find.results.len().saturating_sub(1));
            }
            ("up", _) | ("p", true) => find.cursor = find.cursor.saturating_sub(1),
            ("backspace", _) => {
                find.query.pop();
                self.refind();
            }
            (_, false) => {
                if let Some(text) = typed.filter(|t| !t.chars().any(char::is_control)) {
                    find.query.push_str(text);
                    self.refind();
                }
            }
            _ => {}
        }
        true
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
            && !b.tree().node(id).has(rsdisk_core::tree::flag::REMOVED)
        {
            b.jump_to(id);
            self.hover = None;
            if let Some(i) = b.selected_index() {
                self.scroll.scroll_to_item(i, ScrollStrategy::Center);
            }
        }
    }

    pub fn render_find(&self) -> Option<impl IntoElement + use<>> {
        let find = self.find.as_ref()?;
        let b = self.browser()?;
        let t = &self.theme;
        let tree = b.tree();
        let mut list = div().flex().flex_col();
        for (i, &id) in find.results.iter().enumerate() {
            let path = tree.path(id);
            let shown = path
                .strip_prefix(&self.root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            list = list.child(
                div()
                    .flex()
                    .justify_between()
                    .gap_4()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .when(i == find.cursor, |d| d.bg(rgb(t.raised)))
                    .child(div().min_w_0().truncate().font_family("Menlo").child(shown))
                    .child(
                        div()
                            .text_color(rgb(t.dim))
                            .child(format::bytes(tree.node(id).size)),
                    ),
            );
        }
        let hint = if find.query.is_empty() {
            "Type part of a name; use / for folders above it, like lib/cach"
        } else if find.results.is_empty() {
            "Nothing matches"
        } else {
            "Enter jumps there, arrows or Ctrl-n / Ctrl-p move, Esc closes"
        };
        Some(
            div()
                .absolute()
                .top(px(60.))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .w(px(640.))
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.faint))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .font_family("Menlo")
                                .px_2()
                                .child(format!("/{}\u{258f}", find.query)),
                        )
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
