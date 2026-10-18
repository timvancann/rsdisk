//! `r` rescans the current folder, `c` scans a skipped cloud folder (spec §3.5); the result
//! is grafted into the tree without restarting.

use crate::view::{RsDisk, Stage};
use rsdisk_core::scan::{Progress, ScanOptions, ScanResult, locate_unreadable, scan};
use rsdisk_core::tree::{NodeId, flag};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

pub struct Rescan {
    pub folder: NodeId,
    pub path: PathBuf,
    /// Live counters for the header, and Esc's cancel switch.
    pub progress: Arc<Progress>,
    done: Receiver<ScanResult>,
}

impl RsDisk {
    /// `r`: the folder the list shows.
    pub fn rescan_folder(&mut self) {
        if let Stage::Browsing(b) = &self.stage {
            let opts = ScanOptions::for_home(&self.home);
            self.start_rescan(b.folder(), opts);
        }
    }

    /// `c`: the selected folder, when it was skipped as a cloud folder.
    pub fn scan_cloud(&mut self) {
        if let Stage::Browsing(b) = &self.stage
            && let Some(id) = b.selected()
        {
            if b.tree().node(id).has(flag::NOT_SCANNED) {
                self.start_rescan(id, ScanOptions::default());
            } else {
                self.note = Some("c scans a folder marked \"not scanned\"".into());
            }
        }
    }

    fn start_rescan(&mut self, folder: NodeId, opts: ScanOptions) {
        let Stage::Browsing(b) = &self.stage else {
            return;
        };
        if let Some(r) = &self.rescan {
            self.note = Some(format!("Already scanning {}", self.shown(&r.path)));
            return;
        }
        let path = b.tree().path(folder);
        let (tx, rx) = channel();
        let progress = Arc::new(Progress::default());
        let (target, p) = (path.clone(), progress.clone());
        std::thread::spawn(move || {
            let mut result = scan(&target, &opts, &p);
            locate_unreadable(&mut result);
            let _ = tx.send(result);
        });
        self.rescan = Some(Rescan {
            folder,
            path,
            progress,
            done: rx,
        });
    }

    /// Esc while rescanning: stop it and keep the tree as it was.
    pub fn cancel_rescan(&mut self) -> bool {
        match &self.rescan {
            Some(r) => {
                r.progress.cancel();
                true
            }
            None => false,
        }
    }

    /// Grafts a finished rescan in, carrying marks inside it over by path. Returns whether
    /// anything changed (a running rescan counts, so its counters keep moving).
    pub fn poll_rescan(&mut self) -> bool {
        let Some(r) = &self.rescan else { return false };
        let Ok(result) = r.done.try_recv() else {
            return true;
        };
        let (folder, path) = (r.folder, r.path.clone());
        self.rescan = None;
        if result.cancelled {
            self.note = Some(format!(
                "Stopped scanning {}; nothing changed",
                self.shown(&path)
            ));
            return true;
        }
        let Stage::Browsing(b) = &mut self.stage else {
            return false;
        };
        let whole = b.result_mut();
        let carried: Vec<PathBuf> = self
            .marks
            .ids(&whole.tree)
            .into_iter()
            .filter(|&id| crate::marks::is_below(&whole.tree, id, folder))
            .map(|id| whole.tree.path(id))
            .collect();
        whole.errors.retain(|e| !e.path.starts_with(&path));
        whole.errors.extend(result.errors);
        whole.errors.sort_by(|a, b| a.path.cmp(&b.path));
        whole.tree.graft(folder, result.tree);
        self.marks.prune(&whole.tree);
        b.refresh();
        self.hover = None;
        self.reguard();
        if let Stage::Browsing(b) = &self.stage {
            for p in carried {
                if let Some(id) = b.tree().find(&p) {
                    self.marks.toggle(b.tree(), id);
                }
            }
        }
        true
    }

    /// A path relative to the scan root, or the whole path for the root itself.
    pub fn shown(&self, path: &std::path::Path) -> String {
        path.strip_prefix(&self.root)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string())
    }
}
