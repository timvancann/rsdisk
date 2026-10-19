//! The two macOS permissions rsdisk needs (spec §3.13): Full Disk Access, to read the private
//! parts of `~/Library`, and control of Finder, to move items to the Trash so Put Back works.
//! Both belong to whoever macOS holds responsible: RSdisk.app, or the terminal it runs from.

use std::ffi::c_void;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Granted,
    Denied,
    /// macOS hasn't asked yet (Finder control only; Full Disk Access is never asked).
    NotAsked,
    /// Couldn't tell, for example because Finder isn't running.
    Unknown,
}

/// The system's own permission database is only readable with Full Disk Access, which makes
/// opening it the usual test for it.
pub fn full_disk_access() -> Status {
    let db = Path::new("/Library/Application Support/com.apple.TCC/TCC.db");
    match std::fs::File::open(db) {
        Ok(_) => Status::Granted,
        Err(_) => Status::Denied,
    }
}

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, result: *mut AEDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AEDesc,
        event_class: u32,
        event_id: u32,
        ask_user_if_needed: u8,
    ) -> i32;
}

const TYPE_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
const WILDCARD: u32 = u32::from_be_bytes(*b"****");

/// Whether rsdisk may send Finder Apple Events. With `ask`, macOS shows its prompt when it
/// hasn't asked before, and this blocks until it's answered: call it off the main thread.
pub fn finder_control(ask: bool) -> Status {
    let finder = b"com.apple.finder";
    let mut desc = AEDesc {
        descriptor_type: 0,
        data_handle: std::ptr::null_mut(),
    };
    // SAFETY: `desc` is a valid out-pointer and is disposed of before returning; the
    // bundle id bytes outlive the call that copies them.
    let status = unsafe {
        if AECreateDesc(
            TYPE_BUNDLE_ID,
            finder.as_ptr().cast(),
            finder.len() as isize,
            &mut desc,
        ) != 0
        {
            return Status::Unknown;
        }
        let status = AEDeterminePermissionToAutomateTarget(&desc, WILDCARD, WILDCARD, ask as u8);
        AEDisposeDesc(&mut desc);
        status
    };
    match status {
        0 => Status::Granted,
        -1743 => Status::Denied,
        -1744 => Status::NotAsked,
        _ => Status::Unknown,
    }
}

/// Opens System Settings at Privacy & Security > Full Disk Access.
pub fn open_full_disk_access() {
    open_pane("Privacy_AllFiles");
}

/// Opens System Settings at Privacy & Security > Automation.
pub fn open_automation() {
    open_pane("Privacy_Automation");
}

fn open_pane(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = std::process::Command::new("open").arg(url).spawn();
}
