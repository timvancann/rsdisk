//! Moving marked items to the Trash and keeping the tree in step (spec §2.11, §3.11).

use crate::tree::{NodeId, Tree};
use std::path::Path;

/// Moves one file or folder to the Trash.
pub trait Trasher {
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// The real Trash, through Finder so "Put Back" works. Slow: call it off the main thread.
pub struct FinderTrash;

impl Trasher for FinderTrash {
    fn trash(&self, path: &Path) -> Result<(), String> {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut ctx = trash::TrashContext::default();
        ctx.set_delete_method(DeleteMethod::Finder);
        ctx.delete(path).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Default)]
pub struct TrashOutcome {
    pub removed: Vec<NodeId>,
    pub failed: Vec<(NodeId, String)>,
}

/// Trashes each node's path. Successes leave the tree (sizes shrink upward and folder
/// summaries are redone once at the end); failures stay and come back with the reason.
pub fn trash_all(tree: &mut Tree, ids: &[NodeId], trasher: &dyn Trasher) -> TrashOutcome {
    let mut out = TrashOutcome::default();
    for &id in ids {
        if id == tree.root() {
            out.failed
                .push((id, "the scan root can't be trashed".into()));
            continue;
        }
        match trasher.trash(&tree.path(id)) {
            Ok(()) => {
                tree.remove(id);
                out.removed.push(id);
            }
            Err(e) => out.failed.push((id, e)),
        }
    }
    if !out.removed.is_empty() {
        tree.summarize();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;
    use crate::tree::{Node, TreeBuilder, flag};
    use std::cell::RefCell;
    use std::path::PathBuf;

    /// Records what it was asked to trash and fails on names containing "locked".
    #[derive(Default)]
    struct Fake(RefCell<Vec<PathBuf>>);

    impl Trasher for Fake {
        fn trash(&self, path: &Path) -> Result<(), String> {
            self.0.borrow_mut().push(path.to_path_buf());
            if path.to_string_lossy().contains("locked") {
                Err("permission denied".into())
            } else {
                Ok(())
            }
        }
    }

    /// /r { a { big.mov:100, small.txt:1 }, locked.zip:50 }
    fn tree() -> (Tree, [NodeId; 4]) {
        let mut b = TreeBuilder::new(Node::new("/r", 0, 0, Kind::Other, flag::DIR));
        let a = b.add_children(
            0,
            [
                Node::new("a", 0, 0, Kind::Other, flag::DIR),
                Node::new("locked.zip", 50, 0, Kind::Archive, 0),
            ],
        );
        let big = b.add_children(
            a,
            [
                Node::new("big.mov", 100, 0, Kind::Video, 0),
                Node::new("small.txt", 1, 0, Kind::Document, 0),
            ],
        );
        (b.finish(), [a, a + 1, big, big + 1])
    }

    #[test]
    fn successes_leave_the_tree_and_ancestors_shrink() {
        let (mut t, [a, _, big, _]) = tree();
        let fake = Fake::default();
        let out = trash_all(&mut t, &[big], &fake);
        assert_eq!(out.removed, [big]);
        assert!(out.failed.is_empty());
        assert_eq!(fake.0.borrow().as_slice(), [PathBuf::from("/r/a/big.mov")]);
        assert_eq!(t.node(a).size, 1);
        assert_eq!(t.node(t.root()).size, 51);
        assert!(t.node(big).has(flag::REMOVED));
    }

    #[test]
    fn failures_stay_with_their_reason() {
        let (mut t, [_, locked, big, _]) = tree();
        let out = trash_all(&mut t, &[locked, big], &Fake::default());
        assert_eq!(out.removed, [big]);
        assert_eq!(out.failed, [(locked, "permission denied".to_string())]);
        assert!(!t.node(locked).has(flag::REMOVED));
    }

    #[test]
    fn the_root_is_refused_without_asking() {
        let (mut t, _) = tree();
        let fake = Fake::default();
        let root = t.root();
        let out = trash_all(&mut t, &[root], &fake);
        assert_eq!(out.failed.len(), 1);
        assert!(fake.0.borrow().is_empty());
    }

    #[test]
    fn folder_colours_follow_the_removal() {
        let (mut t, [a, _, big, _]) = tree();
        assert_eq!(t.node(a).kind, Kind::Video);
        trash_all(&mut t, &[big], &Fake::default());
        assert_eq!(t.node(a).kind, Kind::Document);
    }

    #[test]
    #[ignore = "moves a real temp file to the Trash through Finder"]
    fn finder_trashes_a_real_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("rsdisk-trash-test.txt");
        std::fs::write(&file, b"bye").unwrap();
        FinderTrash.trash(&file).unwrap();
        assert!(!file.exists());
    }
}
