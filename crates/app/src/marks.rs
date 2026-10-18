//! What's marked for the Trash (spec §3.11). Marks are nodes; a marked folder covers
//! everything inside it, so nothing is counted twice.

use rsdisk_core::tree::{NodeId, Tree, flag};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Default, Clone)]
pub struct Marks {
    ids: BTreeSet<NodeId>,
    /// Nodes that must never go to the Trash, with why; see `guard`.
    guarded: Vec<(NodeId, &'static str)>,
}

impl Marks {
    /// Marks or unmarks `id`, or returns why it can't be marked. The root can't be marked,
    /// nor a guarded item or a folder holding one; an item inside a marked folder is already
    /// covered, and marking a folder drops the marks inside it.
    pub fn toggle(&mut self, tree: &Tree, id: NodeId) -> Option<String> {
        if id == tree.root() || tree.node(id).has(flag::REMOVED) {
            return None;
        }
        if self.ids.remove(&id) {
            return None;
        }
        if self.covered_by_ancestor(tree, id) {
            return None;
        }
        if let Some(why) = self.refusal(tree, id) {
            return Some(why);
        }
        self.ids.retain(|m| !is_below(tree, *m, id));
        self.ids.insert(id);
        None
    }

    /// Guards against trashing cloud folders (`cloud`, each folder directly inside them, and
    /// everything holding them), folders that weren't scanned, and other volumes. Their size
    /// shows as 0 B or is unknown, so a mark on them would look harmless; trashing iCloud
    /// Drive or a Google Drive account deletes files on every device. Call again whenever the
    /// tree changes.
    pub fn guard(&mut self, tree: &Tree, cloud: &[PathBuf]) {
        let mut guarded = Vec::new();
        for id in 0..tree.len() as NodeId {
            let n = tree.node(id);
            if n.has(flag::REMOVED) {
                continue;
            }
            if n.has(flag::NOT_SCANNED) {
                guarded.push((id, "not scanned"));
            } else if n.has(flag::OTHER_VOLUME) {
                guarded.push((id, "on another volume"));
            }
        }
        let root = tree.path(tree.root());
        for c in cloud {
            let folder = if root.starts_with(c) {
                // Scanning from inside a cloud folder: only its own folders are in the tree.
                (&root == c).then(|| tree.root())
            } else {
                tree.find(c)
            };
            if let Some(f) = folder {
                guarded.push((f, "a cloud folder"));
                guarded.extend(tree.children(f).iter().map(|&c| (c, "a cloud folder")));
            }
        }
        self.guarded = guarded;
        self.ids.retain(|m| {
            !self
                .guarded
                .iter()
                .any(|(g, _)| g == m || is_below(tree, *g, *m))
        });
    }

    fn refusal(&self, tree: &Tree, id: NodeId) -> Option<String> {
        let name = &tree.node(id).name;
        self.guarded
            .iter()
            .filter(|(g, _)| !tree.node(*g).has(flag::REMOVED))
            .find_map(|&(g, why)| {
                if g == id {
                    Some(format!("Can't mark {name}: it is {why}"))
                } else if is_below(tree, g, id) {
                    let inner = &tree.node(g).name;
                    Some(format!(
                        "Can't mark {name}: it holds {inner}, which is {why}"
                    ))
                } else {
                    None
                }
            })
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
pub(crate) fn is_below(tree: &Tree, id: NodeId, folder: NodeId) -> bool {
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

    /// /r { Library { CloudStorage { GoogleDrive-x { doc:7 } }, Caches:9 }, vol, a:3 }
    fn cloud_tree() -> (Tree, [NodeId; 6]) {
        let mut b = TreeBuilder::new(dir("/r"));
        let mut vol = dir("vol");
        vol.flags |= flag::OTHER_VOLUME;
        let lib = b.add_children(0, [dir("Library"), vol, file("a", 3)]);
        let cloud = b.add_children(lib, [dir("CloudStorage"), file("Caches", 9)]);
        let drive = b.add_children(cloud, [dir("GoogleDrive-x")]);
        let doc = b.add_children(drive, [file("doc", 7)]);
        (b.finish(), [lib, lib + 1, cloud, cloud + 1, drive, doc])
    }

    #[test]
    fn cloud_folders_and_what_holds_them_cannot_be_marked() {
        let (t, [lib, vol, cloud, caches, drive, doc]) = cloud_tree();
        let mut m = Marks::default();
        m.guard(&t, &[PathBuf::from("/r/Library/CloudStorage")]);
        for id in [lib, cloud, drive, vol] {
            assert!(m.toggle(&t, id).is_some(), "{}", t.node(id).name);
        }
        assert!(m.is_empty());
        let why = m.toggle(&t, lib).unwrap();
        assert!(why.contains("CloudStorage"), "{why}");
        // Next to and deep inside a cloud folder is fine.
        assert_eq!(m.toggle(&t, caches), None);
        assert_eq!(m.toggle(&t, doc), None);
        assert_eq!(m.ids(&t), [caches, doc]);
    }

    #[test]
    fn guarding_drops_marks_it_now_refuses() {
        let (t, [lib, ..]) = cloud_tree();
        let mut m = Marks::default();
        m.toggle(&t, lib);
        m.guard(&t, &[PathBuf::from("/r/Library/CloudStorage")]);
        assert!(m.is_empty());
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
