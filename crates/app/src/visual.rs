//! The right-hand pane: the current folder as a treemap or sunburst (spec §2.10, §3.8).
//!
//! Layouts depend on the pane's size, which is only known while painting. So the canvas
//! records its bounds, the view lays out in `render` with the last bounds it saw (asking for
//! one more frame when they change), and the canvas paints what `render` prepared.

use crate::browser::Browser;
use crate::colour::{self, AGE_LEGEND};
use crate::layout::Rect;
use crate::layout::sunburst::{self, Arc};
use crate::layout::treemap::{self, Cell, HEADER};
use crate::scanning::count;
use crate::settings::{ColourMode, Visual};
use crate::theme::{MONO, Theme};
use crate::view::RsDisk;
use gpui_kit::{prelude::*, *};
use rsdisk_core::format;
use rsdisk_core::kind::Kind;
use rsdisk_core::tree::{NodeId, Tree, flag};
use std::cell::RefCell;
use std::collections::HashSet;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// What a layout was computed for; any change recomputes it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Key {
    folder: NodeId,
    visual: Visual,
    w: f32,
    h: f32,
    /// Changes when the tree does (removals, plan 05).
    total: u64,
    nodes: usize,
}

#[derive(Default)]
pub struct VisualCache {
    /// The pane's bounds at the last paint.
    bounds: Option<Bounds<Pixels>>,
    key: Option<Key>,
    cells: Vec<Cell>,
    /// Folders the treemap subdivided (drawn as frames, not filled).
    framed: HashSet<NodeId>,
    arcs: Vec<Arc>,
    ring: f32,
}

/// What the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Node(NodeId),
    /// The merged remainder of a folder.
    Rest(NodeId),
    /// The sunburst's centre: the folder itself.
    Centre,
}

#[derive(Debug, Clone, Copy)]
pub struct Hover {
    pub target: Target,
    pub at: Point<Pixels>,
}

/// Everything the canvas needs to paint one frame.
struct Frame {
    /// The view, to re-render when the pane's bounds change.
    view: EntityId,
    visual: Visual,
    cache: Rc<RefCell<VisualCache>>,
    fills: Vec<u32>,
    /// Treemap cells drawn as a frame around their children rather than filled.
    framed: Vec<bool>,
    labels: Vec<Option<SharedString>>,
    centre_label: (SharedString, SharedString),
    selected: Option<usize>,
    hovered: Option<usize>,
    /// Cells or arcs of marked nodes, outlined dashed.
    marked: Vec<usize>,
    theme: Theme,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn hsla(c: u32) -> Hsla {
    rgb(c).into()
}

/// Roughly how wide one character is at 11 px type.
const CHAR_W: f32 = 6.4;

/// `text` cut to roughly fit `width` px at 11 px type; `None` below `min` characters.
fn fit(text: &str, width: f32, min: usize) -> Option<String> {
    let max = (width / CHAR_W) as usize;
    let len = text.chars().count();
    if len <= max {
        return Some(text.to_string());
    }
    if max < min.max(2) {
        return None;
    }
    let cut: String = text.chars().take(max - 1).collect();
    Some(format!("{cut}\u{2026}"))
}

/// `name size`, cutting the name (to no fewer than `min` characters) rather than the size;
/// just the size when the name has no room, nothing when even that doesn't fit.
fn label(name: &str, size: u64, width: f32, min: usize) -> Option<SharedString> {
    let size = format::bytes(size);
    let size_w = (size.chars().count() + 1) as f32 * CHAR_W;
    match fit(name, width - size_w, min) {
        Some(name) => Some(format!("{name} {size}").into()),
        None => fit(&size, width, usize::MAX).map(Into::into),
    }
}

/// The size of `parent`'s merged small items: what its drawn children leave over.
fn rest_size<T>(
    tree: &Tree,
    items: &[T],
    parent: NodeId,
    node: impl Fn(&T) -> Option<NodeId>,
    parent_of: impl Fn(&T) -> NodeId,
) -> u64 {
    let shown: u64 = items
        .iter()
        .filter(|i| parent_of(i) == parent)
        .filter_map(&node)
        .map(|id| tree.node(id).size)
        .sum();
    tree.node(parent).size.saturating_sub(shown)
}

/// The child of `folder` that `id` lies under (or `id` itself).
fn top_level(tree: &Tree, folder: NodeId, mut id: NodeId) -> Option<NodeId> {
    loop {
        let parent = tree.node(id).parent()?;
        if parent == folder {
            return Some(id);
        }
        id = parent;
    }
}

impl RsDisk {
    /// Recomputes the layout when the folder, size, visual or tree changed.
    fn prepare_layout(&self, b: &Browser) {
        let mut cache = self.visual_cache.borrow_mut();
        let Some(bounds) = cache.bounds else { return };
        let tree = b.tree();
        let (w, h): (f32, f32) = (bounds.size.width.into(), bounds.size.height.into());
        let key = Key {
            folder: b.folder(),
            visual: self.settings.visual,
            w,
            h,
            total: tree.node(tree.root()).size,
            nodes: tree.len(),
        };
        if cache.key == Some(key) {
            return;
        }
        cache.key = Some(key);
        cache.cells.clear();
        cache.arcs.clear();
        cache.framed.clear();
        match key.visual {
            Visual::Treemap => {
                cache.cells = treemap::treemap(tree, key.folder, Rect::new(0., 0., w, h));
                let framed: HashSet<NodeId> = cache.cells.iter().map(|c| c.parent).collect();
                cache.framed = framed;
            }
            Visual::Sunburst => {
                let radius = (w.min(h) / 2. - 8.).max(10.);
                cache.ring = sunburst::ring(radius);
                cache.arcs = sunburst::sunburst(tree, key.folder, radius);
            }
        }
    }

    fn frame(&self, b: &Browser, view: EntityId) -> Frame {
        self.prepare_layout(b);
        let tree = b.tree();
        let cache = self.visual_cache.borrow();
        let now = now();
        let mode = self.settings.colour;
        let theme = &self.theme;
        let selected_top = b.selected();
        let hovered = self.hover.map(|h| h.target);
        let fill_of =
            |node: Option<NodeId>| colour::fill(node.map(|id| tree.node(id)), mode, theme, now);
        let framed: Vec<bool> = cache
            .cells
            .iter()
            .map(|c| c.node.is_some_and(|id| cache.framed.contains(&id)))
            .collect();
        let (fills, labels, selected, hovered_ix) = match self.settings.visual {
            Visual::Treemap => {
                let fills = cache
                    .cells
                    .iter()
                    .zip(&framed)
                    .map(|(c, framed)| {
                        if *framed {
                            theme.surface
                        } else {
                            fill_of(c.node)
                        }
                    })
                    .collect();
                let labels = cache
                    .cells
                    .iter()
                    .map(|c| {
                        let Some(id) = c.node else {
                            // A folder's merged small items, labelled when there's room.
                            let size =
                                rest_size(tree, &cache.cells, c.parent, |c| c.node, |c| c.parent);
                            return (c.rect.w > 60. && c.rect.h > 30.)
                                .then(|| label("small items", size, c.rect.w - 8., usize::MAX))
                                .flatten();
                        };
                        let framed = cache.framed.contains(&id);
                        let fits = if framed {
                            c.header
                        } else {
                            c.depth <= 3 && c.rect.h > 16. && c.rect.w > 40.
                        };
                        let n = tree.node(id);
                        fits.then(|| label(&n.name, n.size, c.rect.w - 8., 4))
                            .flatten()
                    })
                    .collect();
                let selected = cache
                    .cells
                    .iter()
                    .position(|c| c.node.is_some() && c.node == selected_top);
                let hovered_ix = cache.cells.iter().position(|c| match hovered {
                    Some(Target::Node(id)) => c.node == Some(id),
                    Some(Target::Rest(p)) => c.node.is_none() && c.parent == p,
                    _ => false,
                });
                (fills, labels, selected, hovered_ix)
            }
            Visual::Sunburst => {
                let fills = cache.arcs.iter().map(|a| fill_of(a.node)).collect();
                let labels = cache
                    .arcs
                    .iter()
                    .map(|a| {
                        let id = a.node?;
                        let mid = (a.inner + a.outer) / 2.;
                        let arc_len = (a.end - a.start) * mid;
                        // Whole names or at least 8 characters; stubs say nothing, and the
                        // tooltip has the rest.
                        (arc_len > 60. && cache.ring > 30.)
                            .then(|| fit(&tree.node(id).name, cache.ring - 6., 8))
                            .flatten()
                            .map(SharedString::from)
                    })
                    .collect();
                let selected = cache
                    .arcs
                    .iter()
                    .position(|a| a.node.is_some() && a.node == selected_top);
                let hovered_ix = cache.arcs.iter().position(|a| match hovered {
                    Some(Target::Node(id)) => a.node == Some(id),
                    Some(Target::Rest(p)) => a.node.is_none() && a.parent == p,
                    _ => false,
                });
                (fills, labels, selected, hovered_ix)
            }
        };
        let nodes: Vec<Option<NodeId>> = match self.settings.visual {
            Visual::Treemap => cache.cells.iter().map(|c| c.node).collect(),
            Visual::Sunburst => cache.arcs.iter().map(|a| a.node).collect(),
        };
        let marked = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_some_and(|id| self.marks.is_marked(id)))
            .map(|(i, _)| i)
            .collect();
        let folder = tree.node(b.folder());
        let name = if b.folder() == tree.root() {
            self.root.file_name().map_or_else(
                || self.root.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        } else {
            folder.name.to_string()
        };
        Frame {
            view,
            visual: self.settings.visual,
            cache: self.visual_cache.clone(),
            fills,
            framed,
            labels,
            centre_label: (name.into(), {
                let result = b.result();
                let at_least = if b.folder() == tree.root()
                    && (result.error_count() > 0 || result.cancelled)
                {
                    "at least "
                } else {
                    ""
                };
                format!("{at_least}{}", format::bytes(folder.size)).into()
            }),
            selected,
            hovered: hovered_ix,
            marked,
            theme: theme.clone(),
        }
    }

    /// What lies under a window position.
    fn target_at(&self, pos: Point<Pixels>) -> Option<Target> {
        let cache = self.visual_cache.borrow();
        let bounds = cache.bounds?;
        let (x, y): (f32, f32) = (
            (pos.x - bounds.origin.x).into(),
            (pos.y - bounds.origin.y).into(),
        );
        match self.settings.visual {
            Visual::Treemap => cache
                .cells
                .iter()
                .rev()
                .find(|c| c.rect.contains(x, y))
                .map(|c| c.node.map_or(Target::Rest(c.parent), Target::Node)),
            Visual::Sunburst => {
                let (w, h): (f32, f32) = (bounds.size.width.into(), bounds.size.height.into());
                match sunburst::hit(&cache.arcs, cache.ring, x - w / 2., y - h / 2.)? {
                    None => Some(Target::Centre),
                    Some(i) => {
                        let a = &cache.arcs[i];
                        Some(a.node.map_or(Target::Rest(a.parent), Target::Node))
                    }
                }
            }
        }
    }

    fn hover_at(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let hover = self.target_at(pos).map(|target| Hover { target, at: pos });
        let changed = hover.map(|h| h.target) != self.hover.map(|h| h.target);
        let moved = hover.is_some();
        self.hover = hover;
        if changed || moved {
            cx.notify();
        }
    }

    fn click_visual(&mut self, pos: Point<Pixels>, clicks: usize, cx: &mut Context<Self>) {
        let target = self.target_at(pos);
        self.with_browser(cx, |b| match target {
            Some(Target::Centre) => b.back(),
            Some(Target::Node(id)) if clicks >= 2 => {
                b.jump_to(id);
                if b.can_open(id) {
                    b.open();
                }
            }
            Some(Target::Node(id)) | Some(Target::Rest(id)) => {
                if let Some(top) = top_level(b.tree(), b.folder(), id) {
                    b.jump_to(top);
                }
            }
            None => {}
        });
    }

    pub fn render_visual(&self, b: &Browser, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        if b.rows().is_empty() {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(rgb(t.faint))
                .text_color(rgb(t.dim))
                .child("Nothing here");
        }
        let frame = self.frame(b, cx.entity_id());
        let tooltip = self.hover.and_then(|h| self.tooltip(b, h));
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(rgb(t.faint))
            .child(
                div()
                    .id("visual")
                    .flex_1()
                    .min_h_0()
                    .m_2()
                    .relative()
                    .on_mouse_move(
                        cx.listener(|view, e: &MouseMoveEvent, _, cx| {
                            view.hover_at(e.position, cx)
                        }),
                    )
                    .on_hover(cx.listener(|view, hovered: &bool, _, cx| {
                        if !hovered {
                            view.hover = None;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(|view, e: &ClickEvent, _, cx| {
                        view.click_visual(e.position(), e.click_count(), cx)
                    }))
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, cx| paint(bounds, frame, window, cx),
                        )
                        .size_full(),
                    )
                    .children(tooltip),
            )
            .child(self.render_legend())
    }

    fn tooltip(&self, b: &Browser, hover: Hover) -> Option<impl IntoElement + use<>> {
        let t = &self.theme;
        let tree = b.tree();
        let bounds = self.visual_cache.borrow().bounds?;
        let (title, detail) = match hover.target {
            Target::Centre => return None,
            Target::Rest(parent) => {
                let cache = self.visual_cache.borrow();
                let rest = match self.settings.visual {
                    Visual::Treemap => {
                        rest_size(tree, &cache.cells, parent, |c| c.node, |c| c.parent)
                    }
                    Visual::Sunburst => {
                        rest_size(tree, &cache.arcs, parent, |a| a.node, |a| a.parent)
                    }
                };
                (
                    format!("{}/ (small items)", self.relative(tree, parent)),
                    format!("{}, too small to draw one by one", format::bytes(rest)),
                )
            }
            Target::Node(id) => self.describe(tree, id),
        };
        // Deferred, so it paints above the legend and the rest of the window; in the lower
        // half of the pane it opens upwards, away from the window's bottom edge.
        let lower = hover.at.y > bounds.origin.y + bounds.size.height / 2.;
        let (corner, offset) = if lower {
            (Anchor::BottomLeft, point(px(14.), px(-10.)))
        } else {
            (Anchor::TopLeft, point(px(14.), px(16.)))
        };
        let tip = div()
            .max_w(px(420.))
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(t.surface))
            .border_1()
            .border_color(rgb(t.faint))
            .text_size(px(12.))
            .child(div().font_family(MONO).child(title))
            .child(div().text_color(rgb(t.dim)).child(detail));
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(hover.at)
                    .offset(offset)
                    .snap_to_window_with_margin(px(8.))
                    .child(tip),
            )
            .with_priority(1),
        )
    }

    /// An item's path and a line about it: size, files, kind, age. The tooltip and the line
    /// under the list both show it.
    pub(crate) fn describe(&self, tree: &Tree, id: NodeId) -> (String, String) {
        let n = tree.node(id);
        let mut title = self.relative(tree, id);
        if n.is_dir() {
            title.push('/');
        }
        let size = if n.has(flag::UNREADABLE | flag::NOT_SCANNED | flag::OTHER_VOLUME) {
            "size unknown".to_string()
        } else {
            format::bytes(n.size)
        };
        let detail = if n.is_dir() {
            format!(
                "{size}, {} files, mostly {}, {}",
                count(n.files as u64),
                n.kind.label(),
                age_text(n.mtime as u64)
            )
        } else {
            format!("{size}, {}, {}", n.kind.label(), age_text(n.mtime as u64))
        };
        (title, detail)
    }

    /// A path relative to the scan root, for tooltips.
    fn relative(&self, tree: &Tree, id: NodeId) -> String {
        let path = tree.path(id);
        path.strip_prefix(&self.root)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string())
    }

    fn render_legend(&self) -> impl IntoElement {
        let t = &self.theme;
        let swatch = |colour: u32, text: String| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(div().size(px(9.)).rounded_sm().bg(rgb(colour)))
                .child(text)
        };
        let mut row = div().flex().flex_wrap().gap_x_3().gap_y_1();
        match self.settings.colour {
            ColourMode::Kind => {
                for kind in Kind::ALL {
                    row = row.child(swatch(t.kind(kind), kind.label().into()));
                }
            }
            ColourMode::Age => {
                for (text, days) in AGE_LEGEND {
                    row = row.child(swatch(colour::age(days), text.into()));
                }
                row = row.child(swatch(t.dim, "age unknown".into()));
            }
        }
        let visual = match self.settings.visual {
            Visual::Treemap => "treemap",
            Visual::Sunburst => "sunburst",
        };
        // The keys get their own slot on the right, so they don't move as the swatches wrap.
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap_3()
            .px_3()
            .pb_2()
            .text_size(px(11.))
            .text_color(rgb(t.dim))
            .child(row.child(swatch(t.small_items(), "small items".into())))
            .child(
                div()
                    .flex_shrink_0()
                    .child(format!("v: {visual}, a: colour, ?: keys")),
            )
    }
}

fn age_text(mtime: u64) -> String {
    if mtime == 0 {
        return "age unknown".into();
    }
    let days = now().saturating_sub(mtime) / 86_400;
    match days {
        0 => "changed today".into(),
        1..=59 => format!("changed {days} days ago"),
        60..=729 => format!("changed {} months ago", days / 30),
        _ => format!("changed {:.1} years ago", days as f32 / 365.),
    }
}

fn paint(bounds: Bounds<Pixels>, frame: Frame, window: &mut Window, cx: &mut App) {
    let stale = {
        let mut cache = frame.cache.borrow_mut();
        let stale = cache.bounds != Some(bounds);
        cache.bounds = Some(bounds);
        stale
    };
    if stale {
        // The layout was made for other bounds (or none yet): render again to lay out. A
        // notify during paint is dropped, so it waits for the next frame.
        let view = frame.view;
        window.on_next_frame(move |_, cx| cx.notify(view));
    }
    let cache = frame.cache.borrow();
    let o = bounds.origin;
    match frame.visual {
        Visual::Treemap => paint_treemap(&cache.cells, &frame, o, window, cx),
        Visual::Sunburst => {
            let centre = point(o.x + bounds.size.width / 2., o.y + bounds.size.height / 2.);
            paint_sunburst(&cache.arcs, cache.ring, &frame, centre, window, cx)
        }
    }
}

fn rect_bounds(r: &Rect, o: Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(point(o.x + px(r.x), o.y + px(r.y)), size(px(r.w), px(r.h)))
}

fn text(s: &SharedString, at: Point<Pixels>, colour: u32, window: &mut Window, cx: &mut App) {
    let run = TextRun {
        len: s.len(),
        font: window.text_style().font(),
        color: hsla(colour),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(s.clone(), px(11.), &[run], None);
    let _ = line.paint(at, px(14.), TextAlign::Left, None, window, cx);
}

fn paint_treemap(cells: &[Cell], f: &Frame, o: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let t = &f.theme;
    for (i, c) in cells.iter().enumerate() {
        let b = rect_bounds(&c.rect, o);
        let framed = f.framed[i];
        if framed {
            window.paint_quad(quad(
                b,
                px(2.),
                hsla(t.surface),
                px(1.),
                hsla(t.faint),
                BorderStyle::Solid,
            ));
        } else {
            window.paint_quad(quad(
                b,
                px(2.),
                hsla(f.fills[i]),
                px(0.5),
                hsla(t.bg),
                BorderStyle::Solid,
            ));
        }
        if let Some(label) = &f.labels[i] {
            let ink = if framed { t.dim } else { t.ink(f.fills[i]) };
            let y = if c.header { (HEADER - 14.) / 2. } else { 3. };
            text(
                label,
                point(b.origin.x + px(4.), b.origin.y + px(y)),
                ink,
                window,
                cx,
            );
        }
    }
    // Marks are dashed in the text colour, so no fill colour can hide them and they never
    // read as the selection (solid).
    for &i in &f.marked {
        window.paint_quad(quad(
            rect_bounds(&cells[i].rect, o),
            px(2.),
            transparent_black(),
            px(2.),
            hsla(t.text),
            BorderStyle::Dashed,
        ));
    }
    for (ix, width) in [(f.hovered, 1.5), (f.selected, 2.5)] {
        if let Some(i) = ix {
            let b = rect_bounds(&cells[i].rect, o);
            window.paint_quad(quad(
                b,
                px(2.),
                transparent_black(),
                px(width),
                hsla(t.text),
                BorderStyle::Solid,
            ));
        }
    }
}

/// An arc as a polygon, clockwise from 12 o'clock.
fn arc_path(a: &Arc, centre: Point<Pixels>, builder: PathBuilder) -> Option<Path<Pixels>> {
    let mut p = builder;
    let steps = (((a.end - a.start) / TAU) * 180.).ceil().max(1.) as usize;
    let at = |r: f32, angle: f32| {
        point(
            centre.x + px(r * angle.sin()),
            centre.y - px(r * angle.cos()),
        )
    };
    let gap = 0.6 / a.outer.max(1.);
    let (start, end) = (a.start + gap.min((a.end - a.start) / 4.), a.end);
    p.move_to(at(a.outer - 0.5, start));
    for s in 1..=steps {
        p.line_to(at(
            a.outer - 0.5,
            start + (end - start) * s as f32 / steps as f32,
        ));
    }
    for s in (0..=steps).rev() {
        p.line_to(at(
            a.inner + 0.5,
            start + (end - start) * s as f32 / steps as f32,
        ));
    }
    p.close();
    p.build().ok()
}

fn paint_sunburst(
    arcs: &[Arc],
    ring: f32,
    f: &Frame,
    centre: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let t = &f.theme;
    for (i, a) in arcs.iter().enumerate() {
        if let Some(path) = arc_path(a, centre, PathBuilder::fill()) {
            window.paint_path(path, hsla(f.fills[i]));
        }
    }
    let disc = Bounds::new(
        point(centre.x - px(ring - 2.), centre.y - px(ring - 2.)),
        size(px(2. * ring - 4.), px(2. * ring - 4.)),
    );
    window.paint_quad(quad(
        disc,
        px(ring),
        hsla(t.surface),
        px(1.),
        hsla(t.faint),
        BorderStyle::Solid,
    ));
    let (name, total) = &f.centre_label;
    if let Some(name) = fit(name, 2. * ring - 16., 4).map(SharedString::from) {
        let w = name.chars().count() as f32 * CHAR_W;
        text(
            &name,
            point(centre.x - px(w / 2.), centre.y - px(15.)),
            t.text,
            window,
            cx,
        );
    }
    let w = total.chars().count() as f32 * 6.4;
    text(
        total,
        point(centre.x - px(w / 2.), centre.y + px(1.)),
        t.dim,
        window,
        cx,
    );

    for (i, a) in arcs.iter().enumerate() {
        if let Some(label) = &f.labels[i] {
            let mid = (a.start + a.end) / 2.;
            let r = (a.inner + a.outer) / 2.;
            let w = label.chars().count() as f32 * 6.4;
            let at = point(
                centre.x + px(r * mid.sin() - w / 2.),
                centre.y - px(r * mid.cos() + 7.),
            );
            text(label, at, t.ink(f.fills[i]), window, cx);
        }
    }
    for &i in &f.marked {
        let dashed = PathBuilder::stroke(px(2.)).dash_array(&[px(5.), px(4.)]);
        if let Some(path) = arc_path(&arcs[i], centre, dashed) {
            window.paint_path(path, hsla(t.text));
        }
    }
    for (ix, width) in [(f.hovered, 1.5), (f.selected, 2.5)] {
        if let Some(i) = ix
            && let Some(path) = arc_path(&arcs[i], centre, PathBuilder::stroke(px(width)))
        {
            window.paint_path(path, hsla(t.text));
        }
    }
}
