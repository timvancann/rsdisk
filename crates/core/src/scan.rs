//! The parallel walk (spec §3.2). Each directory is a rayon task that lists and stats its
//! entries, then descends into its subdirectories in parallel. The nested result is flattened
//! into a `Tree` afterwards, on one thread.

use crate::kind::{Kind, kind_of};
use crate::tree::{Node, NodeId, Tree, TreeBuilder, flag};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

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
    cancel: AtomicBool,
}

impl Progress {
    /// The directory most recently entered.
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
}

/// One listed entry, with its subtree when it is a directory that was entered.
struct Entry {
    node: Node,
    dev: u64,
    children: Vec<Entry>,
}

struct Ctx<'a> {
    root_dev: u64,
    opts: &'a ScanOptions,
    progress: &'a Progress,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    errors: Mutex<Vec<ScanError>>,
}

/// Whether an entry on `dev` lies on another volume than the scan root.
pub fn crosses_volume(root_dev: u64, dev: u64) -> bool {
    root_dev != dev
}

/// Allocated bytes, as `du` counts them.
fn allocated(meta: &Metadata) -> u64 {
    meta.blocks() * 512
}

pub fn scan(root: &Path, opts: &ScanOptions, progress: &Progress) -> ScanResult {
    let start = Instant::now();
    let meta = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) => {
            let node = Node::new(
                root.to_string_lossy(),
                0,
                0,
                Kind::Other,
                flag::DIR | flag::UNREADABLE,
            );
            return ScanResult {
                tree: TreeBuilder::new(node).finish(),
                errors: vec![ScanError {
                    path: root.to_path_buf(),
                    message: e.to_string(),
                }],
                elapsed: start.elapsed(),
                cancelled: false,
            };
        }
    };
    let ctx = Ctx {
        root_dev: meta.dev(),
        opts,
        progress,
        seen_inodes: Mutex::new(HashSet::new()),
        errors: Mutex::new(Vec::new()),
    };
    let mut root_entry = ctx.entry(root.to_string_lossy().into(), &meta);
    if meta.is_dir() {
        progress.dirs.fetch_add(1, Relaxed);
        root_entry.children = ctx.list(root, &mut root_entry.node);
    } else {
        progress.files.fetch_add(1, Relaxed);
    }
    progress.bytes.fetch_add(root_entry.node.size, Relaxed);

    let tree = flatten(root_entry);
    let mut errors = ctx.errors.into_inner().unwrap_or_default();
    errors.sort_by(|a, b| a.path.cmp(&b.path));
    ScanResult {
        tree,
        errors,
        elapsed: start.elapsed(),
        cancelled: progress.is_cancelled(),
    }
}

impl Ctx<'_> {
    fn error(&self, path: &Path, e: impl ToString) {
        if let Ok(mut errors) = self.errors.lock() {
            errors.push(ScanError {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
        }
    }

    /// A node for one entry, without its children. Hardlinked files count once.
    fn entry(&self, name: Box<str>, meta: &Metadata) -> Entry {
        let ft = meta.file_type();
        let mut flags = 0;
        let mut size = allocated(meta);
        let kind = if ft.is_dir() {
            flags |= flag::DIR;
            if kind_of(&name) == Kind::App {
                Kind::App
            } else {
                Kind::Other
            }
        } else {
            if ft.is_symlink() {
                flags |= flag::SYMLINK;
            }
            if meta.nlink() > 1
                && let Ok(mut seen) = self.seen_inodes.lock()
                && !seen.insert((meta.dev(), meta.ino()))
            {
                size = 0;
            }
            kind_of(&name)
        };
        Entry {
            node: Node::new(name, size, meta.mtime(), kind, flags),
            dev: meta.dev(),
            children: Vec::new(),
        }
    }

    /// Lists `dir` and recurses into its subdirectories in parallel. Marks `node` unreadable
    /// when the listing fails.
    fn list(&self, dir: &Path, node: &mut Node) -> Vec<Entry> {
        if self.progress.is_cancelled() {
            return Vec::new();
        }
        if let Ok(mut latest) = self.progress.latest.try_lock() {
            latest.as_mut_os_string().clear();
            latest.push(dir);
        }
        let read = match fs::read_dir(dir) {
            Ok(r) => r,
            Err(e) => {
                node.flags |= flag::UNREADABLE;
                self.error(dir, e);
                return Vec::new();
            }
        };
        let mut entries: Vec<Entry> = Vec::new();
        let (mut files, mut dirs, mut bytes) = (0, 0, 0);
        for item in read {
            let item = match item {
                Ok(i) => i,
                Err(e) => {
                    self.error(dir, e);
                    continue;
                }
            };
            // `DirEntry::metadata` does not follow symlinks.
            let meta = match item.metadata() {
                Ok(m) => m,
                Err(e) => {
                    self.error(&item.path(), e);
                    continue;
                }
            };
            let entry = self.entry(item.file_name().to_string_lossy().into(), &meta);
            if entry.node.is_dir() {
                dirs += 1;
            } else {
                files += 1;
            }
            bytes += entry.node.size;
            entries.push(entry);
        }
        self.progress.files.fetch_add(files, Relaxed);
        self.progress.dirs.fetch_add(dirs, Relaxed);
        self.progress.bytes.fetch_add(bytes, Relaxed);

        entries
            .par_iter_mut()
            .filter(|e| e.node.is_dir())
            .for_each(|e| {
                let path = dir.join(&*e.node.name);
                if crosses_volume(self.root_dev, e.dev) {
                    e.node.flags |= flag::OTHER_VOLUME;
                    e.node.size = 0;
                } else if self.opts.skip.contains(&path) {
                    e.node.flags |= flag::NOT_SCANNED;
                } else {
                    e.children = self.list(&path, &mut e.node);
                }
            });
        entries
    }
}

fn flatten(root: Entry) -> Tree {
    let mut builder = TreeBuilder::new(root.node);
    let mut stack: Vec<(NodeId, Vec<Entry>)> = vec![(0, root.children)];
    while let Some((parent, children)) = stack.pop() {
        for child in children {
            let id = builder.add(parent, child.node);
            if !child.children.is_empty() {
                stack.push((id, child.children));
            }
        }
    }
    builder.finish()
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
        let (r, _) = run(t.path(), &ScanOptions::default());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let id = find(&r.tree, &locked).unwrap();
        assert!(r.tree.node(id).has(flag::UNREADABLE));
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].path, locked);
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
    fn a_file_as_root_is_a_one_node_tree() {
        let t = sample();
        let (r, _) = run(&t.path().join("five"), &ScanOptions::default());
        assert_eq!(r.tree.len(), 1);
        assert_eq!(r.tree.node(0).files, 1);
    }

    // Crossing into another volume can't be staged in a temp dir, so the check itself is
    // tested here and the walker's use of it is covered by reading `list`.
    #[test]
    fn volume_crossing_is_a_device_change() {
        assert!(!crosses_volume(16777230, 16777230));
        assert!(crosses_volume(16777230, 16777234));
    }
}
