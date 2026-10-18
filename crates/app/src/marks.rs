//! What's marked for the Trash (spec §3.11). Marks are nodes; a marked folder covers
//! everything inside it, so nothing is counted twice.

use rsdisk_core::tree::{NodeId, Tree, flag};
use std::collections::BTreeSet;

#[derive(Debug, Default, Clone)]
pub struct Marks {
    ids: BTreeSet<NodeId>,
}

impl Marks {
    /// Marks or unmarks `id`. The root can't be marked, an item inside a marked folder is
    /// already covered, and marking a folder drops the marks inside it.
    pub fn toggle(&mut self, tree: &Tree, id: NodeId) {
        if id == tree.root() || tree.node(id).has(flag::REMOVED) {
            return;
        }
        if self.ids.remove(&id) {
            return;
        }
        if self.covered_by_ancestor(tree, id) {
            return;
        }
        self.ids.retain(|m| !is_below(tree, *m, id));
        self.ids.insert(id);
    }

    pub fn clear(&mut self) {
        self.ids.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_marked(&self, id: NodeId) -> bool {
        self.ids.contains(&id)
    }

    /// Whether `id` or a folder above it is marked.
    pub fn covered(&self, tree: &Tree, id: NodeId) -> bool {
        self.is_marked(id) || self.covered_by_ancestor(tree, id)
    }

    fn covered_by_ancestor(&self, tree: &Tree, id: NodeId) -> bool {
        let mut at = tree.node(id).parent();
        while let Some(p) = at {
            if self.ids.contains(&p) {
                return true;
            }
            at = tree.node(p).parent();
        }
        false
    }

    /// The marked nodes still in the tree.
    pub fn ids(&self, tree: &Tree) -> Vec<NodeId> {
        self.ids
            .iter()
            .copied()
            .filter(|id| !tree.node(*id).has(flag::REMOVED))
            .collect()
    }

    pub fn total_size(&self, tree: &Tree) -> u64 {
        self.ids(tree).iter().map(|id| tree.node(*id).size).sum()
    }

    pub fn total_files(&self, tree: &Tree) -> u64 {
        self.ids(tree)
            .iter()
            .map(|id| tree.node(*id).files as u64)
            .sum()
    }

    /// Forgets marks on nodes that left the tree.
    pub fn prune(&mut self, tree: &Tree) {
        self.ids.retain(|id| !tree.node(*id).has(flag::REMOVED));
    }
}

/// Whether `id` lies strictly below `folder`.
fn is_below(tree: &Tree, id: NodeId, folder: NodeId) -> bool {
    let mut at = tree.node(id).parent();
    while let Some(p) = at {
        if p == folder {
            return true;
        }
        at = tree.node(p).parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsdisk_core::kind::Kind;
    use rsdisk_core::tree::{Node, TreeBuilder};

    fn dir(name: &str) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Other, 0)
    }

    /// /r { a { x:10, y:20 }, b:5 }
    fn tree() -> (Tree, [NodeId; 4]) {
        let mut b = TreeBuilder::new(dir("/r"));
        let a = b.add_children(0, [dir("a"), file("b", 5)]);
        let x = b.add_children(a, [file("x", 10), file("y", 20)]);
        (b.finish(), [a, a + 1, x, x + 1])
    }

    #[test]
    fn toggling_marks_and_unmarks() {
        let (t, [_, b, ..]) = tree();
        let mut m = Marks::default();
        m.toggle(&t, b);
        assert!(m.is_marked(b));
        m.toggle(&t, b);
        assert!(m.is_empty());
    }

    #[test]
    fn the_root_cannot_be_marked() {
        let (t, _) = tree();
        let mut m = Marks::default();
        m.toggle(&t, t.root());
        assert!(m.is_empty());
    }

    #[test]
    fn marking_a_folder_absorbs_marks_inside_it() {
        let (t, [a, b, x, y]) = tree();
        let mut m = Marks::default();
        m.toggle(&t, x);
        m.toggle(&t, y);
        m.toggle(&t, b);
        m.toggle(&t, a);
        assert_eq!(m.ids(&t), [a, b]);
        assert!(m.covered(&t, x));
    }

    #[test]
    fn items_inside_a_marked_folder_are_left_alone() {
        let (t, [a, _, x, _]) = tree();
        let mut m = Marks::default();
        m.toggle(&t, a);
        m.toggle(&t, x);
        assert_eq!(m.ids(&t), [a]);
    }

    #[test]
    fn totals_count_each_byte_once() {
        let (t, [a, b, x, _]) = tree();
        let mut m = Marks::default();
        m.toggle(&t, x);
        m.toggle(&t, a);
        m.toggle(&t, b);
        assert_eq!(m.total_size(&t), 35);
        assert_eq!(m.total_files(&t), 3);
    }

    #[test]
    fn removed_nodes_drop_out() {
        let (mut t, [_, b, x, _]) = tree();
        let mut m = Marks::default();
        m.toggle(&t, b);
        m.toggle(&t, x);
        t.remove(b);
        assert_eq!(m.ids(&t), [x]);
        m.prune(&t);
        assert_eq!(m.len(), 1);
        m.toggle(&t, b);
        assert_eq!(m.len(), 1);
    }
}
