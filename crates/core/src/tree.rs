//! The size tree: an arena of nodes built once after a scan and afterwards only shrunk by
//! deletions (spec §3.3).

use crate::kind::Kind;
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
    /// Modified time, unix seconds.
    pub mtime: i64,
    /// Files at or below this node (a file counts itself).
    pub files: u32,
    parent: NodeId,
    children: Vec<NodeId>,
    pub kind: Kind,
    pub flags: u8,
}

impl Node {
    /// A new node; `size` is the entry's own allocated bytes.
    pub fn new(name: impl Into<Box<str>>, size: u64, mtime: i64, kind: Kind, flags: u8) -> Self {
        Node {
            name: name.into(),
            size,
            mtime,
            files: if flags & flag::DIR == 0 { 1 } else { 0 },
            parent: NO_PARENT,
            children: Vec::new(),
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

/// Collects nodes, parents before children, then rolls sizes up in `finish`.
pub struct TreeBuilder {
    nodes: Vec<Node>,
}

impl TreeBuilder {
    pub fn new(root: Node) -> Self {
        TreeBuilder { nodes: vec![root] }
    }

    pub fn with_capacity(root: Node, capacity: usize) -> Self {
        let mut nodes = Vec::with_capacity(capacity);
        nodes.push(root);
        TreeBuilder { nodes }
    }

    /// Adds `node` under `parent`, which must already be in the builder.
    pub fn add(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        let id = self.nodes.len() as NodeId;
        node.parent = parent;
        self.nodes.push(node);
        self.nodes[parent as usize].children.push(id);
        id
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id as usize]
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
        let sizes: Vec<u64> = self.nodes.iter().map(|n| n.size).collect();
        for n in &mut self.nodes {
            n.children
                .sort_unstable_by(|a, b| sizes[*b as usize].cmp(&sizes[*a as usize]));
        }
        Tree { nodes: self.nodes }
    }
}

#[derive(Debug, Clone)]
pub struct Tree {
    nodes: Vec<Node>,
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
        &self.nodes[id as usize].children
    }

    /// Every node ever added, including removed ones (check `flag::REMOVED`).
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn path(&self, id: NodeId) -> PathBuf {
        let mut names = Vec::new();
        let mut at = Some(id);
        while let Some(i) = at {
            let n = self.node(i);
            names.push(&*n.name);
            at = n.parent();
        }
        names.iter().rev().collect()
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
        self.nodes[parent as usize].children.retain(|c| *c != id);
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
        let a = b.add(0, dir("a"));
        let x = b.add(a, file("x", 10));
        let y = b.add(a, file("y", 30));
        let bb = b.add(0, file("b", 5));
        let c = b.add(0, dir("c"));
        let z = b.add(c, file("z", 100));
        let _ = (x, z);
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
        let (mut t, [a, _, y, _, c, _]) = sample();
        t.remove(y);
        assert_eq!(t.node(a).size, 10);
        assert_eq!(t.node(a).files, 1);
        assert_eq!(t.node(t.root()).size, 115);
        assert_eq!(t.node(t.root()).files, 3);
        assert!(!t.children(a).contains(&y));
        assert!(t.node(y).has(flag::REMOVED));
        t.remove(c);
        assert_eq!(t.node(t.root()).size, 15);
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
    fn nodes_stay_small() {
        assert!(
            std::mem::size_of::<Node>() < 100,
            "{}",
            std::mem::size_of::<Node>()
        );
    }
}
