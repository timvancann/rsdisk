//! The window's one view: scanning first, then browsing (spec §3.2, §3.8). It owns the state
//! and turns input into `Browser` calls; drawing lives in `scanning.rs` and `list_view.rs`.

use crate::browser::Browser;
use crate::cleanup::Dialog;
use crate::find::Find;
use crate::keys::{Action, Keymap};
use crate::marks::Marks;
use crate::quick_look;
use crate::rescan::Rescan;
use crate::settings::{ColourMode, Settings, Visual};
use crate::theme::{self, Theme};
use crate::visual::{Hover, VisualCache};
use gpui_kit::*;
use rsdisk_core::scan::{
    Progress, ScanError, ScanOptions, ScanResult, apply_unreadable, probe_unreadable, scan,
    unreadable_candidates,
};
use rsdisk_core::tree::{NodeId, flag};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
pub const ROW_HEIGHT: f32 = 32.;

pub enum Stage {
    Scanning {
        progress: Arc<Progress>,
        started: Instant,
        done: Receiver<ScanResult>,
        /// The scan so far, drawn like the browser (spec §3.2), and the
        /// `Progress::live_version` it was built from.
        so_far: Box<Browser>,
        so_far_version: u64,
    },
    Browsing(Browser),
}

pub struct RsDisk {
    pub root: PathBuf,
    pub home: PathBuf,
    pub stage: Stage,
    pub theme: Theme,
    pub help: bool,
    pub scroll: UniformListScrollHandle,
    pub(crate) settings: Settings,
    pub(crate) visual_cache: Rc<RefCell<VisualCache>>,
    pub(crate) hover: Option<Hover>,
    pub(crate) marks: Marks,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) find: Option<Find>,
    /// The last search's results and where `n`/`N` are in them.
    pub(crate) last_found: (Vec<NodeId>, usize),
    pub(crate) rescan: Option<Rescan>,
    /// A one-line answer to the last key, such as why a mark was refused; the next key
    /// clears it.
    pub(crate) note: Option<String>,
    pub(crate) themes: Vec<Theme>,
    /// Whether the system appearance is dark, which picks the dark or light theme.
    pub(crate) dark: bool,
    /// The settings window while it is open (Cmd-, brings it forward instead of opening a
    /// second one).
    pub(crate) settings_window: Option<AnyWindowHandle>,
    keymap: Keymap,
    focus: FocusHandle,
    /// Unreadable folders found after the scan, while that search runs (spec §3.2).
    located: Option<Receiver<Vec<(NodeId, ScanError)>>>,
    _appearance: Subscription,
}

/// Who macOS asks permissions of: RSdisk itself when it runs from its app bundle, else the
/// terminal it was started from.
pub fn grantee() -> &'static str {
    let bundled =
        std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"));
    if bundled { "RSdisk" } else { "your terminal" }
}

/// The scan so far as a result the browser can draw.
fn so_far(root: &Path, progress: &Progress) -> ScanResult {
    ScanResult {
        tree: progress.tree_so_far(root),
        errors: Vec::new(),
        elapsed: Duration::ZERO,
        cancelled: false,
        unlocated: 0,
    }
}

fn is_dark(window: &Window) -> bool {
    matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    )
}

impl RsDisk {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = std::env::home_dir().unwrap_or_default();
        let settings = Settings::default_path()
            .map(|p| Settings::load(&p))
            .unwrap_or_default();
        let themes = theme::available(&theme::zed_dirs());
        let current = Self::theme_for(&themes, &settings, is_dark(window));
        theme::apply(&current, Some(window), cx);

        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let appearance = cx.observe_window_appearance(window, |view, window, cx| {
            view.dark = is_dark(window);
            view.theme = Self::theme_for(&view.themes, &view.settings, view.dark);
            theme::apply(&view.theme, Some(window), cx);
            cx.notify();
            cx.refresh_windows();
        });

        RsDisk {
            stage: Self::start_scan(&root, &home),
            root,
            home,
            theme: current,
            help: false,
            scroll: UniformListScrollHandle::new(),
            themes,
            dark: is_dark(window),
            settings_window: None,
            settings,
            visual_cache: Rc::default(),
            hover: None,
            marks: Marks::default(),
            dialog: None,
            find: None,
            last_found: (Vec::new(), 0),
            rescan: None,
            note: None,
            keymap: Keymap::default(),
            focus,
            located: None,
            _appearance: appearance,
        }
    }

    /// A change from the settings window (or `v` / `a`): saves the settings and redraws every
    /// window with them.
    pub(crate) fn change_settings(
        &mut self,
        change: impl FnOnce(&mut Settings),
        cx: &mut Context<Self>,
    ) {
        change(&mut self.settings);
        if let Some(path) = Settings::default_path() {
            let _ = self.settings.save(&path);
        }
        self.theme = Self::theme_for(&self.themes, &self.settings, self.dark);
        theme::apply(&self.theme, None, cx);
        self.hover = None;
        cx.notify();
        cx.refresh_windows();
    }

    fn theme_for(themes: &[Theme], settings: &Settings, dark: bool) -> Theme {
        let name = if dark {
            &settings.dark_theme
        } else {
            &settings.light_theme
        };
        theme::pick(themes, name.as_deref(), dark)
    }

    fn start_scan(root: &Path, home: &Path) -> Stage {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let (root, opts, p) = (
            root.to_path_buf(),
            ScanOptions::for_home(home),
            progress.clone(),
        );
        let so_far = Box::new(Browser::so_far(so_far(&root, &progress)));
        std::thread::spawn(move || {
            let _ = tx.send(scan(&root, &opts, &p));
        });
        Stage::Scanning {
            progress,
            started: Instant::now(),
            done: rx,
            so_far,
            so_far_version: 0,
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if let Stage::Scanning {
            done,
            progress,
            so_far: shown,
            so_far_version,
            ..
        } = &mut self.stage
        {
            match done.try_recv() {
                Ok(result) => self.browse(result),
                Err(TryRecvError::Empty) if progress.live_version() != *so_far_version => {
                    *so_far_version = progress.live_version();
                    **shown = Browser::so_far(so_far(&self.root, progress));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
            cx.notify();
            return;
        }
        if self.poll_trash() | self.poll_rescan() {
            cx.notify();
        }
        if let Some(rx) = &self.located
            && let Ok(found) = rx.try_recv()
        {
            self.located = None;
            if let Stage::Browsing(b) = &mut self.stage {
                apply_unreadable(b.result_mut(), found);
                b.refresh();
            }
            self.reguard();
            cx.notify();
        }
    }

    fn browse(&mut self, result: ScanResult) {
        let candidates = unreadable_candidates(&result);
        if !candidates.is_empty() {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(probe_unreadable(candidates));
            });
            self.located = Some(rx);
        }
        self.stage = Stage::Browsing(Browser::new(result));
        // A hover over the scan-so-far points at its nodes, not the real tree's.
        self.hover = None;
        self.reguard();
    }

    /// Refreshes what can't be marked (`Marks::guard`) after the tree changed.
    pub(crate) fn reguard(&mut self) {
        if let Stage::Browsing(b) = &self.stage {
            let cloud = ScanOptions::for_home(&self.home).skip;
            self.marks.guard(b.tree(), &cloud);
        }
    }

    pub fn browser(&self) -> Option<&Browser> {
        match &self.stage {
            Stage::Browsing(b) => Some(b),
            Stage::Scanning { .. } => None,
        }
    }

    /// What the list and visual draw: the browser, or the scan so far while it runs. Only
    /// drawing uses this; keys and clicks act on `browser()`.
    pub fn shown_browser(&self) -> &Browser {
        match &self.stage {
            Stage::Browsing(b) => b,
            Stage::Scanning { so_far, .. } => so_far,
        }
    }

    pub fn locating(&self) -> bool {
        self.located.is_some()
    }

    /// How many rows the list shows, from its height at the last paint (the header's height
    /// varies with notes and warnings).
    fn visible_rows(&self, window: &Window) -> usize {
        let painted = self.scroll.0.borrow().base_handle.bounds().size.height;
        let height: f32 = if painted > px(0.) {
            painted.into()
        } else {
            (window.viewport_size().height - px(90.)).into()
        };
        (height / ROW_HEIGHT).max(1.) as usize
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A screenshot launch must never act on keys: they belong to whatever Tim is typing
        // in (one stray `a` switched the colour mode during a screenshot run).
        if std::env::var_os("RSDISK_NO_FOCUS").is_some() {
            return;
        }
        if self.note.take().is_some() {
            cx.notify();
        }
        let k = &event.keystroke;
        if k.modifiers.platform {
            if k.key == "q" {
                cx.quit();
            }
            return;
        }
        let named = k.key.chars().count() > 1;
        let key = match (&k.key_char, named || k.modifiers.control) {
            (Some(c), false) => c.as_str(),
            _ => k.key.as_str(),
        };
        if self.dialog_key(key) || self.find_key(&k.key, k.key_char.as_deref(), k.modifiers.control)
        {
            cx.notify();
            return;
        }
        let Some(action) = self.keymap.feed(key, k.modifiers.control) else {
            return;
        };
        self.act(action, window, cx);
    }

    pub fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if action == Action::Quit {
            cx.quit();
            return;
        }
        if self.help {
            if matches!(action, Action::Help | Action::Cancel) {
                self.help = false;
                cx.notify();
            }
            return;
        }
        if matches!(action, Action::SwitchVisual | Action::SwitchColour) {
            self.change_settings(
                |s| match action {
                    Action::SwitchVisual => {
                        s.visual = match s.visual {
                            Visual::Treemap => Visual::Sunburst,
                            Visual::Sunburst => Visual::Treemap,
                        }
                    }
                    _ => {
                        s.colour = match s.colour {
                            ColourMode::Kind => ColourMode::Age,
                            ColourMode::Age => ColourMode::Kind,
                        }
                    }
                },
                cx,
            );
            return;
        }
        match action {
            Action::ToggleMark => self.toggle_mark(),
            Action::UnmarkAll => self.marks.clear(),
            Action::Trash => self.open_confirm(),
            Action::Search => self.open_find(),
            Action::NextMatch => self.step_found(true),
            Action::PrevMatch => self.step_found(false),
            Action::Rescan => self.rescan_folder(),
            Action::ScanCloud => self.scan_cloud(),
            Action::ListUnreadable => self.open_unreadable(),
            // Esc closes or stops the most recent thing: a rescan, the preview, the search
            // position.
            Action::Cancel if self.browser().is_some() => {
                if !self.cancel_rescan() && !quick_look::close() {
                    self.last_found = (Vec::new(), 0);
                }
            }
            Action::Open => self.explain_unopenable(),
            _ => {}
        }
        let rows = self.visible_rows(window);
        match &mut self.stage {
            Stage::Scanning { progress, .. } => {
                if action == Action::Cancel {
                    progress.cancel();
                }
            }
            Stage::Browsing(b) => {
                match action {
                    Action::Down => b.down(),
                    Action::Up => b.up(),
                    Action::Top => b.top(),
                    Action::Bottom => b.bottom(),
                    Action::HalfPageDown => b.half_page_down(rows),
                    Action::HalfPageUp => b.half_page_up(rows),
                    Action::Open => b.open(),
                    Action::Back => b.back(),
                    Action::CycleSort => b.cycle_sort(),
                    Action::ToggleMark
                    | Action::UnmarkAll
                    | Action::Trash
                    | Action::Search
                    | Action::NextMatch
                    | Action::PrevMatch
                    | Action::Rescan
                    | Action::ScanCloud
                    | Action::ListUnreadable
                    | Action::Cancel => {}
                    Action::Help => self.help = true,
                    Action::Preview => {
                        if let Some(s) = b.selected() {
                            quick_look::toggle(&b.tree().path(s));
                        }
                    }
                    Action::Reveal => {
                        if let Some(s) = b.selected() {
                            let path = b.tree().path(s);
                            let _ = std::process::Command::new("open")
                                .arg("-R")
                                .arg(path)
                                .spawn();
                        }
                    }
                    _ => return,
                }
                if let Some(i) = b.selected_index() {
                    self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
                }
            }
        }
        cx.notify();
    }

    /// The dimmed layer under help, search and dialogs. It takes the mouse, so rows and cells
    /// behind it can't be clicked.
    pub(crate) fn scrim(&self) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .justify_center()
            .items_center()
            .bg(rgba((self.theme.bg << 8) | 0x99))
            .occlude()
    }

    /// `l` on a folder that can't be entered says why, instead of doing nothing.
    fn explain_unopenable(&mut self) {
        let Some(b) = self.browser() else { return };
        let Some(id) = b.selected() else { return };
        let n = b.tree().node(id);
        if !n.is_dir() || b.can_open(id) {
            return;
        }
        let why = if n.has(flag::UNREADABLE) {
            "is unreadable (see Full Disk Access)"
        } else if n.has(flag::NOT_SCANNED) {
            "wasn't scanned; c scans it"
        } else {
            "is on another volume, which rsdisk doesn't cross"
        };
        self.note = Some(format!("{} {why}", n.name));
    }

    /// Runs `f` on the browser (if browsing), keeps the list scrolled to the selection, and
    /// redraws.
    pub fn with_browser(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Browser)) {
        if let Stage::Browsing(b) = &mut self.stage {
            let folder = b.folder();
            f(b);
            if b.folder() != folder {
                self.hover = None;
            }
            if let Some(i) = b.selected_index() {
                self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
            }
            cx.notify();
        }
    }

    /// Mouse: select a row, or open it on a double click.
    pub fn click_row(&mut self, id: NodeId, open: bool, cx: &mut Context<Self>) {
        if let Stage::Browsing(b) = &mut self.stage {
            b.jump_to(id);
            if open {
                b.open();
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    pub fn click_crumb(&mut self, folder: NodeId, cx: &mut Context<Self>) {
        if let Stage::Browsing(b) = &mut self.stage {
            b.go_to_folder(folder);
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            cx.notify();
        }
    }
}

impl Render for RsDisk {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let body = match &self.stage {
            Stage::Scanning {
                progress,
                started,
                so_far,
                ..
            } => self
                .render_browser(so_far, Some((progress, *started)), cx)
                .into_any_element(),
            Stage::Browsing(b) => self.render_browser(b, None, cx).into_any_element(),
        };
        // Every selection change ends in a render, so this is where the preview follows it.
        if let Stage::Browsing(b) = &self.stage
            && let Some(s) = b.selected()
            && quick_look::is_open()
        {
            quick_look::follow(&b.tree().path(s));
        }
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.text))
            .font_family(theme::UI)
            .text_size(px(13.))
            .child(body)
    }
}
