//! The parallel walk (spec §3.2). dua-core does the walking: its workers list directories in
//! parallel with macOS's bulk attribute call and hand entries to this thread, which applies
//! rsdisk's size rules and groups them by directory. The groups become a `Tree` at the end.

use crate::kind::{Kind, kind_of};
use crate::tree::{Node, NodeId, Tree, TreeBuilder, flag};
use dua_core::{Entry, Order};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Folders that appear in the tree but are not descended into (spec §2.8).
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub skip: Vec<PathBuf>,
}

impl ScanOptions {
    /// The cloud folders under a home folder: Google Drive, Dropbox and friends, and iCloud.
    pub fn for_home(home: &Path) -> Self {
        ScanOptions {
            skip: vec![
                home.join("Library/CloudStorage"),
                home.join("Library/Mobile Documents"),
            ],
        }
    }
}

/// Live counters for the UI to poll while a scan runs, plus the cancel switch.
#[derive(Debug, Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    latest: Mutex<PathBuf>,
    tops: Mutex<Vec<Top>>,
    cancel: AtomicBool,
}

/// Running totals for one item directly under the scan root, so the window can show where
/// the space is while the walk runs, as dua does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Top {
    pub name: Box<str>,
    pub dir: bool,
    pub sum: Sum,
}

/// What a set of entries adds up to: what `Tree::summarize` would work out from them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sum {
    pub bytes: u64,
    pub files: u64,
    /// File bytes by `Kind as usize`.
    pub kinds: [u64; 8],
    /// Modified time times size over the dated files, and their size.
    pub dated: u128,
    pub weight: u64,
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
}

impl Progress {
    /// The items under the root so far, in the order the walk met them.
    pub fn tops(&self) -> Vec<Top> {
        self.tops.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// The scan so far as a tree one level deep, for the window to show while the walk runs.
    /// Folders get the kind and age their contents so far give them.
    pub fn tree_so_far(&self, root: &Path) -> Tree {
        let tops = self.tops();
        let root_node = Node::new(root.to_string_lossy(), 0, 0, Kind::Other, flag::DIR);
        let mut builder = TreeBuilder::with_capacity(root_node, tops.len() + 1);
        builder.add_children(
            0,
            tops.iter().map(|t| {
                let flags = if t.dir { flag::DIR } else { 0 };
                let mut n = Node::new(t.name.clone(), t.sum.bytes, 0, Kind::Other, flags);
                n.files = t.sum.files as u32;
                n
            }),
        );
        let mut tree = builder.finish();
        // `finish` rolled sizes up but found no files to colour by; give each item its own.
        let by_name: std::collections::HashMap<&str, &Sum> =
            tops.iter().map(|t| (&*t.name, &t.sum)).collect();
        let children = tree.children(tree.root()).to_vec();
        for id in children {
            let n = tree.node_mut(id);
            let Some(sum) = by_name.get(&*n.name) else {
                continue;
            };
            let (best, bytes) =
                sum.kinds.iter().enumerate().fold(
                    (7, 0),
                    |best, (k, b)| {
                        if *b > best.1 { (k, *b) } else { best }
                    },
                );
            n.kind = if kind_of(&n.name) == Kind::App {
                Kind::App
            } else if bytes == 0 {
                Kind::Other
            } else {
                Kind::ALL[best]
            };
            n.mtime = (sum.dated / sum.weight.max(1) as u128) as u32;
        }
        tree
    }

    /// A directory the scan recently passed through.
    pub fn latest(&self) -> PathBuf {
        self.latest.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug)]
pub struct ScanResult {
    pub tree: Tree,
    pub errors: Vec<ScanError>,
    pub elapsed: Duration,
    pub cancelled: bool,
    /// Directories that couldn't be opened but aren't in `errors` or flagged yet, because the
    /// walk doesn't say which ones they were. `locate_unreadable` finds them.
    pub unlocated: usize,
}

impl ScanResult {
    /// Every error, counting the unlocated ones.
    pub fn error_count(&self) -> usize {
        self.errors.len() + self.unlocated
    }
}

/// Whether an entry on `dev` lies on another volume than the scan root.
pub fn crosses_volume(root_dev: u64, dev: u64) -> bool {
    root_dev != dev
}

/// How often (in entries) the consumer refreshes `Progress::latest`.
const LATEST_EVERY: u64 = 4096;

/// How often the consumer publishes `Progress::tops`.
const TOPS_EVERY: Duration = Duration::from_millis(100);

/// A listed entry waiting for the tree: its node, and its walk directory id when it is a
/// directory that was entered.
type Pending = (Node, Option<usize>);

/// Adds each entry to the item under the root that holds it. The walk can hand over a
/// folder's contents before the folder itself, so entries whose item isn't known yet wait,
/// and `settle` places them once the chain up to the root has arrived.
#[derive(Default)]
struct Tops {
    items: Vec<Top>,
    /// By walk directory id: the item it belongs to, and its parent's directory id.
    top_of: Vec<Option<usize>>,
    parent_of: Vec<Option<usize>>,
    /// Entries whose item isn't known yet, by the directory id they are in.
    waiting: Vec<(usize, Sum)>,
}

impl Tops {
    fn add(&mut self, entry: &Entry, node: &Node) {
        let sum = Sum::of(node);
        let dir = entry.directory_id.map(|d| d.index());
        if let Some(d) = dir
            && self.top_of.len() <= d
        {
            self.top_of.resize(d + 1, None);
            self.parent_of.resize(d + 1, None);
        }
        match (entry.depth, entry.parent_directory_id.map(|p| p.index())) {
            (0, _) | (_, None) => {}
            (1, _) => {
                if let Some(d) = dir {
                    self.top_of[d] = Some(self.items.len());
                }
                self.items.push(Top {
                    name: node.name.clone(),
                    dir: node.is_dir(),
                    sum,
                });
            }
            (_, Some(p)) => {
                if let Some(d) = dir {
                    self.parent_of[d] = Some(p);
                    self.top_of[d] = self.top_of.get(p).copied().flatten();
                }
                self.credit(p, sum);
            }
        }
    }

    fn credit(&mut self, dir: usize, sum: Sum) {
        match self.top_of.get(dir).copied().flatten() {
            Some(t) => self.items[t].sum.add(&sum),
            None => self.waiting.push((dir, sum)),
        }
    }

    /// Places what's waiting wherever its chain up to the root is now complete.
    fn settle(&mut self) {
        for (dir, sum) in std::mem::take(&mut self.waiting) {
            self.resolve(dir);
            self.credit(dir, sum);
        }
    }

    /// Finds the item `dir` belongs to and remembers it along the way; None while a link is
    /// missing.
    fn resolve(&mut self, dir: usize) -> Option<()> {
        let mut chain = Vec::new();
        let mut at = dir;
        let top = loop {
            if let Some(t) = self.top_of.get(at).copied().flatten() {
                break t;
            }
            chain.push(at);
            at = self.parent_of.get(at).copied().flatten()?;
        };
        for d in chain {
            self.top_of[d] = Some(top);
        }
        Some(())
    }

    fn publish(&mut self, progress: &Progress) {
        self.settle();
        if let Ok(mut tops) = progress.tops.lock() {
            tops.clone_from(&self.items);
        }
    }
}

pub fn scan(root: &Path, opts: &ScanOptions, progress: &Progress) -> ScanResult {
    let start = Instant::now();
    let root_meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) => {
            return finish_early(root, flag::DIR | flag::UNREADABLE, Some(e), start, false);
        }
    };
    if progress.is_cancelled() {
        return finish_early(root, flag::DIR, None, start, true);
    }

    let root_dev = root_meta.dev();
    let skip: Arc<[PathBuf]> = opts.skip.clone().into();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let descend = move |e: &Entry| {
        e.depth == 0
            || (matches!(&e.metadata, Some(Ok(m)) if !crosses_volume(root_dev, m.dev()))
                && !skip.contains(&e.path()))
    };
    let walk = dua_core::walk(
        root,
        threads,
        Order::Completion,
        Default::default(),
        descend,
    );

    let mut root_node = None;
    // Children by the walk's directory id of their parent.
    let mut pending: Vec<Vec<Pending>> = Vec::new();
    let mut errors = Vec::new();
    let mut open_failures = 0usize;
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut count = 0u64;
    let mut tops = Tops::default();
    let mut published = Instant::now();

    for item in walk {
        if progress.is_cancelled() {
            break;
        }
        let entry = match item {
            Ok(e) => e,
            // A directory that couldn't be opened; dua-core doesn't say which (see `locate_unreadable`).
            Err(_) => {
                open_failures += 1;
                continue;
            }
        };
        count += 1;
        if count.is_multiple_of(LATEST_EVERY)
            && let Ok(mut latest) = progress.latest.try_lock()
        {
            latest.as_mut_os_string().clear();
            latest.push(&*entry.parent_path);
        }

        let is_root = entry.depth == 0;
        let name: Box<str> = if is_root {
            root.to_string_lossy().into()
        } else {
            entry.file_name.to_string_lossy().into()
        };
        let mut node = node_for(name, &entry, root_dev, &mut seen_inodes);
        if let Some(Err(e)) = &entry.metadata {
            errors.push(ScanError {
                path: entry.path(),
                message: e.to_string(),
            });
        }
        if node.is_dir() {
            progress.dirs.fetch_add(1, Relaxed);
        } else {
            progress.files.fetch_add(1, Relaxed);
        }
        progress.bytes.fetch_add(node.size, Relaxed);

        tops.add(&entry, &node);
        if count.is_multiple_of(256) && published.elapsed() >= TOPS_EVERY {
            tops.publish(progress);
            published = Instant::now();
        }

        if node.is_dir() && !is_root && opts.skip.contains(&entry.path()) {
            node.flags |= flag::NOT_SCANNED;
        }
        let not_entered = flag::OTHER_VOLUME | flag::UNREADABLE | flag::NOT_SCANNED;
        let entered = entry
            .directory_id
            .filter(|_| node.is_dir() && !node.has(not_entered))
            .map(|d| d.index());

        match entry.parent_directory_id {
            None => root_node = Some((node, entered)),
            Some(parent) => {
                let p = parent.index();
                if pending.len() <= p {
                    pending.resize_with(p + 1, Vec::new);
                }
                pending[p].push((node, entered));
            }
        }
    }

    tops.publish(progress);
    let cancelled = progress.is_cancelled();
    let Some(root_node) = root_node else {
        return finish_early(root, flag::DIR, None, start, cancelled);
    };
    let tree = build(root_node, pending, count as usize).finish();
    errors.sort_by(|a, b| a.path.cmp(&b.path));
    ScanResult {
        tree,
        errors,
        elapsed: start.elapsed(),
        cancelled,
        unlocated: open_failures,
    }
}

/// A node for one entry, without its children. Hardlinked files count once; a directory on
/// another volume counts as nothing.
fn node_for(name: Box<str>, entry: &Entry, root_dev: u64, seen: &mut HashSet<(u64, u64)>) -> Node {
    let ft = entry.file_type;
    let mut flags = 0;
    if ft.is_dir() {
        flags |= flag::DIR;
    }
    if ft.is_symlink() {
        flags |= flag::SYMLINK;
    }
    let kind = match kind_of(&name) {
        Kind::App => Kind::App,
        _ if ft.is_dir() => Kind::Other,
        k => k,
    };
    let Some(Ok(meta)) = &entry.metadata else {
        if ft.is_dir() {
            flags |= flag::UNREADABLE;
        }
        return Node::new(name, 0, 0, kind, flags);
    };
    let mut size = meta.allocated_size();
    if ft.is_dir() && crosses_volume(root_dev, meta.dev()) {
        flags |= flag::OTHER_VOLUME;
        size = 0;
    } else if !ft.is_dir() && meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
        size = 0;
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    Node::new(name, size, mtime, kind, flags)
}

/// Lays the grouped entries out as a tree, each directory's children added in one go.
fn build(root: Pending, mut pending: Vec<Vec<Pending>>, count: usize) -> TreeBuilder {
    let (root_node, root_dir) = root;
    let mut builder = TreeBuilder::with_capacity(root_node, count.max(1));
    let mut stack: Vec<(NodeId, usize)> = root_dir.map(|d| (0, d)).into_iter().collect();
    while let Some((parent, dir)) = stack.pop() {
        let children = pending.get_mut(dir).map(std::mem::take).unwrap_or_default();
        if children.is_empty() {
            continue;
        }
        let mut subdirs = Vec::new();
        let first = builder.add_children(
            parent,
            children.into_iter().enumerate().map(|(i, (node, d))| {
                if let Some(d) = d {
                    subdirs.push((i as NodeId, d));
                }
                node
            }),
        );
        stack.extend(subdirs.into_iter().map(|(i, d)| (first + i, d)));
    }
    builder
}

/// Finds the directories the walk couldn't open, flags them and adds them to the errors.
/// dua-core's error carries no path, so this re-opens every entered directory that came back
/// empty. It takes a second or two when macOS privacy settings block many folders, which is
/// why `scan` leaves it to the caller to run after showing the result (spec §3.2).
pub fn locate_unreadable(result: &mut ScanResult) {
    let found = probe_unreadable(unreadable_candidates(result));
    apply_unreadable(result, found);
}

/// The directories `locate_unreadable` re-opens: none when nothing failed to open.
pub fn unreadable_candidates(result: &ScanResult) -> Vec<(NodeId, PathBuf)> {
    if result.unlocated == 0 {
        return Vec::new();
    }
    let not_entered = flag::OTHER_VOLUME | flag::UNREADABLE | flag::NOT_SCANNED | flag::REMOVED;
    result.tree.childless_dirs(not_entered)
}

/// Re-opens each candidate and returns the ones that fail. Touches only the filesystem, so a
/// GUI can run it off the main thread.
pub fn probe_unreadable(candidates: Vec<(NodeId, PathBuf)>) -> Vec<(NodeId, ScanError)> {
    candidates
        .into_par_iter()
        .filter_map(|(id, path)| match fs::read_dir(&path) {
            Ok(_) => None,
            Err(e) => Some((
                id,
                ScanError {
                    path,
                    message: e.to_string(),
                },
            )),
        })
        .collect()
}

/// Flags what `probe_unreadable` found and moves it into the errors.
pub fn apply_unreadable(result: &mut ScanResult, found: Vec<(NodeId, ScanError)>) {
    for (id, error) in found {
        result.tree.node_mut(id).flags |= flag::UNREADABLE;
        result.errors.push(error);
    }
    result.unlocated = 0;
    result.errors.sort_by(|a, b| a.path.cmp(&b.path));
}

fn finish_early(
    root: &Path,
    flags: u8,
    error: Option<std::io::Error>,
    start: Instant,
    cancelled: bool,
) -> ScanResult {
    let node = Node::new(root.to_string_lossy(), 0, 0, Kind::Other, flags);
    ScanResult {
        tree: TreeBuilder::new(node).finish(),
        errors: error
            .map(|e| ScanError {
                path: root.to_path_buf(),
                message: e.to_string(),
            })
            .into_iter()
            .collect(),
        elapsed: start.elapsed(),
        cancelled,
        unlocated: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use tempfile::TempDir;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; bytes]).unwrap();
    }

    /// Independent `du`-style total: every entry's allocated bytes, each inode once.
    fn du(root: &Path) -> u64 {
        fn walk(p: &Path, seen: &mut HashSet<(u64, u64)>) -> u64 {
            let m = fs::symlink_metadata(p).unwrap();
            if !seen.insert((m.dev(), m.ino())) {
                return 0;
            }
            let mut total = m.blocks() * 512;
            if m.is_dir() {
                for e in fs::read_dir(p).unwrap() {
                    total += walk(&e.unwrap().path(), seen);
                }
            }
            total
        }
        walk(root, &mut HashSet::new())
    }

    fn sample() -> TempDir {
        let t = TempDir::new().unwrap();
        let r = t.path();
        write(&r.join("a/one.mov"), 300_000);
        write(&r.join("a/b/two.txt"), 5_000);
        write(&r.join("a/b/c/three.rs"), 70_000);
        write(&r.join("d/four.zip"), 1_000_000);
        write(&r.join("five"), 10);
        fs::create_dir_all(r.join("empty")).unwrap();
        t
    }

    fn run(root: &Path, opts: &ScanOptions) -> (ScanResult, Progress) {
        let p = Progress::default();
        let r = scan(root, opts, &p);
        (r, p)
    }

    fn find(tree: &Tree, path: &Path) -> Option<NodeId> {
        (0..tree.len() as NodeId).find(|id| tree.path(*id) == path)
    }

    #[test]
    fn total_matches_an_independent_du() {
        let t = sample();
        let (r, _) = run(t.path(), &ScanOptions::default());
        assert_eq!(r.tree.node(r.tree.root()).size, du(t.path()));
        assert_eq!(r.tree.node(r.tree.root()).files, 5);
        assert!(r.errors.is_empty());
        assert!(!r.cancelled);
    }

    #[test]
    fn kinds_are_assigned() {
        let t = sample();
        let (r, _) = run(t.path(), &ScanOptions::default());
        let mov = find(&r.tree, &t.path().join("a/one.mov")).unwrap();
        assert_eq!(r.tree.node(mov).kind, Kind::Video);
    }

    #[test]
    fn hardlinked_file_counts_once() {
        let t = sample();
        fs::hard_link(
            t.path().join("d/four.zip"),
            t.path().join("a/four-again.zip"),
        )
        .unwrap();
        let (r, _) = run(t.path(), &ScanOptions::default());
        assert_eq!(r.tree.node(r.tree.root()).size, du(t.path()));
        assert_eq!(r.tree.node(r.tree.root()).files, 6);
    }

    #[test]
    fn symlink_counts_only_itself() {
        let t = sample();
        symlink(t.path().join("d/four.zip"), t.path().join("link.zip")).unwrap();
        let (r, _) = run(t.path(), &ScanOptions::default());
        let link = find(&r.tree, &t.path().join("link.zip")).unwrap();
        let node = r.tree.node(link);
        assert!(node.has(flag::SYMLINK));
        assert!(node.size < 1_000_000);
        assert_eq!(r.tree.node(r.tree.root()).size, du(t.path()));
    }

    #[test]
    fn unreadable_folder_is_flagged_and_reported() {
        let t = sample();
        let locked = t.path().join("locked");
        write(&locked.join("secret"), 100);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let (mut r, _) = run(t.path(), &ScanOptions::default());
        let id = find(&r.tree, &locked).unwrap();
        assert_eq!(r.unlocated, 1);
        assert_eq!(r.error_count(), 1);
        assert!(!r.tree.node(id).has(flag::UNREADABLE));

        locate_unreadable(&mut r);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(r.tree.node(id).has(flag::UNREADABLE));
        assert_eq!(r.unlocated, 0);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].path, locked);
    }

    #[test]
    fn probe_then_apply_matches_locate() {
        let t = sample();
        let locked = t.path().join("locked");
        write(&locked.join("secret"), 100);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let (mut whole, _) = run(t.path(), &ScanOptions::default());
        let (mut split, _) = run(t.path(), &ScanOptions::default());
        locate_unreadable(&mut whole);
        let found = probe_unreadable(unreadable_candidates(&split));
        apply_unreadable(&mut split, found);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let paths = |r: &ScanResult| r.errors.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&whole), paths(&split));
        assert_eq!(split.unlocated, 0);
        let id = find(&split.tree, &locked).unwrap();
        assert!(split.tree.node(id).has(flag::UNREADABLE));
    }

    #[test]
    fn empty_folders_are_not_mistaken_for_unreadable_ones() {
        let t = sample();
        let locked = t.path().join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let (mut r, _) = run(t.path(), &ScanOptions::default());
        locate_unreadable(&mut r);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let empty = find(&r.tree, &t.path().join("empty")).unwrap();
        assert!(!r.tree.node(empty).has(flag::UNREADABLE));
    }

    #[test]
    fn skipped_folder_is_present_but_not_scanned() {
        let t = sample();
        let cloud = t.path().join("Library/CloudStorage");
        write(&cloud.join("GoogleDrive/huge.mov"), 500_000);
        let opts = ScanOptions::for_home(t.path());
        let (r, _) = run(t.path(), &opts);
        let id = find(&r.tree, &cloud).unwrap();
        let node = r.tree.node(id);
        assert!(node.has(flag::NOT_SCANNED));
        assert!(r.tree.children(id).is_empty());
        assert!(node.size < 500_000);
    }

    #[test]
    fn cancelled_scan_returns_a_partial_result() {
        let t = sample();
        let p = Progress::default();
        p.cancel();
        let r = scan(t.path(), &ScanOptions::default(), &p);
        assert!(r.cancelled);
        assert!(r.tree.children(r.tree.root()).is_empty());
    }

    #[test]
    fn progress_matches_the_tree() {
        let t = sample();
        let (r, p) = run(t.path(), &ScanOptions::default());
        let root = r.tree.node(r.tree.root());
        assert_eq!(p.files.load(Relaxed), root.files as u64);
        assert_eq!(p.bytes.load(Relaxed), root.size);
        // a, a/b, a/b/c, d, empty, plus the root itself.
        assert_eq!(p.dirs.load(Relaxed), 6);
    }

    #[test]
    fn the_tree_so_far_ends_matching_the_items_under_the_root() {
        let t = sample();
        let (r, p) = run(t.path(), &ScanOptions::default());
        let level = |tree: &Tree| {
            let mut items: Vec<_> = tree
                .children(tree.root())
                .iter()
                .map(|&id| {
                    let n = tree.node(id);
                    (n.name.clone(), n.is_dir(), n.size, n.files, n.kind, n.mtime)
                })
                .collect();
            items.sort_by(|a, b| a.0.cmp(&b.0));
            items
        };
        let so_far = p.tree_so_far(t.path());
        assert_eq!(level(&so_far), level(&r.tree));
        assert_eq!(
            so_far.node(so_far.root()).size,
            r.tree.node(r.tree.root()).size
        );
    }

    #[test]
    fn contents_that_arrive_before_their_folder_still_count() {
        let mut tops = Tops {
            top_of: vec![None; 4],
            parent_of: vec![None; 4],
            ..Default::default()
        };
        // Dir 3 (under 2, under 1) reports 10 bytes before 2 or 1 is known.
        let file = Sum {
            bytes: 10,
            files: 1,
            ..Sum::default()
        };
        tops.credit(3, file);
        tops.parent_of[3] = Some(2);
        tops.settle();
        assert_eq!(tops.waiting.len(), 1);
        tops.parent_of[2] = Some(1);
        tops.items.push(Top {
            name: "a".into(),
            dir: true,
            sum: Sum::default(),
        });
        tops.top_of[1] = Some(0);
        tops.settle();
        assert!(tops.waiting.is_empty());
        assert_eq!(tops.items[0].sum, file);
    }

    /// Size and file count of every live node, by path.
    fn totals(tree: &Tree) -> Vec<(PathBuf, u64, u32)> {
        let mut out: Vec<(PathBuf, u64, u32)> = Vec::new();
        let mut stack = vec![tree.root()];
        while let Some(id) = stack.pop() {
            let n = tree.node(id);
            out.push((tree.path(id), n.size, n.files));
            stack.extend_from_slice(tree.children(id));
        }
        out.sort();
        out
    }

    fn graft_rescan(r: &mut ScanResult, folder: &Path) {
        let id = find(&r.tree, folder).unwrap();
        let sub = scan(folder, &ScanOptions::default(), &Progress::default());
        r.tree.graft(id, sub.tree);
    }

    #[test]
    fn a_grafted_rescan_matches_a_fresh_scan() {
        let t = sample();
        let (mut r, _) = run(t.path(), &ScanOptions::default());
        write(&t.path().join("a/b/new.mov"), 400_000);
        fs::remove_file(t.path().join("a/b/c/three.rs")).unwrap();
        graft_rescan(&mut r, &t.path().join("a/b"));
        let (fresh, _) = run(t.path(), &ScanOptions::default());
        assert_eq!(totals(&r.tree), totals(&fresh.tree));
        // Children stay sorted largest first after sizes changed.
        let a = find(&r.tree, &t.path().join("a")).unwrap();
        let sizes: Vec<u64> = r
            .tree
            .children(a)
            .iter()
            .map(|c| r.tree.node(*c).size)
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    }

    #[test]
    fn grafting_clears_not_scanned_and_works_twice() {
        let t = sample();
        let cloud = t.path().join("Library/CloudStorage");
        write(&cloud.join("GoogleDrive/huge.mov"), 500_000);
        let (mut r, _) = run(t.path(), &ScanOptions::for_home(t.path()));
        graft_rescan(&mut r, &cloud);
        let id = find(&r.tree, &cloud).unwrap();
        assert!(!r.tree.node(id).has(flag::NOT_SCANNED));
        assert!(r.tree.node(id).size >= 500_000);
        graft_rescan(&mut r, &cloud);
        let (fresh, _) = run(t.path(), &ScanOptions::default());
        assert_eq!(totals(&r.tree), totals(&fresh.tree));
    }

    #[test]
    fn grafting_an_emptied_folder_leaves_no_children() {
        let t = sample();
        let (mut r, _) = run(t.path(), &ScanOptions::default());
        fs::remove_dir_all(t.path().join("d")).unwrap();
        fs::create_dir(t.path().join("d")).unwrap();
        graft_rescan(&mut r, &t.path().join("d"));
        let d = find(&r.tree, &t.path().join("d")).unwrap();
        assert!(r.tree.children(d).is_empty());
        let (fresh, _) = run(t.path(), &ScanOptions::default());
        assert_eq!(totals(&r.tree), totals(&fresh.tree));
    }

    #[test]
    fn a_file_as_root_is_a_one_node_tree() {
        let t = sample();
        let (r, _) = run(&t.path().join("five"), &ScanOptions::default());
        assert_eq!(r.tree.len(), 1);
        assert_eq!(r.tree.node(0).files, 1);
    }

    // Crossing into another volume can't be staged in a temp dir, so the check itself is
    // tested here and the walker's use of it is covered by reading `scan`.
    #[test]
    fn volume_crossing_is_a_device_change() {
        assert!(!crosses_volume(16777230, 16777230));
        assert!(crosses_volume(16777230, 16777234));
    }
}
