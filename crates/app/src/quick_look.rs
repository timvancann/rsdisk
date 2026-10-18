//! `p` previews the selected item in the system Quick Look panel, the one Finder shows on
//! space, and the panel follows the selection while it is open. `qlmanage -p` was tried
//! first and crashed on videos.
//!
//! The panel takes its items from whichever object in the key window's responder chain
//! accepts control of it. gpui's chain is not ours to extend, so the three controller methods
//! are copied onto gpui's application delegate, which sits at the end of every chain.

use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, ffi, msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSInteger, NSString, NSURL};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

thread_local! {
    static SOURCE: RefCell<Option<Retained<Source>>> = const { RefCell::new(None) };
    static SHOWN: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static LOADING: Cell<bool> = const { Cell::new(false) };
    static CLOSE_AFTER_LOAD: Cell<bool> = const { Cell::new(false) };
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Source` has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RsDiskQuickLookSource"]
    struct Source;

    unsafe impl NSObjectProtocol for Source {}

    unsafe impl QLPreviewPanelDataSource for Source {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn count(&self, _panel: Option<&QLPreviewPanel>) -> NSInteger {
            SHOWN.with_borrow(|p| p.is_some() as NSInteger)
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn item(
            &self,
            _panel: Option<&QLPreviewPanel>,
            _index: NSInteger,
        ) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
            SHOWN.with_borrow(|p| p.clone()).map(|path| {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                ProtocolObject::from_retained(url)
            })
        }
    }

    // Copied onto gpui's app delegate by `install`; `self` is that delegate, so these
    // never touch it.
    impl Source {
        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts(&self, _panel: Option<&QLPreviewPanel>) -> bool {
            true
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin(&self, panel: Option<&QLPreviewPanel>) {
            if let Some(panel) = panel {
                SOURCE.with_borrow(|s| unsafe {
                    panel.setDataSource(s.as_deref().map(ProtocolObject::from_ref))
                });
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end(&self, panel: Option<&QLPreviewPanel>) {
            if let Some(panel) = panel {
                unsafe { panel.setDataSource(None) };
            }
        }
    }
);

fn install(mtm: MainThreadMarker) {
    if INSTALLED.get() {
        return;
    }
    // Without a delegate yet, try again on the next `p`.
    let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
        return;
    };
    INSTALLED.set(true);
    let source: Retained<Source> = unsafe { msg_send![Source::alloc(mtm), init] };
    SOURCE.set(Some(source));
    let target = unsafe { ffi::object_getClass(&*delegate as *const _ as *const AnyObject) };
    let from: &AnyClass = Source::class();
    let selectors: [Sel; 3] = [
        sel!(acceptsPreviewPanelControl:),
        sel!(beginPreviewPanelControl:),
        sel!(endPreviewPanelControl:),
    ];
    for sel in selectors {
        unsafe {
            let method = ffi::class_getInstanceMethod(from, sel);
            if let Some(imp) = ffi::method_getImplementation(method) {
                ffi::class_addMethod(
                    target as *mut AnyClass,
                    sel,
                    imp,
                    ffi::method_getTypeEncoding(method),
                );
            }
        }
    }
}

fn open_panel(mtm: MainThreadMarker) -> Option<Retained<QLPreviewPanel>> {
    unsafe { QLPreviewPanel::sharedPreviewPanelExists(mtm) }
        .then(|| unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) })
        .flatten()
        .filter(|p| p.isVisible())
}

/// `p`: shows `path`, or closes the panel when it is already open.
///
/// The panel runs a nested event loop while it loads a preview. Inside a gpui handler that
/// loop lets gpui's own tasks run while the app is borrowed, which panics, so all panel work
/// is queued to run on the main queue after the handler returns.
pub fn toggle(path: &Path) {
    let path = path.to_path_buf();
    DispatchQueue::main().exec_async(move || toggle_now(path));
}

fn toggle_now(path: PathBuf) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // A `p` that arrives while the panel is still loading (from inside its nested loop)
    // means close it once it is up.
    if LOADING.get() {
        CLOSE_AFTER_LOAD.set(true);
        return;
    }
    if let Some(panel) = open_panel(mtm) {
        panel.orderOut(None);
        return;
    }
    install(mtm);
    SHOWN.set(Some(path));
    if let Some(panel) = unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) } {
        LOADING.set(true);
        unsafe {
            panel.updateController();
            panel.reloadData();
        }
        // orderFront, not makeKey: the keyboard stays with the list, so j/k keep moving.
        panel.orderFront(None);
        LOADING.set(false);
        if CLOSE_AFTER_LOAD.replace(false) {
            panel.orderOut(None);
        }
    }
}

/// While the panel is open, shows `path` instead of what it shows now.
pub fn follow(path: &Path) {
    if SHOWN.with_borrow(|p| p.as_deref() == Some(path)) {
        return;
    }
    SHOWN.set(Some(path.to_path_buf()));
    DispatchQueue::main().exec_async(|| {
        if let Some(panel) = MainThreadMarker::new().and_then(open_panel) {
            unsafe { panel.reloadData() };
        }
    });
}

/// Whether the panel is showing, so callers can skip work when it isn't.
pub fn is_open() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| open_panel(mtm).is_some())
}
