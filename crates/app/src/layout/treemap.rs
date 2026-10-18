//! Squarified treemap (Bruls, Huizing and van Wijk) of a folder, nested as deep as cells stay
//! visible. Children too small to see are merged into one "(other)" cell per folder.

use super::Rect;
use rsdisk_core::tree::{NodeId, Tree};

/// Children whose cell would be smaller than this (in px², about 6 by 6) join the remainder.
const MIN_AREA: f32 = 36.;
/// Folders at these depths get a strip for their label.
const HEADED_DEPTHS: u8 = 2;
pub const HEADER: f32 = 16.;
/// Folders smaller than this on either side aren't subdivided.
const MIN_SIDE: f32 = 8.;

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The node drawn, or `None` for the merged remainder of `parent`.
    pub node: Option<NodeId>,
    pub parent: NodeId,
    /// 1 for the shown folder's children, 2 for theirs, and so on.
    pub depth: u8,
    pub rect: Rect,
    /// Whether the cell is a folder drawn with a label strip at the top.
    pub header: bool,
}

/// The cells for `folder` laid out in `bounds`, parents before their children.
pub fn treemap(tree: &Tree, folder: NodeId, bounds: Rect) -> Vec<Cell> {
    let mut cells = Vec::new();
    lay_out(tree, folder, bounds, 1, &mut cells);
    cells
}

fn lay_out(tree: &Tree, folder: NodeId, rect: Rect, depth: u8, cells: &mut Vec<Cell>) {
    let children: Vec<(NodeId, u64)> = tree
        .children(folder)
        .iter()
        .map(|c| (*c, tree.node(*c).size))
        .filter(|(_, s)| *s > 0)
        .collect();
    let total: u64 = children.iter().map(|(_, s)| s).sum();
    if total == 0 || rect.w < 1. || rect.h < 1. {
        return;
    }
    let per_byte = rect.area() / total as f32;
    // Largest first, so everything after the first too-small child is too small as well.
    let kept = children
        .iter()
        .take_while(|(_, s)| *s as f32 * per_byte >= MIN_AREA)
        .count();
    let rest: u64 = children[kept..].iter().map(|(_, s)| s).sum();
    let mut areas: Vec<f32> = children[..kept]
        .iter()
        .map(|(_, s)| *s as f32 * per_byte)
        .collect();
    if rest > 0 {
        areas.push(rest as f32 * per_byte);
    }
    let rects = squarify(&areas, rect);

    for (i, r) in rects.into_iter().enumerate() {
        let Some(&(id, _)) = children[..kept].get(i) else {
            cells.push(Cell {
                node: None,
                parent: folder,
                depth,
                rect: r,
                header: false,
            });
            continue;
        };
        let node = tree.node(id);
        let nested = node.is_dir() && !tree.children(id).is_empty();
        let header = nested && depth <= HEADED_DEPTHS && r.h > HEADER * 2. && r.w > 40.;
        cells.push(Cell {
            node: Some(id),
            parent: folder,
            depth,
            rect: r,
            header,
        });
        if nested && r.w >= MIN_SIDE && r.h >= MIN_SIDE {
            let inner = if header {
                Rect::new(r.x + 2., r.y + HEADER, r.w - 4., r.h - HEADER - 2.)
            } else {
                Rect::new(r.x + 1., r.y + 1., r.w - 2., r.h - 2.)
            };
            lay_out(tree, id, inner, depth.saturating_add(1), cells);
        }
    }
}

/// Splits `rect` into rectangles of the given areas (which sum to its area), in order,
/// keeping them as close to square as the order allows.
pub fn squarify(areas: &[f32], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(areas.len());
    let mut r = rect;
    let mut i = 0;
    while i < areas.len() {
        let side = r.w.min(r.h);
        let mut end = i + 1;
        let mut best = worst(&areas[i..end], side);
        while end < areas.len() {
            let next = worst(&areas[i..=end], side);
            if next > best {
                break;
            }
            best = next;
            end += 1;
        }
        let row = &areas[i..end];
        let sum: f32 = row.iter().sum();
        if r.w >= r.h {
            // A column along the left edge.
            let w = if r.h > 0. { sum / r.h } else { 0. };
            let mut y = r.y;
            for a in row {
                let h = if w > 0. { a / w } else { 0. };
                out.push(Rect::new(r.x, y, w, h));
                y += h;
            }
            r.x += w;
            r.w = (r.w - w).max(0.);
        } else {
            // A row along the top edge.
            let h = if r.w > 0. { sum / r.w } else { 0. };
            let mut x = r.x;
            for a in row {
                let w = if h > 0. { a / h } else { 0. };
                out.push(Rect::new(x, r.y, w, h));
                x += w;
            }
            r.y += h;
            r.h = (r.h - h).max(0.);
        }
        i = end;
    }
    out
}

/// The worst aspect ratio of a row of areas laid along a side of length `side`.
fn worst(row: &[f32], side: f32) -> f32 {
    let sum: f32 = row.iter().sum();
    let (max, min) = row
        .iter()
        .fold((f32::MIN, f32::MAX), |(hi, lo), a| (hi.max(*a), lo.min(*a)));
    if sum <= 0. || min <= 0. {
        return f32::MAX;
    }
    let s2 = side * side;
    (s2 * max / (sum * sum)).max(sum * sum / (s2 * min))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsdisk_core::kind::Kind;
    use rsdisk_core::tree::{Node, TreeBuilder, flag};

    fn dir(name: &str) -> Node {
        Node::new(name, 0, 0, Kind::Other, flag::DIR)
    }

    fn file(name: &str, size: u64) -> Node {
        Node::new(name, size, 0, Kind::Other, 0)
    }

    fn flat(sizes: &[u64]) -> Tree {
        let mut b = TreeBuilder::new(dir("/r"));
        b.add_children(
            0,
            sizes
                .iter()
                .enumerate()
                .map(|(i, s)| file(&format!("f{i}"), *s)),
        );
        b.finish()
    }

    const BOUNDS: Rect = Rect {
        x: 10.,
        y: 20.,
        w: 600.,
        h: 400.,
    };

    fn ratio(r: &Rect) -> f32 {
        r.w.max(r.h) / r.w.min(r.h)
    }

    #[test]
    fn cells_tile_the_folder_exactly() {
        let t = flat(&[6, 6, 4, 3, 2, 2, 1]);
        let cells = treemap(&t, 0, BOUNDS);
        let area: f32 = cells.iter().map(|c| c.rect.area()).sum();
        assert!((area - BOUNDS.area()).abs() < 1.0, "{area}");
        assert!(cells.iter().all(|c| BOUNDS.encloses(&c.rect)));
    }

    #[test]
    fn areas_are_proportional_to_size() {
        let t = flat(&[6, 6, 4, 3, 2, 2, 1]);
        let cells = treemap(&t, 0, BOUNDS);
        let per = BOUNDS.area() / 24.;
        for c in &cells {
            let size = t.node(c.node.unwrap()).size as f32;
            assert!((c.rect.area() - size * per).abs() < 1.0);
        }
    }

    #[test]
    fn cells_stay_roughly_square() {
        let t = flat(&[6, 6, 4, 3, 2, 2, 1]);
        for c in treemap(&t, 0, BOUNDS) {
            assert!(ratio(&c.rect) < 4., "{:?}", c.rect);
        }
    }

    #[test]
    fn tiny_children_merge_into_one_remainder() {
        let mut sizes = vec![1_000_000u64];
        sizes.extend(std::iter::repeat_n(1, 50));
        let cells = treemap(&flat(&sizes), 0, BOUNDS);
        assert_eq!(cells.len(), 2);
        let rest = cells.iter().find(|c| c.node.is_none()).unwrap();
        assert_eq!(rest.parent, 0);
        let area: f32 = cells.iter().map(|c| c.rect.area()).sum();
        assert!((area - BOUNDS.area()).abs() < 1.0);
    }

    #[test]
    fn empty_folders_and_zero_sizes_give_nothing() {
        assert!(treemap(&flat(&[]), 0, BOUNDS).is_empty());
        assert!(treemap(&flat(&[0, 0]), 0, BOUNDS).is_empty());
    }

    #[test]
    fn a_single_child_fills_the_folder() {
        let cells = treemap(&flat(&[42]), 0, BOUNDS);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].rect, BOUNDS);
    }

    #[test]
    fn folders_nest_inside_their_cell_below_a_header() {
        // /r { a { x:300, y:100 }, b:200 }
        let mut b = TreeBuilder::new(dir("/r"));
        let a = b.add_children(0, [dir("a"), file("b", 200)]);
        b.add_children(a, [file("x", 300), file("y", 100)]);
        let t = b.finish();
        let cells = treemap(&t, 0, BOUNDS);
        let outer = cells.iter().find(|c| c.node == Some(a)).unwrap();
        assert!(outer.header);
        let inner: Vec<&Cell> = cells.iter().filter(|c| c.parent == a).collect();
        assert_eq!(inner.len(), 2);
        for c in &inner {
            assert_eq!(c.depth, 2);
            assert!(outer.rect.encloses(&c.rect));
            assert!(c.rect.y >= outer.rect.y + HEADER - 0.01);
        }
    }
}
