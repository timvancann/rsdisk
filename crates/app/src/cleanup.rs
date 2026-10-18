//! Marked items, the confirm dialog, and moving to the Trash (spec §3.8, §3.11).

use crate::view::{RsDisk, Stage};
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::trash::{FinderTrash, apply, targets, trash_paths};
use rsdisk_core::tree::NodeId;
use std::sync::mpsc::{Receiver, channel};

/// How many marked paths the dialog lists before "and N more".
const LISTED: usize = 12;

pub enum Dialog {
    Confirm {
        ids: Vec<NodeId>,
        size: u64,
    },
    Trashing {
        count: usize,
        done: Receiver<Vec<(NodeId, Result<(), String>)>>,
    },
    Done {
        moved: usize,
        freed: u64,
        failed: Vec<(String, String)>,
    },
}

impl RsDisk {
    /// `space`: marks or unmarks the selected row and moves down, as in dua.
    pub fn toggle_mark(&mut self) {
        if let Stage::Browsing(b) = &mut self.stage
            && let Some(id) = b.selected()
        {
            self.marks.toggle(b.tree(), id);
            b.down();
        }
    }

    /// `d`: review the marked items.
    pub fn open_confirm(&mut self) {
        if let Stage::Browsing(b) = &self.stage
            && !self.marks.is_empty()
        {
            let ids = self.marks.ids(b.tree());
            let size = self.marks.total_size(b.tree());
            self.dialog = Some(Dialog::Confirm { ids, size });
        }
    }

    /// Keys while a dialog is open. Returns whether the key was used.
    pub fn dialog_key(&mut self, key: &str) -> bool {
        match &self.dialog {
            None => false,
            Some(Dialog::Confirm { ids, .. }) => {
                match key {
                    "enter" | "y" => {
                        let ids = ids.clone();
                        self.start_trash(ids);
                    }
                    "escape" | "n" | "q" => self.dialog = None,
                    _ => {}
                }
                true
            }
            Some(Dialog::Trashing { .. }) => true,
            Some(Dialog::Done { .. }) => {
                if matches!(key, "enter" | "escape" | "q" | "y") {
                    self.dialog = None;
                }
                true
            }
        }
    }

    fn start_trash(&mut self, ids: Vec<NodeId>) {
        let Stage::Browsing(b) = &self.stage else {
            return;
        };
        let todo = targets(b.tree(), &ids);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(trash_paths(todo, &FinderTrash));
        });
        self.dialog = Some(Dialog::Trashing {
            count: ids.len(),
            done: rx,
        });
    }

    /// Applies a finished trash run to the tree. Returns whether anything changed.
    pub fn poll_trash(&mut self) -> bool {
        let Some(Dialog::Trashing { done, .. }) = &self.dialog else {
            return false;
        };
        let Ok(results) = done.try_recv() else {
            return false;
        };
        let Stage::Browsing(b) = &mut self.stage else {
            return false;
        };
        let sizes: Vec<(NodeId, u64)> = results
            .iter()
            .map(|(id, _)| (*id, b.tree().node(*id).size))
            .collect();
        let tree = &mut b.result_mut().tree;
        let paths: Vec<(NodeId, String)> = results
            .iter()
            .map(|(id, _)| (*id, tree.path(*id).display().to_string()))
            .collect();
        let outcome = apply(tree, results);
        let freed = sizes
            .iter()
            .filter(|(id, _)| outcome.removed.contains(id))
            .map(|(_, s)| s)
            .sum();
        let failed = outcome
            .failed
            .iter()
            .map(|(id, why)| {
                let path = paths.iter().find(|(p, _)| p == id).map(|(_, p)| p.clone());
                (path.unwrap_or_default(), why.clone())
            })
            .collect();
        self.marks.prune(tree);
        b.refresh();
        self.dialog = Some(Dialog::Done {
            moved: outcome.removed.len(),
            freed,
            failed,
        });
        true
    }

    pub fn render_marked_pane(&self) -> Option<impl IntoElement + use<>> {
        let b = self.browser()?;
        if self.marks.is_empty() {
            return None;
        }
        let t = &self.theme;
        let tree = b.tree();
        let n = self.marks.len();
        let items = if n == 1 { "item" } else { "items" };
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(rgb(t.faint))
                .bg(rgb(t.surface))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                    "{n} {items} marked, {}",
                    format::bytes(self.marks.total_size(tree))
                )))
                .child(
                    div()
                        .text_color(rgb(t.dim))
                        .child("d to review and move to the Trash, u to clear"),
                ),
        )
    }

    pub fn render_dialog(&self) -> Option<impl IntoElement + use<>> {
        let dialog = self.dialog.as_ref()?;
        let b = self.browser()?;
        let t = &self.theme;
        let tree = b.tree();
        let mut body = div().flex().flex_col().gap_2().w(px(560.));
        match dialog {
            Dialog::Confirm { ids, size } => {
                let n = ids.len();
                body = body.child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!(
                            "Move {n} {} ({}) to the Trash?",
                            if n == 1 { "item" } else { "items" },
                            format::bytes(*size)
                        )),
                );
                let mut list = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .font_family("Menlo")
                    .text_size(px(12.));
                for id in ids.iter().take(LISTED) {
                    let path = tree.path(*id);
                    let shown = path
                        .strip_prefix(&self.root)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| path.display().to_string());
                    list = list.child(
                        div()
                            .flex()
                            .justify_between()
                            .gap_4()
                            .child(div().min_w_0().truncate().child(shown))
                            .child(div().child(format::bytes(tree.node(*id).size))),
                    );
                }
                if n > LISTED {
                    list = list.child(
                        div()
                            .text_color(rgb(t.dim))
                            .child(format!("and {} more", n - LISTED)),
                    );
                }
                body = body.child(list).child(div().text_color(rgb(t.dim)).child(
                    "Finder moves them, so Put Back restores them. Enter or y to go ahead, \
                     Esc to cancel.",
                ));
            }
            Dialog::Trashing { count, .. } => {
                body = body.child(format!(
                    "Moving {count} {} to the Trash...",
                    if *count == 1 { "item" } else { "items" }
                ));
            }
            Dialog::Done {
                moved,
                freed,
                failed,
            } => {
                body = body.child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!(
                            "Moved {moved} {} to the Trash, {} freed",
                            if *moved == 1 { "item" } else { "items" },
                            format::bytes(*freed)
                        )),
                );
                if !failed.is_empty() {
                    body = body.child(
                        div()
                            .text_color(rgb(t.error))
                            .child(format!("{} couldn't be moved:", failed.len())),
                    );
                    let mut list = div().flex().flex_col().gap_1().text_size(px(12.));
                    for (path, why) in failed.iter().take(LISTED) {
                        list = list.child(
                            div()
                                .child(div().font_family("Menlo").truncate().child(path.clone()))
                                .child(div().text_color(rgb(t.dim)).child(why.clone())),
                        );
                    }
                    body = body.child(list);
                    let automation = failed
                        .iter()
                        .any(|(_, why)| why.contains("-1743") || why.contains("not allowed"));
                    if automation {
                        body = body.child(div().text_color(rgb(t.accent)).child(
                            "macOS blocked controlling Finder. Allow it for your terminal in \
                             System Settings > Privacy & Security > Automation, then try again.",
                        ));
                    }
                }
                body = body.child(
                    div()
                        .text_color(rgb(t.dim))
                        .child("Enter or Esc closes this"),
                );
            }
        }
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0., 0., 0., 0.25))
                .child(
                    div()
                        .p_5()
                        .rounded_lg()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.faint))
                        .child(body),
                ),
        )
    }
}
