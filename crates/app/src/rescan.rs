//! `r` rescans the current folder, `c` scans a skipped cloud folder (spec §3.5); the result
//! is grafted into the tree without restarting.

use crate::view::{RsDisk, Stage};
use rsdisk_core::scan::{Progress, ScanOptions, ScanResult, locate_unreadable, scan};
use rsdisk_core::tree::{NodeId, flag};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

pub struct Rescan {
    pub folder: NodeId,
    pub path: PathBuf,
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
            && b.tree().node(id).has(flag::NOT_SCANNED)
        {
            self.start_rescan(id, ScanOptions::default());
        }
    }

    fn start_rescan(&mut self, folder: NodeId, opts: ScanOptions) {
        let Stage::Browsing(b) = &self.stage else {
            return;
        };
        if self.rescan.is_some() {
            return;
        }
        let path = b.tree().path(folder);
        let (tx, rx) = channel();
        let target = path.clone();
        std::thread::spawn(move || {
            let mut result = scan(&target, &opts, &Progress::default());
            locate_unreadable(&mut result);
            let _ = tx.send(result);
        });
        self.rescan = Some(Rescan {
            folder,
            path,
            done: rx,
        });
    }

    /// Grafts a finished rescan in. Returns whether anything changed.
    pub fn poll_rescan(&mut self) -> bool {
        let Some(r) = &self.rescan else { return false };
        let Ok(result) = r.done.try_recv() else {
            return false;
        };
        let (folder, path) = (r.folder, r.path.clone());
        self.rescan = None;
        let Stage::Browsing(b) = &mut self.stage else {
            return false;
        };
        let whole = b.result_mut();
        whole.errors.retain(|e| !e.path.starts_with(&path));
        whole.errors.extend(result.errors);
        whole.errors.sort_by(|a, b| a.path.cmp(&b.path));
        whole.tree.graft(folder, result.tree);
        self.marks.prune(&whole.tree);
        b.refresh();
        self.hover = None;
        true
    }
}
