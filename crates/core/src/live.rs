//! The scan so far, a few levels deep, for the window to draw while the walk runs (spec §3.2).
//! dua shows its list filling in; rsdisk shows the list and the visual doing the same.

use crate::kind::{Kind, kind_of};
use crate::tree::{Node, NodeId, Tree, TreeBuilder, flag};
use dua_core::Entry;
use std::collections::HashMap;
use std::path::Path;

/// Folders this deep or less get their own node, as many levels as the sunburst has rings.
/// Anything deeper, and every file below the first level, adds to the folder holding it.
pub const DEPTH: usize = 5;

/// What a set of entries adds up to: what `Tree::summarize` works out from them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Sum {
    bytes: u64,
    files: u64,
    /// File bytes by `Kind as usize`.
    kinds: [u64; 8],
    /// Modified time times size over the dated files, and their size.
    dated: u128,
    weight: u64,
}

impl Sum {
    fn of(node: &Node) -> Sum {
        let mut sum = Sum {
            bytes: node.size,
            ..Sum::default()
        };
        if !node.is_dir() {
            sum.files = 1;
            sum.kinds[node.kind as usize] = node.size;
            if node.mtime > 0 {
                sum.dated = node.mtime as u128 * node.size as u128;
                sum.weight = node.size;
            }
        }
        sum
    }

    fn add(&mut self, other: &Sum) {
        self.bytes += other.bytes;
        self.files += other.files;
        for (a, b) in self.kinds.iter_mut().zip(other.kinds) {
            *a += b;
        }
        self.dated += other.dated;
        self.weight += other.weight;
    }

    /// The folder kind `Tree::summarize` would give a folder `name` holding this.
    fn kind(&self, name: &str) -> Kind {
        if kind_of(name) == Kind::App {
            return Kind::App;
        }
        let (best, bytes) =
            self.kinds.iter().enumerate().fold(
                (7, 0),
                |best, (k, b)| {
                    if *b > best.1 { (k, *b) } else { best }
                },
            );
        if bytes == 0 {
            Kind::Other
        } else {
            Kind::ALL[best]
        }
    }

    fn mtime(&self) -> u32 {
        if self.weight == 0 {
            0
        } else {
            (self.dated / self.weight as u128) as u32
        }
    }
}

/// A node of the scan so far: its own entry plus everything folded into it.
#[derive(Debug, Clone)]
pub(crate) struct LiveNode {
    name: Box<str>,
    parent: usize,
    dir: bool,
    /// The entry's own kind and age, which a file keeps.
    kind: Kind,
    mtime: u32,
    sum: Sum,
}

/// An entry on its way into the scan so far; `name` is set when it gets a node of its own.
struct Arrival {
    name: Option<Box<str>>,
    /// The walk directory ids of its parent and, for a folder, itself.
    parent: Option<usize>,
    dir: Option<usize>,
    is_dir: bool,
    kind: Kind,
    mtime: u32,
    sum: Sum,
}

/// Builds the scan so far as entries arrive. The walk can hand over a folder's contents
/// before the folder itself, so entries whose folder isn't placed yet wait for it.
pub(crate) struct Live {
    pub(crate) nodes: Vec<LiveNode>,
    /// By walk directory id: the node collecting what's in that folder.
    holder: Vec<Option<usize>>,
    waiting: HashMap<usize, Vec<Arrival>>,
}

impl Default for Live {
    fn default() -> Self {
        let root = LiveNode {
            name: "".into(),
            parent: 0,
            dir: true,
            kind: Kind::Other,
            mtime: 0,
            sum: Sum::default(),
        };
        // The walk numbers the root's directory 0.
        Live {
            nodes: vec![root],
            holder: vec![Some(0)],
            waiting: HashMap::new(),
        }
    }
}

impl Live {
    pub(crate) fn add(&mut self, entry: &Entry, node: &Node) {
        let own = entry.depth == 1 || (node.is_dir() && (1..=DEPTH).contains(&entry.depth));
        self.place(Arrival {
            name: own.then(|| node.name.clone()),
            parent: entry.parent_directory_id.map(|p| p.index()),
            dir: entry.directory_id.map(|d| d.index()),
            is_dir: node.is_dir(),
            kind: node.kind,
            mtime: node.mtime,
            sum: Sum::of(node),
        });
    }

    fn place(&mut self, a: Arrival) {
        let mine = match a.parent {
            None => {
                self.nodes[0].sum.add(&a.sum);
                0
            }
            Some(p) => {
                let Some(at) = self.holder.get(p).copied().flatten() else {
                    self.waiting.entry(p).or_default().push(a);
                    return;
                };
                match a.name {
                    Some(name) => {
                        self.nodes.push(LiveNode {
                            name,
                            parent: at,
                            dir: a.is_dir,
                            kind: a.kind,
                            mtime: a.mtime,
                            sum: a.sum,
                        });
                        self.nodes.len() - 1
                    }
                    None => {
                        self.nodes[at].sum.add(&a.sum);
                        at
                    }
                }
            }
        };
        if let Some(d) = a.dir {
            if self.holder.len() <= d {
                self.holder.resize(d + 1, None);
            }
            self.holder[d] = Some(mine);
            for waiting in self.waiting.remove(&d).unwrap_or_default() {
                self.place(waiting);
            }
        }
    }
}

/// The scan so far as a tree whose sizes, kinds and ages match what the finished scan will
/// show for the same folders, given the same entries.
pub(crate) fn tree(nodes: &[LiveNode], root: &Path) -> Tree {
    // Everything below each node; parents come before their children.
    let mut below: Vec<Sum> = nodes.iter().map(|n| n.sum).collect();
    for i in (1..nodes.len()).rev() {
        let mut sum = below[i];
        // An app bundle counts as App for its parents, as in `Tree::summarize`.
        if nodes[i].dir && kind_of(&nodes[i].name) == Kind::App {
            sum.kinds = [0; 8];
            sum.kinds[Kind::App as usize] = below[i].kinds.iter().sum();
        }
        below[nodes[i].parent].add(&sum);
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, n) in nodes.iter().enumerate().skip(1) {
        children[n.parent].push(i);
    }
    let make = |i: usize, name: Box<str>| {
        let n = &nodes[i];
        let flags = if n.dir { flag::DIR } else { 0 };
        let mut node = Node::new(name, n.sum.bytes, 0, n.kind, flags);
        node.files = n.sum.files as u32;
        node.mtime = n.mtime;
        node
    };
    let mut builder =
        TreeBuilder::with_capacity(make(0, root.to_string_lossy().into()), nodes.len());
    let mut ids: Vec<NodeId> = vec![0; nodes.len()];
    let mut stack = vec![0];
    while let Some(i) = stack.pop() {
        if children[i].is_empty() {
            continue;
        }
        let first = builder.add_children(
            ids[i],
            children[i].iter().map(|&c| make(c, nodes[c].name.clone())),
        );
        for (k, &c) in children[i].iter().enumerate() {
            ids[c] = first + k as NodeId;
            stack.push(c);
        }
    }
    let mut tree = builder.finish();
    // `finish` only saw the files kept as nodes; the sums saw them all.
    for (i, &id) in ids.iter().enumerate() {
        if nodes[i].dir {
            let n = tree.node_mut(id);
            n.kind = below[i].kind(&n.name);
            n.mtime = below[i].mtime();
        }
    }
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(bytes: u64) -> Sum {
        Sum {
            bytes,
            files: 1,
            ..Sum::default()
        }
    }

    fn arrival(name: Option<&str>, parent: usize, dir: Option<usize>, sum: Sum) -> Arrival {
        Arrival {
            name: name.map(Into::into),
            parent: Some(parent),
            dir,
            is_dir: dir.is_some(),
            kind: Kind::Other,
            mtime: 0,
            sum,
        }
    }

    #[test]
    fn contents_that_arrive_before_their_folder_still_count() {
        let mut live = Live::default();
        // a/b/c.txt, where a has dir id 1 and b has 2, arrives first, then b, then a.
        live.place(arrival(None, 2, None, file(10)));
        live.place(arrival(Some("b"), 1, Some(2), Sum::default()));
        assert_eq!(live.nodes.len(), 1);
        live.place(arrival(Some("a"), 0, Some(1), Sum::default()));
        assert!(live.waiting.is_empty());
        let names: Vec<&str> = live.nodes.iter().map(|n| &*n.name).collect();
        assert_eq!(names, ["", "a", "b"]);
        assert_eq!(live.nodes[2].parent, 1);
        assert_eq!(live.nodes[2].sum, file(10));
    }
}
