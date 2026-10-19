//! The settings window (spec §3.12, §3.13), opened with Cmd-, or RSdisk > Settings: the
//! themes, the visual and its colours, and the two macOS permissions rsdisk needs, with their
//! state and a way to grant each. Laid out like macOS System Settings: titled groups of rows,
//! a label on the left and its control on the right.

use crate::permissions::{self, Status};
use crate::settings::{ColourMode, Visual};
use crate::theme::{self, Theme};
use crate::view::{RsDisk, grantee};
use gpui_kit::component::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::{prelude::*, *};
use std::time::Duration;

/// How often the window re-reads the permissions, so it notices a change made in System
/// Settings.
const RECHECK: Duration = Duration::from_secs(2);

type Choice = Entity<SelectState<SearchableVec<SharedString>>>;

pub struct SettingsWindow {
    main: WeakEntity<RsDisk>,
    dark_theme: Choice,
    light_theme: Choice,
    visual: Choice,
    colour: Choice,
    full_disk: Status,
    finder: Status,
    /// macOS is showing its Finder prompt.
    asking: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

const VISUALS: [(&str, Visual); 2] = [("Treemap", Visual::Treemap), ("Sunburst", Visual::Sunburst)];
const COLOURS: [(&str, ColourMode); 2] =
    [("File kind", ColourMode::Kind), ("Age", ColourMode::Age)];

/// Opens the settings window, or brings it forward when it is already open.
pub fn open(main: WeakEntity<RsDisk>, cx: &mut App) {
    let Some(rsdisk) = main.upgrade() else { return };
    if let Some(handle) = rsdisk.read(cx).settings_window
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    // A screenshot launch (see main.rs) must not take the keyboard.
    let focus = std::env::var_os("RSDISK_NO_FOCUS").is_none();
    let bounds = Bounds::centered(None, size(px(600.), px(610.)), cx);
    let opened = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                ..Default::default()
            }),
            is_resizable: false,
            is_minimizable: false,
            focus,
            kind: if focus {
                WindowKind::Normal
            } else {
                WindowKind::PopUp
            },
            ..Default::default()
        },
        cx,
        |window, cx| cx.new(|cx| SettingsWindow::new(main.clone(), window, cx)),
    );
    if let Ok((handle, _)) = opened {
        rsdisk.update(cx, |view, _| view.settings_window = Some(handle));
        if focus {
            cx.activate(true);
        }
    }
}

impl SettingsWindow {
    fn new(main: WeakEntity<RsDisk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (themes, settings) = main
            .read_with(cx, |m, _| (m.themes.clone(), m.settings.clone()))
            .unwrap_or_default();
        let names = |dark: bool| -> Vec<SharedString> {
            themes
                .iter()
                .filter(|t| t.dark == dark)
                .map(|t| SharedString::from(t.name.clone()))
                .collect()
        };
        let current = |names: &[SharedString], chosen: Option<&str>, dark: bool| {
            let name = chosen.unwrap_or(if dark {
                theme::DARK_NAME
            } else {
                theme::LIGHT_NAME
            });
            names.iter().position(|n| n == name).map(IndexPath::new)
        };
        let choice = |items: Vec<SharedString>,
                      at: Option<IndexPath>,
                      search: bool,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), at, window, cx).searchable(search)
            })
        };
        let dark_names = names(true);
        let light_names = names(false);
        let dark_at = current(&dark_names, settings.dark_theme.as_deref(), true);
        let light_at = current(&light_names, settings.light_theme.as_deref(), false);
        let dark_theme = choice(dark_names, dark_at, true, window, cx);
        let light_theme = choice(light_names, light_at, true, window, cx);
        fn labels<T>(all: &[(&'static str, T)]) -> Vec<SharedString> {
            all.iter().map(|(l, _)| SharedString::from(*l)).collect()
        }
        let visual_at = VISUALS.iter().position(|(_, v)| *v == settings.visual);
        let colour_at = COLOURS.iter().position(|(_, c)| *c == settings.colour);
        let visual = choice(
            labels(&VISUALS),
            visual_at.map(IndexPath::new),
            false,
            window,
            cx,
        );
        let colour = choice(
            labels(&COLOURS),
            colour_at.map(IndexPath::new),
            false,
            window,
            cx,
        );

        let mut subscriptions = vec![
            Self::on_pick(&dark_theme, window, cx, |s, name| {
                s.dark_theme = Some(name.to_string())
            }),
            Self::on_pick(&light_theme, window, cx, |s, name| {
                s.light_theme = Some(name.to_string())
            }),
            Self::on_pick(&visual, window, cx, |s, name| {
                if let Some((_, v)) = VISUALS.iter().find(|(l, _)| *l == name) {
                    s.visual = *v;
                }
            }),
            Self::on_pick(&colour, window, cx, |s, name| {
                if let Some((_, c)) = COLOURS.iter().find(|(l, _)| *l == name) {
                    s.colour = *c;
                }
            }),
        ];
        // `v` and `a` in the main window change these too; keep the menus in step.
        if let Some(rsdisk) = main.upgrade() {
            subscriptions.push(cx.observe_in(&rsdisk, window, |view, rsdisk, window, cx| {
                let s = rsdisk.read(cx).settings.clone();
                let visual = VISUALS.iter().position(|(_, v)| *v == s.visual);
                let colour = COLOURS.iter().position(|(_, c)| *c == s.colour);
                for (state, at) in [(&view.visual, visual), (&view.colour, colour)] {
                    if state.read(cx).selected_index(cx) != at.map(IndexPath::new) {
                        state.update(cx, |s, cx| {
                            s.set_selected_index(at.map(IndexPath::new), window, cx)
                        });
                    }
                }
            }));
        }

        cx.spawn_in(window, async move |this, cx| {
            loop {
                let checked = cx
                    .background_executor()
                    .spawn(async {
                        (
                            permissions::full_disk_access(),
                            permissions::finder_control(false),
                        )
                    })
                    .await;
                let alive = this.update(cx, |view, cx| {
                    if (view.full_disk, view.finder) != checked {
                        (view.full_disk, view.finder) = checked;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
                cx.background_executor().timer(RECHECK).await;
            }
        })
        .detach();

        let focus = cx.focus_handle();
        focus.focus(window, cx);
        SettingsWindow {
            main,
            dark_theme,
            light_theme,
            visual,
            colour,
            full_disk: Status::Unknown,
            finder: Status::Unknown,
            asking: false,
            focus,
            _subscriptions: subscriptions,
        }
    }

    /// Applies a pick from one of the menus to the settings in the main window.
    fn on_pick(
        state: &Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
        apply: impl Fn(&mut crate::settings::Settings, &str) + 'static,
    ) -> Subscription {
        cx.subscribe_in(
            state,
            window,
            move |view, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                let name = name.clone();
                let _ = view.main.update(cx, |main, cx| {
                    main.change_settings(|s| apply(s, &name), cx);
                });
            },
        )
    }

    /// Shows macOS's own prompt for controlling Finder, then shows the answer.
    fn ask_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.asking {
            return;
        }
        self.asking = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let answer = cx
                .background_executor()
                .spawn(async { permissions::finder_control(true) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.asking = false;
                view.finder = answer;
                cx.notify();
            });
        })
        .detach();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, _: &mut Context<Self>) {
        let k = &event.keystroke;
        if (k.modifiers.platform && k.key == "w") || k.key == "escape" {
            window.remove_window();
        }
    }
}

/// A titled group of rows, like a section of System Settings.
fn group(t: &Theme, title: &str, rows: Vec<AnyElement>) -> Div {
    let mut body = div()
        .flex()
        .flex_col()
        .rounded_lg()
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.faint));
    for (i, row) in rows.into_iter().enumerate() {
        body = body.child(
            div()
                .when(i > 0, |d| d.border_t_1().border_color(rgb(t.faint)))
                .child(row),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .px_1()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(t.dim))
                .child(title.to_string()),
        )
        .child(body)
}

/// One row: a label (and a line under it) on the left, the control on the right.
fn row(t: &Theme, label: &str, detail: Option<String>, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .px_3()
        .py_2()
        .min_h(px(44.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .min_w_0()
                .child(label.to_string())
                .children(detail.map(|d| div().text_size(px(12.)).text_color(rgb(t.dim)).child(d))),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .child(control),
        )
        .into_any_element()
}

/// A permission's state: a dot in its colour and a word.
fn state(t: &Theme, status: Status, granted: &str) -> Div {
    let (colour, text) = match status {
        Status::Granted => (t.success, granted),
        Status::Denied => (t.error, "Off"),
        Status::NotAsked => (t.warning, "Not asked yet"),
        Status::Unknown => (t.dim, "Unknown"),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .text_color(rgb(t.dim))
        .child(div().size(px(8.)).rounded_full().bg(rgb(colour)))
        .child(text.to_string())
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(main) = self.main.upgrade() else {
            return div();
        };
        let t = main.read(cx).theme.clone();
        let select = |state: &Choice| Select::new(state).w(px(220.)).menu_max_h(px(320.));

        let appearance = group(
            &t,
            "Appearance",
            vec![
                row(&t, "Dark theme", None, select(&self.dark_theme)),
                row(&t, "Light theme", None, select(&self.light_theme)),
            ],
        );
        let view = group(
            &t,
            "View",
            vec![
                row(
                    &t,
                    "Visual",
                    Some("Also v in the main window".into()),
                    select(&self.visual),
                ),
                row(
                    &t,
                    "Colour by",
                    Some("Also a in the main window".into()),
                    select(&self.colour),
                ),
            ],
        );

        let full_disk_detail = match self.full_disk {
            Status::Granted => {
                "Reads Mail, Messages, Safari and the other private folders in ~/Library."
                    .to_string()
            }
            _ => format!(
                "Without it, private folders in ~/Library count as unreadable. Turn on {} in \
                 the list (add it with + if it's missing), then quit and start rsdisk again.",
                grantee()
            ),
        };
        let full_disk = div()
            .flex()
            .items_center()
            .gap_3()
            .child(state(&t, self.full_disk, "On"))
            .when(self.full_disk != Status::Granted, |d| {
                d.child(
                    Button::new("open-full-disk")
                        .label("Open Settings\u{2026}")
                        .on_click(|_, _, _| permissions::open_full_disk_access()),
                )
            });
        let finder_detail = match self.finder {
            Status::Denied => format!(
                "Moving to the Trash fails until {} may control Finder. Turn it on under \
                 Automation.",
                grantee()
            ),
            _ => "Moves items to the Trash through Finder, so Put Back works.".to_string(),
        };
        let finder_button = match self.finder {
            Status::NotAsked => Some(
                Button::new("allow-finder")
                    .primary()
                    .label(if self.asking {
                        "Asking\u{2026}"
                    } else {
                        "Allow\u{2026}"
                    })
                    .on_click(cx.listener(|view, _, window, cx| view.ask_finder(window, cx))),
            ),
            Status::Denied | Status::Unknown => Some(
                Button::new("open-automation")
                    .label("Open Settings\u{2026}")
                    .on_click(|_, _, _| permissions::open_automation()),
            ),
            Status::Granted => None,
        };
        let finder = div()
            .flex()
            .items_center()
            .gap_3()
            .child(state(&t, self.finder, "Allowed"))
            .children(finder_button);
        let who = if grantee() == "RSdisk" {
            "macOS gives these to RSdisk itself.".to_string()
        } else {
            "rsdisk runs from your terminal, so macOS checks the terminal's permissions.".into()
        };
        let permissions = group(
            &t,
            "Permissions",
            vec![
                row(&t, "Full Disk Access", Some(full_disk_detail), full_disk),
                row(&t, "Control Finder", Some(finder_detail), finder),
            ],
        );

        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .size_full()
            .bg(rgb(t.bg))
            .text_color(rgb(t.text))
            .text_size(px(13.))
            .p_5()
            .flex()
            .flex_col()
            .gap_5()
            .child(appearance)
            .child(view)
            .child(permissions)
            .child(
                div()
                    .px_1()
                    .text_size(px(12.))
                    .text_color(rgb(t.dim))
                    .child(who),
            )
    }
}
