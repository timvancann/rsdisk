//! The window's one view: scanning first, then browsing (spec §3.2, §3.8). It owns the state
//! and turns input into `Browser` calls; drawing lives in `scanning.rs` and `list_view.rs`.

use crate::browser::Browser;
use crate::cleanup::Dialog;
use crate::find::Find;
use crate::keys::{Action, Keymap};
use crate::marks::Marks;
use crate::settings::{ColourMode, Settings, Visual};
use crate::theme::{self, Theme};
use crate::visual::{Hover, VisualCache};
use gpui_kit::*;
use rsdisk_core::scan::{
    Progress, ScanError, ScanOptions, ScanResult, apply_unreadable, probe_unreadable, scan,
    unreadable_candidates,
};
use rsdisk_core::tree::NodeId;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
pub const ROW_HEIGHT: f32 = 26.;

pub enum Stage {
    Scanning {
        progress: Arc<Progress>,
        started: Instant,
        done: Receiver<ScanResult>,
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
    themes: Vec<Theme>,
    keymap: Keymap,
    focus: FocusHandle,
    /// Unreadable folders found after the scan, while that search runs (spec §3.2).
    located: Option<Receiver<Vec<(NodeId, ScanError)>>>,
    _appearance: Subscription,
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
            view.theme = Self::theme_for(&view.themes, &view.settings, is_dark(window));
            theme::apply(&view.theme, Some(window), cx);
            cx.notify();
        });

        RsDisk {
            stage: Self::start_scan(&root, &home),
            root,
            home,
            theme: current,
            help: false,
            scroll: UniformListScrollHandle::new(),
            themes,
            settings,
            visual_cache: Rc::default(),
            hover: None,
            marks: Marks::default(),
            dialog: None,
            find: None,
            last_found: (Vec::new(), 0),
            keymap: Keymap::default(),
            focus,
            located: None,
            _appearance: appearance,
        }
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
        std::thread::spawn(move || {
            let _ = tx.send(scan(&root, &opts, &p));
        });
        Stage::Scanning {
            progress,
            started: Instant::now(),
            done: rx,
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if let Stage::Scanning { done, .. } = &self.stage {
            match done.try_recv() {
                Ok(result) => self.browse(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return,
            }
            cx.notify();
            return;
        }
        if self.poll_trash() {
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
    }

    pub fn browser(&self) -> Option<&Browser> {
        match &self.stage {
            Stage::Browsing(b) => Some(b),
            Stage::Scanning { .. } => None,
        }
    }

    pub fn locating(&self) -> bool {
        self.located.is_some()
    }

    fn visible_rows(window: &Window) -> usize {
        // The list fills the window below the header (about 90px).
        let height: f32 = (window.viewport_size().height - px(90.)).into();
        (height / ROW_HEIGHT).max(1.) as usize
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // A screenshot launch must never act on keys: they belong to whatever Tim is typing
        // in (one stray `a` switched the colour mode during a screenshot run).
        if std::env::var_os("RSDISK_NO_FOCUS").is_some() {
            return;
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
            let s = &mut self.settings;
            match action {
                Action::SwitchVisual => {
                    s.visual = match s.visual {
                        Visual::Treemap => Visual::Sunburst,
                        Visual::Sunburst => Visual::Treemap,
                    };
                    self.hover = None;
                }
                _ => {
                    s.colour = match s.colour {
                        ColourMode::Kind => ColourMode::Age,
                        ColourMode::Age => ColourMode::Kind,
                    }
                }
            }
            if let Some(path) = Settings::default_path() {
                let _ = s.save(&path);
            }
            cx.notify();
            return;
        }
        match action {
            Action::ToggleMark => self.toggle_mark(),
            Action::UnmarkAll => self.marks.clear(),
            Action::Trash => self.open_confirm(),
            Action::Search => self.open_find(),
            Action::NextMatch => self.step_found(true),
            Action::PrevMatch => self.step_found(false),
            _ => {}
        }
        let rows = Self::visible_rows(window);
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
                    | Action::PrevMatch => {}
                    Action::Help => self.help = true,
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let body = match &self.stage {
            Stage::Scanning {
                progress, started, ..
            } => self.render_scanning(progress, *started).into_any_element(),
            Stage::Browsing(b) => self.render_browser(b, window, cx).into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.text))
            .text_size(px(13.))
            .child(body)
    }
}
