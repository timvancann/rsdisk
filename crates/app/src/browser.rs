//! Where you are in the scan and what is selected (spec §3.8, §3.9). The selection is a node,
//! never a row index, so re-sorting or changing the tree can't move it to another item.

use rsdisk_core::scan::ScanResult;
use rsdisk_core::tree::{NodeId, Tree, flag};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Size,
    Name,
    Files,
}

pub struct Browser {
    result: ScanResult,
    folder: NodeId,
    selected: Option<NodeId>,
    sort: Sort,
    rows: Vec<NodeId>,
}

impl Browser {
    pub fn new(result: ScanResult) -> Self {
        let folder = result.tree.root();
        let mut b = Browser {
            result,
            folder,
            selected: None,
            sort: Sort::Size,
            rows: Vec::new(),
        };
        b.refresh();
        b.selected = b.rows.first().copied();
        b
    }

    /// The scan so far, shown while it runs: nothing is selected, since keys can't move yet.
    pub fn so_far(result: ScanResult) -> Self {
        let mut b = Self::new(result);
        b.selected = None;
        b
    }

    pub fn result(&self) -> &ScanResult {
        &self.result
    }

    pub fn result_mut(&mut self) -> &mut ScanResult {
        &mut self.result
    }

    pub fn tree(&self) -> &Tree {
        &self.result.tree
    }

    pub fn folder(&self) -> NodeId {
        self.folder
    }

    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// The current folder's children in the current sort order.
    pub fn rows(&self) -> &[NodeId] {
        &self.rows
    }

    pub fn selected_index(&self) -> Option<usize> {
        let s = self.selected?;
        self.rows.iter().position(|r| *r == s)
    }

    /// The folders from the scan root down to the current one.
    pub fn breadcrumb(&self) -> Vec<NodeId> {
        let mut chain = vec![self.folder];
        while let Some(p) = self.tree().node(*chain.last().unwrap()).parent() {
            chain.push(p);
        }
        chain.reverse();
        chain
    }

    /// Re-reads the current folder's children, after a sort change or a change to the tree.
    /// Keeps the selection when it is still there, else selects the first row.
    pub fn refresh(&mut self) {
        let tree = &self.result.tree;
        let mut rows = tree.children(self.folder).to_vec();
        match self.sort {
            Sort::Size => {}
            Sort::Name => rows.sort_by_cached_key(|id| tree.node(*id).name.to_lowercase()),
            Sort::Files => rows.sort_by_key(|id| std::cmp::Reverse(tree.node(*id).files)),
        }
        self.rows = rows;
        if self.selected.is_none_or(|s| !self.rows.contains(&s)) {
            self.selected = self.rows.first().copied();
        }
    }

    fn select_index(&mut self, i: usize) {
        if !self.rows.is_empty() {
            self.selected = Some(self.rows[i.min(self.rows.len() - 1)]);
        }
    }

    pub fn down(&mut self) {
        self.move_by(1);
    }

    pub fn up(&mut self) {
        self.move_by(-1);
    }

    pub fn top(&mut self) {
        self.select_index(0);
    }

    pub fn bottom(&mut self) {
        self.select_index(usize::MAX);
    }

    /// Moves half of `visible_rows` down, at least one row.
    pub fn half_page_down(&mut self, visible_rows: usize) {
        self.move_by((visible_rows / 2).max(1) as isize);
    }

    pub fn half_page_up(&mut self, visible_rows: usize) {
        self.move_by(-((visible_rows / 2).max(1) as isize));
    }

    fn move_by(&mut self, delta: isize) {
        let Some(i) = self.selected_index() else {
            return;
        };
        self.select_index(i.saturating_add_signed(delta));
    }

    /// Whether a node is a folder that can be entered.
    pub fn can_open(&self, id: NodeId) -> bool {
        let n = self.tree().node(id);
        n.is_dir() && !n.has(flag::UNREADABLE | flag::NOT_SCANNED | flag::OTHER_VOLUME)
    }

    /// Enters the selected folder. Files and folders that weren't scanned stay put.
    pub fn open(&mut self) {
        if let Some(s) = self.selected
            && self.can_open(s)
        {
            self.enter(s, None);
        }
    }

    /// Goes to the parent folder, selecting the folder you came from.
    pub fn back(&mut self) {
        if let Some(parent) = self.tree().node(self.folder).parent() {
            let from = self.folder;
            self.enter(parent, Some(from));
        }
    }

    /// Opens a folder from the breadcrumb (or anywhere), selecting its first row.
    pub fn go_to_folder(&mut self, folder: NodeId) {
        if folder == self.tree().root() || self.can_open(folder) {
            self.enter(folder, None);
        }
    }

    /// Opens `id`'s parent and selects `id`.
    pub fn jump_to(&mut self, id: NodeId) {
        match self.tree().node(id).parent() {
            Some(parent) => self.enter(parent, Some(id)),
            None => self.enter(id, None),
        }
    }

    pub fn cycle_sort(&mut self) {
        self.set_sort(match self.sort {
            Sort::Size => Sort::Name,
            Sort::Name => Sort::Files,
            Sort::Files => Sort::Size,
        });
    }

    /// A click on a column header.
    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.refresh();
    }

    fn enter(&mut self, folder: NodeId, select: Option<NodeId>) {
        self.folder = folder;
        self.selected = select;
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsdisk_core::kind::Kind;
    use rsdisk_core::tree::{Node, TreeBuilder};
    use std::time::Duration;

    fn dir(name: &str, flags: u8) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR | flags)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Other, 0)
    }

    /// /r { big { x:500, y:5 }, Alpha:300, empty {}, locked {unreadable}, cloud {not scanned},
    ///      zed:10 }
    fn browser() -> (Browser, [NodeId; 8]) {
        let mut b = TreeBuilder::new(dir("/r", 0));
        let big = b.add_children(
            0,
            [
                dir("big", 0),
                file("Alpha", 300),
                dir("empty", 0),
                dir("locked", flag::UNREADABLE),
                dir("cloud", flag::NOT_SCANNED),
                file("zed", 10),
            ],
        );
        let x = b.add_children(big, [file("x", 500), file("y", 5)]);
        let result = ScanResult {
            tree: b.finish(),
            errors: Vec::new(),
            elapsed: Duration::ZERO,
            cancelled: false,
            unlocated: 0,
        };
        let [alpha, empty, locked, cloud, zed] = [big + 1, big + 2, big + 3, big + 4, big + 5];
        (
            Browser::new(result),
            [big, alpha, empty, locked, cloud, zed, x, x + 1],
        )
    }

    #[test]
    fn starts_at_the_root_with_the_largest_selected() {
        let (b, [big, ..]) = browser();
        assert_eq!(b.folder(), 0);
        assert_eq!(b.selected(), Some(big));
        assert_eq!(b.selected_index(), Some(0));
    }

    #[test]
    fn moving_clamps_at_both_ends() {
        let (mut b, ..) = browser();
        b.up();
        assert_eq!(b.selected_index(), Some(0));
        for _ in 0..20 {
            b.down();
        }
        assert_eq!(b.selected_index(), Some(b.rows().len() - 1));
        b.top();
        assert_eq!(b.selected_index(), Some(0));
        b.bottom();
        assert_eq!(b.selected_index(), Some(5));
    }

    #[test]
    fn half_pages_move_by_half_the_visible_rows() {
        let (mut b, ..) = browser();
        b.half_page_down(6);
        assert_eq!(b.selected_index(), Some(3));
        b.half_page_up(2);
        assert_eq!(b.selected_index(), Some(2));
        b.half_page_down(100);
        assert_eq!(b.selected_index(), Some(5));
    }

    #[test]
    fn open_then_back_restores_the_selection() {
        let (mut b, [big, .., x, _]) = browser();
        b.open();
        assert_eq!(b.folder(), big);
        assert_eq!(b.selected(), Some(x));
        b.back();
        assert_eq!(b.folder(), 0);
        assert_eq!(b.selected(), Some(big));
        b.back();
        assert_eq!(b.folder(), 0);
    }

    #[test]
    fn open_ignores_files_and_folders_that_werent_scanned() {
        let (mut b, [_, alpha, _, locked, cloud, ..]) = browser();
        for id in [alpha, locked, cloud] {
            b.jump_to(id);
            b.open();
            assert_eq!(b.folder(), 0, "{id}");
            assert_eq!(b.selected(), Some(id));
        }
    }

    #[test]
    fn an_empty_folder_has_no_selection() {
        let (mut b, [_, _, empty, ..]) = browser();
        b.jump_to(empty);
        b.open();
        assert_eq!(b.folder(), empty);
        assert!(b.rows().is_empty());
        assert_eq!(b.selected(), None);
        b.down();
        b.up();
        b.bottom();
        b.half_page_down(10);
        b.open();
        assert_eq!(b.selected(), None);
        b.back();
        assert_eq!(b.selected(), Some(empty));
    }

    #[test]
    fn sorting_keeps_the_selected_node() {
        let (mut b, [_, alpha, ..]) = browser();
        b.jump_to(alpha);
        b.cycle_sort();
        assert_eq!(b.sort(), Sort::Name);
        assert_eq!(b.selected(), Some(alpha));
        assert_eq!(b.selected_index(), Some(0));
        let names: Vec<&str> = b
            .rows()
            .iter()
            .map(|id| &*b.tree().node(*id).name)
            .collect();
        assert_eq!(names, ["Alpha", "big", "cloud", "empty", "locked", "zed"]);
        b.cycle_sort();
        assert_eq!(b.sort(), Sort::Files);
        assert_eq!(
            b.rows()[0],
            b.rows()
                .iter()
                .copied()
                .max_by_key(|id| b.tree().node(*id).files)
                .unwrap()
        );
        b.cycle_sort();
        assert_eq!(b.sort(), Sort::Size);
        assert_eq!(b.selected(), Some(alpha));
    }

    #[test]
    fn a_header_click_sorts_by_its_column() {
        let (mut b, [_, alpha, ..]) = browser();
        b.jump_to(alpha);
        b.set_sort(Sort::Name);
        assert_eq!(b.sort(), Sort::Name);
        assert_eq!(b.selected(), Some(alpha));
        assert_eq!(b.rows()[0], alpha);
        // Straight to a column, not the next one in `s`'s cycle (which would be Files).
        b.set_sort(Sort::Size);
        assert_eq!(b.sort(), Sort::Size);
        assert_eq!(b.selected(), Some(alpha));
    }

    #[test]
    fn jump_to_a_deep_node_opens_its_folder() {
        let (mut b, [big, .., y]) = browser();
        b.jump_to(y);
        assert_eq!(b.folder(), big);
        assert_eq!(b.selected(), Some(y));
        assert_eq!(b.breadcrumb(), [0, big]);
    }

    #[test]
    fn breadcrumb_folders_can_be_revisited() {
        let (mut b, [big, .., y]) = browser();
        b.jump_to(y);
        b.go_to_folder(0);
        assert_eq!(b.folder(), 0);
        assert_eq!(b.selected(), Some(big));
    }

    #[test]
    fn refresh_after_a_removal_moves_off_the_removed_node() {
        let (mut b, [big, alpha, ..]) = browser();
        b.result_mut().tree.remove(big);
        b.refresh();
        assert_eq!(b.selected(), Some(alpha));
    }
}
