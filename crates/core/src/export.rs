//! A pruned copy of the tree for the HTML mockup (spec §3.7): the largest files and folders
//! plus their ancestors, with one `(other)` block per folder for everything left out.

use crate::tree::{NodeId, Tree, flag};
use serde::Serialize;
use std::collections::HashSet;

/// How many of the largest files and folders an export keeps (spec §3.7).
pub const FILES: usize = 100;
pub const DIRS: usize = 100;

#[derive(Debug, Serialize)]
pub struct ExportNode {
    pub name: String,
    pub size: u64,
    pub files: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    pub mtime: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ExportNode>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub other: bool,
}

/// The exported file: the pruned root plus facts about the scan.
#[derive(Debug, Serialize)]
pub struct Export {
    pub path: String,
    /// Unix seconds.
    pub scanned_at: u64,
    pub elapsed_ms: u64,
    pub errors: usize,
    #[serde(flatten)]
    pub root: ExportNode,
}

/// The `files` largest files and `dirs` largest directories, every ancestor of those, and an
/// `(other)` child wherever kept children don't add up to their parent.
pub fn prune(tree: &Tree, files: usize, dirs: usize) -> ExportNode {
    let root = tree.root();
    let live = |id: &NodeId| *id != root && !tree.node(*id).has(flag::REMOVED);
    let all: Vec<NodeId> = (0..tree.len() as NodeId).filter(live).collect();
    let mut keep: HashSet<NodeId> = HashSet::from([root]);
    for (want_dir, n) in [(false, files), (true, dirs)] {
        let mut picked: Vec<NodeId> = all
            .iter()
            .copied()
            .filter(|id| tree.node(*id).is_dir() == want_dir)
            .collect();
        if picked.len() > n {
            picked.select_nth_unstable_by(n, |a, b| tree.node(*b).size.cmp(&tree.node(*a).size));
            picked.truncate(n);
        }
        for mut id in picked {
            while keep.insert(id) {
                match tree.node(id).parent() {
                    Some(p) => id = p,
                    None => break,
                }
            }
        }
    }
    export_node(tree, root, &keep)
}

fn export_node(tree: &Tree, id: NodeId, keep: &HashSet<NodeId>) -> ExportNode {
    let node = tree.node(id);
    let children = node.is_dir().then(|| {
        let mut kept: Vec<ExportNode> = tree
            .children(id)
            .iter()
            .filter(|c| keep.contains(c))
            .map(|c| export_node(tree, *c, keep))
            .collect();
        let size: u64 = kept.iter().map(|c| c.size).sum();
        let files: u32 = kept.iter().map(|c| c.files).sum();
        if node.size > size {
            kept.push(ExportNode {
                name: "(other)".into(),
                size: node.size - size,
                files: node.files - files,
                kind: None,
                mtime: 0,
                children: None,
                other: true,
            });
        }
        kept
    });
    ExportNode {
        name: node.name.to_string(),
        size: node.size,
        files: node.files,
        kind: (!node.is_dir()).then(|| node.kind.label()),
        mtime: node.mtime,
        children,
        other: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::Kind;
    use crate::tree::{Node, TreeBuilder};

    fn dir(name: &str) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Video, 0)
    }

    /// /r { a { f1:100, f2:1, f3:2 }, b { c { f4:50, f5:3 } }, f6:7 }
    fn sample() -> Tree {
        let mut t = TreeBuilder::new(dir("/r"));
        let a = t.add_children(0, [dir("a"), dir("b"), file("f6", 7)]);
        let b = a + 1;
        t.add_children(a, [file("f1", 100), file("f2", 1), file("f3", 2)]);
        let c = t.add_children(b, [dir("c")]);
        t.add_children(c, [file("f4", 50), file("f5", 3)]);
        t.finish()
    }

    fn find<'a>(n: &'a ExportNode, path: &[&str]) -> Option<&'a ExportNode> {
        match path.split_first() {
            None => Some(n),
            Some((first, rest)) => n
                .children
                .as_ref()?
                .iter()
                .find(|c| c.name == *first)
                .and_then(|c| find(c, rest)),
        }
    }

    fn assert_adds_up(n: &ExportNode) {
        if let Some(children) = &n.children {
            assert_eq!(
                children.iter().map(|c| c.size).sum::<u64>(),
                n.size,
                "{}",
                n.name
            );
            assert_eq!(
                children.iter().map(|c| c.files).sum::<u32>(),
                n.files,
                "{}",
                n.name
            );
            children.iter().for_each(assert_adds_up);
        }
    }

    #[test]
    fn largest_files_and_dirs_are_kept_with_their_ancestors() {
        let e = prune(&sample(), 2, 1);
        assert!(find(&e, &["a", "f1"]).is_some());
        assert!(find(&e, &["b", "c", "f4"]).is_some());
        assert!(find(&e, &["a"]).is_some());
        assert!(find(&e, &["f6"]).is_none());
        assert!(find(&e, &["a", "f2"]).is_none());
    }

    #[test]
    fn other_holds_what_was_left_out() {
        let e = prune(&sample(), 2, 1);
        let other = find(&e, &["a", "(other)"]).unwrap();
        assert!(other.other);
        assert_eq!(other.size, 3);
        assert_eq!(other.files, 2);
        let root_other = find(&e, &["(other)"]).unwrap();
        assert_eq!(root_other.size, 7);
    }

    #[test]
    fn no_other_when_nothing_is_left_out() {
        // b's only child, c, is kept (as f4's parent), so b has nothing left over.
        let e = prune(&sample(), 2, 1);
        let b = find(&e, &["b"]).unwrap();
        assert_eq!(b.children.as_ref().unwrap().len(), 1);
        assert!(find(&e, &["b", "(other)"]).is_none());
    }

    #[test]
    fn sizes_add_up_at_every_level() {
        assert_adds_up(&prune(&sample(), 2, 1));
        assert_adds_up(&prune(&sample(), 0, 0));
    }

    #[test]
    fn a_small_tree_is_exported_whole() {
        let e = prune(&sample(), 100, 100);
        fn no_other(n: &ExportNode) -> bool {
            !n.other && n.children.iter().flatten().all(no_other)
        }
        assert!(no_other(&e));
        assert!(find(&e, &["b", "c", "f5"]).is_some());
    }

    #[test]
    fn removed_nodes_are_left_out() {
        let mut t = sample();
        let f1 = (0..t.len() as NodeId)
            .find(|id| &*t.node(*id).name == "f1")
            .unwrap();
        t.remove(f1);
        let e = prune(&t, 100, 100);
        assert!(find(&e, &["a", "f1"]).is_none());
        assert_adds_up(&e);
    }

    #[test]
    fn json_shape() {
        let e = Export {
            path: "/r".into(),
            scanned_at: 1_700_000_000,
            elapsed_ms: 12,
            errors: 0,
            root: prune(&sample(), 2, 1),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["path"], "/r");
        assert_eq!(v["size"], 163);
        assert_eq!(v["errors"], 0);
        assert!(v.get("kind").is_none());
        assert!(v.get("other").is_none());
        let f1 = &v["children"][0]["children"][0];
        assert_eq!(f1["name"], "f1");
        assert_eq!(f1["kind"], "video");
        assert!(f1.get("children").is_none());
    }
}
