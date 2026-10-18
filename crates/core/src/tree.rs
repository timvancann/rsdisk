//! The size tree: an arena of nodes built once after a scan and afterwards only shrunk by
//! deletions (spec §3.3).
//!
//! A directory's children are added in one go, so they get consecutive ids. Their sorted
//! order lives in `order`, a permutation of all ids where each directory owns the slice
//! `order[child_start..child_start + child_len]`. That keeps a node at 48 bytes with no
//! per-directory allocation.

use crate::kind::{Kind, kind_of};
use std::path::PathBuf;

pub type NodeId = u32;

const NO_PARENT: NodeId = NodeId::MAX;

/// Node flags, as bits.
pub mod flag {
    pub const DIR: u8 = 1;
    pub const UNREADABLE: u8 = 1 << 1;
    pub const NOT_SCANNED: u8 = 1 << 2;
    pub const SYMLINK: u8 = 1 << 3;
    pub const OTHER_VOLUME: u8 = 1 << 4;
    pub const REMOVED: u8 = 1 << 5;
}

#[derive(Debug, Clone)]
pub struct Node {
    /// The entry's own name; the root's is its full path.
    pub name: Box<str>,
    /// Allocated bytes; for a directory, its own blocks plus every descendant's.
    pub size: u64,
    /// Files at or below this node (a file counts itself).
    pub files: u32,
    /// Modified time, unix seconds (clamped to 0 before 1970).
    pub mtime: u32,
    parent: NodeId,
    child_start: u32,
    child_len: u32,
    pub kind: Kind,
    pub flags: u8,
}

impl Node {
    /// A new node; `size` is the entry's own allocated bytes.
    pub fn new(name: impl Into<Box<str>>, size: u64, mtime: i64, kind: Kind, flags: u8) -> Self {
        Node {
            name: name.into(),
            size,
            files: if flags & flag::DIR == 0 { 1 } else { 0 },
            mtime: mtime.clamp(0, u32::MAX as i64) as u32,
            parent: NO_PARENT,
            child_start: 0,
            child_len: 0,
            kind,
            flags,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.flags & flag::DIR != 0
    }

    pub fn has(&self, f: u8) -> bool {
        self.flags & f != 0
    }

    pub fn parent(&self) -> Option<NodeId> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }
}

fn path_of(nodes: &[Node], id: NodeId) -> PathBuf {
    let mut names = Vec::new();
    let mut at = Some(id);
    while let Some(i) = at {
        let n = &nodes[i as usize];
        names.push(&*n.name);
        at = n.parent();
    }
    names.iter().rev().collect()
}

/// Collects nodes, parents before children, then rolls sizes up in `finish`.
pub struct TreeBuilder {
    nodes: Vec<Node>,
}

impl TreeBuilder {
    pub fn new(root: Node) -> Self {
        Self::with_capacity(root, 1)
    }

    pub fn with_capacity(root: Node, capacity: usize) -> Self {
        let mut nodes = Vec::with_capacity(capacity);
        nodes.push(root);
        TreeBuilder { nodes }
    }

    /// Adds all of `parent`'s children at once and returns the first child's id; the rest
    /// follow consecutively. `parent` must already be in the builder and get its children
    /// only once.
    pub fn add_children(
        &mut self,
        parent: NodeId,
        children: impl IntoIterator<Item = Node>,
    ) -> NodeId {
        let first = self.nodes.len() as NodeId;
        self.nodes.extend(children.into_iter().map(|mut n| {
            n.parent = parent;
            n
        }));
        let len = self.nodes.len() as u32 - first;
        let p = &mut self.nodes[parent as usize];
        debug_assert_eq!(p.child_len, 0, "children added twice");
        p.child_start = first;
        p.child_len = len;
        first
    }

    pub fn finish(mut self) -> Tree {
        // Children always have higher ids than their parent, so one reverse pass rolls every
        // subtree up before its parent is read.
        for id in (1..self.nodes.len()).rev() {
            let (size, files, parent) = {
                let n = &self.nodes[id];
                (n.size, n.files, n.parent as usize)
            };
            let p = &mut self.nodes[parent];
            p.size += size;
            p.files += files;
        }
        let mut order: Vec<NodeId> = (0..self.nodes.len() as NodeId).collect();
        for n in &self.nodes {
            let range = n.child_start as usize..(n.child_start + n.child_len) as usize;
            order[range].sort_unstable_by(|a, b| {
                self.nodes[*b as usize]
                    .size
                    .cmp(&self.nodes[*a as usize].size)
            });
        }
        let mut tree = Tree {
            nodes: self.nodes,
            order,
        };
        tree.summarize();
        tree
    }
}

#[derive(Debug, Clone)]
pub struct Tree {
    nodes: Vec<Node>,
    order: Vec<NodeId>,
}

impl Tree {
    pub fn root(&self) -> NodeId {
        0
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// Children, largest first.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let n = &self.nodes[id as usize];
        &self.order[n.child_start as usize..(n.child_start + n.child_len) as usize]
    }

    /// Every node ever added, including removed ones (check `flag::REMOVED`).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        path_of(&self.nodes, id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    /// Directories with no children and none of the `exclude` flags, with their paths.
    pub fn childless_dirs(&self, exclude: u8) -> Vec<(NodeId, PathBuf)> {
        (0..self.nodes.len() as NodeId)
            .filter(|id| {
                let n = &self.nodes[*id as usize];
                n.is_dir() && n.child_len == 0 && !n.has(exclude)
            })
            .map(|id| (id, self.path(id)))
            .collect()
    }

    /// Replaces `id`'s contents with a fresh scan of it (`sub`, whose root is `id` scanned
    /// again): the new nodes are appended, the old ones marked removed, sizes and counts
    /// updated up to the root, and folder summaries redone. Used for rescans and for scanning
    /// a skipped cloud folder (spec §3.5).
    pub fn graft(&mut self, id: NodeId, sub: Tree) {
        // The old contents leave.
        let mut stack: Vec<NodeId> = self.children(id).to_vec();
        while let Some(old) = stack.pop() {
            stack.extend_from_slice(self.children(old));
            self.nodes[old as usize].flags |= flag::REMOVED;
        }

        let base = self.nodes.len() as NodeId;
        // Sub ids 1.. become base.., and sub's root becomes `id`.
        let map = |sub_id: NodeId| if sub_id == 0 { id } else { base + sub_id - 1 };
        let Tree {
            nodes: sub_nodes,
            order: sub_order,
        } = sub;
        let sub_root = &sub_nodes[0];
        let (new_size, new_files) = (sub_root.size, sub_root.files);
        let (child_start, child_len) = (sub_root.child_start, sub_root.child_len);
        let unreadable = sub_root.flags & flag::UNREADABLE;
        for mut n in sub_nodes.into_iter().skip(1) {
            n.parent = map(n.parent);
            if n.child_len > 0 {
                n.child_start = map(n.child_start);
            }
            self.nodes.push(n);
        }
        self.order.extend(sub_order.into_iter().skip(1).map(map));

        let (old_size, old_files) = {
            let n = &mut self.nodes[id as usize];
            let old = (n.size, n.files);
            n.size = new_size;
            n.files = new_files;
            n.child_start = if child_len > 0 { map(child_start) } else { 0 };
            n.child_len = child_len;
            n.flags = (n.flags & !(flag::NOT_SCANNED | flag::UNREADABLE)) | unreadable;
            old
        };
        let mut at = self.nodes[id as usize].parent();
        while let Some(p) = at {
            let n = &mut self.nodes[p as usize];
            n.size = n.size - old_size + new_size;
            n.files = n.files - old_files + new_files;
            at = n.parent();
        }
        // Sizes changed all the way up, so each ancestor's siblings may need re-sorting.
        let mut at = self.nodes[id as usize].parent();
        while let Some(parent) = at {
            let p = &self.nodes[parent as usize];
            let range = p.child_start as usize..(p.child_start + p.child_len) as usize;
            let nodes = &self.nodes;
            self.order[range]
                .sort_unstable_by(|a, b| nodes[*b as usize].size.cmp(&nodes[*a as usize].size));
            at = p.parent();
        }
        self.summarize();
    }

    /// Gives every directory the kind holding most of the bytes below it (Other when it holds
    /// none) and the size-weighted mean modified time of those files (0 when none), so the
    /// visuals can colour folders (spec §3.6). An app bundle stays App and counts as App
    /// bytes for its parents. Runs in `finish`; run it again after removals.
    pub fn summarize(&mut self) {
        #[derive(Clone, Copy, Default)]
        struct Acc {
            bytes: [u64; 8],
            dated: u128,
            weight: u64,
        }
        let mut slot = vec![u32::MAX; self.nodes.len()];
        let mut accs: Vec<Acc> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.is_dir() {
                slot[i] = accs.len() as u32;
                accs.push(Acc::default());
            }
        }
        for id in (0..self.nodes.len()).rev() {
            let n = &self.nodes[id];
            if n.has(flag::REMOVED) {
                continue;
            }
            let mine = if n.is_dir() {
                let acc = accs[slot[id] as usize];
                let bundle = kind_of(&n.name) == Kind::App;
                let kind = if bundle {
                    Kind::App
                } else {
                    let (best, bytes) =
                        acc.bytes.iter().enumerate().fold((7, 0), |best, (k, b)| {
                            if *b > best.1 { (k, *b) } else { best }
                        });
                    if bytes == 0 {
                        Kind::Other
                    } else {
                        Kind::ALL[best]
                    }
                };
                let mtime = (acc.dated / acc.weight.max(1) as u128) as u32;
                let n = &mut self.nodes[id];
                n.kind = kind;
                n.mtime = if acc.weight == 0 { 0 } else { mtime };
                if bundle {
                    let mut flat = Acc::default();
                    flat.bytes[Kind::App as usize] = acc.bytes.iter().sum();
                    flat.dated = acc.dated;
                    flat.weight = acc.weight;
                    flat
                } else {
                    acc
                }
            } else {
                let mut acc = Acc::default();
                acc.bytes[n.kind as usize] = n.size;
                if n.mtime > 0 {
                    acc.dated = n.mtime as u128 * n.size as u128;
                    acc.weight = n.size;
                }
                acc
            };
            if let Some(p) = self.nodes[id].parent() {
                let p = &mut accs[slot[p as usize] as usize];
                for (total, b) in p.bytes.iter_mut().zip(mine.bytes) {
                    *total += b;
                }
                p.dated += mine.dated;
                p.weight += mine.weight;
            }
        }
    }

    /// Detaches `id` from the tree and subtracts its size and file count from every ancestor.
    /// Removing the root, or a node already removed, does nothing.
    pub fn remove(&mut self, id: NodeId) {
        let n = &self.nodes[id as usize];
        let Some(parent) = n.parent() else { return };
        if n.has(flag::REMOVED) {
            return;
        }
        let (size, files) = (n.size, n.files);
        self.nodes[id as usize].flags |= flag::REMOVED;

        let p = &mut self.nodes[parent as usize];
        let start = p.child_start as usize;
        let siblings = &mut self.order[start..start + p.child_len as usize];
        if let Some(pos) = siblings.iter().position(|c| *c == id) {
            siblings[pos..].rotate_left(1);
            p.child_len -= 1;
        }

        let mut at = Some(parent);
        while let Some(i) = at {
            let p = &mut self.nodes[i as usize];
            p.size -= size;
            p.files -= files;
            at = p.parent();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Other, 0)
    }

    /// /r { a { x:10, y:30 }, b:5, c { z:100 } }
    fn sample() -> (Tree, [NodeId; 6]) {
        let mut b = TreeBuilder::new(dir("/r"));
        let a = b.add_children(0, [dir("a"), file("b", 5), dir("c")]);
        let (bb, c) = (a + 1, a + 2);
        let x = b.add_children(a, [file("x", 10), file("y", 30)]);
        let y = x + 1;
        let z = b.add_children(c, [file("z", 100)]);
        (b.finish(), [a, x, y, bb, c, z])
    }

    #[test]
    fn sizes_and_counts_roll_up() {
        let (t, [a, _, _, _, c, _]) = sample();
        assert_eq!(t.node(t.root()).size, 145);
        assert_eq!(t.node(t.root()).files, 4);
        assert_eq!(t.node(a).size, 40);
        assert_eq!(t.node(a).files, 2);
        assert_eq!(t.node(c).size, 100);
    }

    #[test]
    fn children_are_largest_first() {
        let (t, [a, x, y, bb, c, _]) = sample();
        assert_eq!(t.children(t.root()), &[c, a, bb]);
        assert_eq!(t.children(a), &[y, x]);
    }

    #[test]
    fn path_rebuilds_from_root() {
        let (t, [a, _, y, ..]) = sample();
        assert_eq!(t.path(y), PathBuf::from("/r/a/y"));
        assert_eq!(t.path(a), PathBuf::from("/r/a"));
        assert_eq!(t.path(t.root()), PathBuf::from("/r"));
    }

    #[test]
    fn remove_updates_every_ancestor_and_detaches() {
        let (mut t, [a, x, y, bb, c, _]) = sample();
        t.remove(y);
        assert_eq!(t.node(a).size, 10);
        assert_eq!(t.node(a).files, 1);
        assert_eq!(t.node(t.root()).size, 115);
        assert_eq!(t.node(t.root()).files, 3);
        assert_eq!(t.children(a), &[x]);
        assert!(t.node(y).has(flag::REMOVED));
        t.remove(c);
        assert_eq!(t.node(t.root()).size, 15);
        assert_eq!(t.children(t.root()), &[a, bb]);
    }

    #[test]
    fn removing_twice_or_the_root_is_a_no_op() {
        let (mut t, [_, x, ..]) = sample();
        t.remove(x);
        t.remove(x);
        assert_eq!(t.node(t.root()).size, 135);
        t.remove(t.root());
        assert_eq!(t.node(t.root()).size, 135);
    }

    #[test]
    fn mtime_before_1970_clamps_to_zero() {
        assert_eq!(Node::new("old", 0, -5, Kind::Other, 0).mtime, 0);
    }

    fn typed(name: &str, size: u64, kind: Kind, mtime: i64) -> Node {
        Node::new(name, size, mtime, kind, 0)
    }

    /// /r { media { a.mov:100 @1000, b.jpg:10 @2000, c.jpg:10, d.jpg:10 }, empty {},
    ///      Tool.app { bin:500 @3000 }, notes.txt:1 }
    fn kinds() -> (Tree, [NodeId; 3]) {
        let mut b = TreeBuilder::new(dir("/r"));
        let media = b.add_children(
            0,
            [
                dir("media"),
                dir("empty"),
                dir("Tool.app"),
                typed("notes.txt", 1, Kind::Document, 0),
            ],
        );
        let (empty, app) = (media + 1, media + 2);
        b.add_children(
            media,
            [
                typed("a.mov", 100, Kind::Video, 1000),
                typed("b.jpg", 10, Kind::Image, 2000),
                typed("c.jpg", 10, Kind::Image, 0),
                typed("d.jpg", 10, Kind::Image, 0),
            ],
        );
        b.add_children(app, [typed("bin", 500, Kind::Other, 3000)]);
        (b.finish(), [media, empty, app])
    }

    #[test]
    fn folders_take_the_kind_with_most_bytes_not_most_files() {
        let (t, [media, ..]) = kinds();
        assert_eq!(t.node(media).kind, Kind::Video);
    }

    #[test]
    fn empty_folders_are_other_and_undated() {
        let (t, [_, empty, _]) = kinds();
        assert_eq!(t.node(empty).kind, Kind::Other);
        assert_eq!(t.node(empty).mtime, 0);
    }

    #[test]
    fn folder_age_is_weighted_by_size_and_ignores_undated_files() {
        let (t, [media, ..]) = kinds();
        // (100 * 1000 + 10 * 2000) / 110
        assert_eq!(t.node(media).mtime, 1090);
    }

    #[test]
    fn app_bundles_stay_app_and_count_as_app_upward() {
        let (t, [_, _, app]) = kinds();
        assert_eq!(t.node(app).kind, Kind::App);
        assert_eq!(t.node(t.root()).kind, Kind::App);
    }

    #[test]
    fn ties_go_to_the_first_kind() {
        let mut b = TreeBuilder::new(dir("/r"));
        b.add_children(
            0,
            [
                typed("x.png", 5, Kind::Image, 0),
                typed("y.mov", 5, Kind::Video, 0),
            ],
        );
        assert_eq!(b.finish().node(0).kind, Kind::Video);
    }

    #[test]
    fn summarize_after_removal_leaves_removed_nodes_out() {
        let (mut t, [media, _, app]) = kinds();
        t.remove(app);
        let movie = t.children(media)[0];
        t.remove(movie);
        t.summarize();
        assert_eq!(t.node(media).kind, Kind::Image);
        assert_eq!(t.node(media).mtime, 2000);
        assert_eq!(t.node(t.root()).kind, Kind::Image);
    }

    #[test]
    fn nodes_stay_small() {
        assert_eq!(std::mem::size_of::<Node>(), 48);
    }
}
