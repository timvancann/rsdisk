//! Sunburst of a folder: the folder is the centre disc, each ring one level deeper, each
//! child's angle proportional to its size. Slivers are merged into one "(other)" arc.

use rsdisk_core::tree::{NodeId, Tree};
use std::f32::consts::TAU;

/// Rings beyond the centre.
pub const RINGS: u8 = 5;
/// Children narrower than this (about one degree) join the remainder.
const MIN_ANGLE: f32 = TAU / 360.;

#[derive(Debug, Clone, PartialEq)]
pub struct Arc {
    /// The node drawn, or `None` for the merged remainder of `parent`.
    pub node: Option<NodeId>,
    pub parent: NodeId,
    /// 1 for the first ring around the centre.
    pub depth: u8,
    pub start: f32,
    pub end: f32,
    pub inner: f32,
    pub outer: f32,
}

/// The ring width for a sunburst of `radius`: the centre disc and each ring are equally wide.
pub fn ring(radius: f32) -> f32 {
    radius / (RINGS as f32 + 1.)
}

/// The arcs around `folder` for a sunburst of `radius`, parents before their children.
pub fn sunburst(tree: &Tree, folder: NodeId, radius: f32) -> Vec<Arc> {
    let mut arcs = Vec::new();
    lay_out(tree, folder, 0., TAU, 1, ring(radius), &mut arcs);
    arcs
}

fn lay_out(
    tree: &Tree,
    folder: NodeId,
    start: f32,
    span: f32,
    depth: u8,
    ring: f32,
    arcs: &mut Vec<Arc>,
) {
    if depth > RINGS {
        return;
    }
    let children: Vec<(NodeId, u64)> = tree
        .children(folder)
        .iter()
        .map(|c| (*c, tree.node(*c).size))
        .filter(|(_, s)| *s > 0)
        .collect();
    let total: u64 = children.iter().map(|(_, s)| s).sum();
    if total == 0 {
        return;
    }
    let per_byte = span / total as f32;
    let (inner, outer) = (ring * depth as f32, ring * (depth as f32 + 1.));
    let mut at = start;
    let mut rest = 0;
    for (id, size) in children {
        let angle = size as f32 * per_byte;
        if angle < MIN_ANGLE {
            rest += size;
            continue;
        }
        arcs.push(Arc {
            node: Some(id),
            parent: folder,
            depth,
            start: at,
            end: at + angle,
            inner,
            outer,
        });
        lay_out(tree, id, at, angle, depth + 1, ring, arcs);
        at += angle;
    }
    if rest > 0 {
        arcs.push(Arc {
            node: None,
            parent: folder,
            depth,
            start: at,
            end: at + rest as f32 * per_byte,
            inner,
            outer,
        });
    }
}

/// What lies under a point relative to the centre: `Some(None)` is the centre disc (the
/// folder itself), `Some(Some(i))` the arc at index `i`, `None` nothing.
pub fn hit(arcs: &[Arc], ring: f32, x: f32, y: f32) -> Option<Option<usize>> {
    let r = (x * x + y * y).sqrt();
    if r < ring {
        return Some(None);
    }
    // Angles run clockwise from 12 o'clock, as drawn.
    let angle = x.atan2(-y).rem_euclid(TAU);
    arcs.iter()
        .position(|a| r >= a.inner && r < a.outer && angle >= a.start && angle < a.end)
        .map(Some)
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

    /// /r { a { x:300, y:100 }, b:200, tiny:0.. }
    fn sample(tiny: usize) -> (Tree, NodeId) {
        let mut b = TreeBuilder::new(dir("/r"));
        let a = b.add_children(
            0,
            [dir("a"), file("b", 200)]
                .into_iter()
                .chain((0..tiny).map(|i| file(&format!("t{i}"), 1))),
        );
        b.add_children(a, [file("x", 300), file("y", 100)]);
        (b.finish(), a)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_first_ring_covers_the_full_circle_in_proportion() {
        let (t, a) = sample(0);
        let arcs = sunburst(&t, 0, 120.);
        let first: Vec<&Arc> = arcs.iter().filter(|x| x.depth == 1).collect();
        assert_eq!(first.len(), 2);
        assert!(close(first[0].start, 0.));
        assert_eq!(first[0].node, Some(a));
        assert!(close(first[0].end, TAU * 400. / 600.));
        assert!(close(first[1].end, TAU));
    }

    #[test]
    fn children_cover_their_parents_span() {
        let (t, a) = sample(0);
        let arcs = sunburst(&t, 0, 120.);
        let parent = arcs.iter().find(|x| x.node == Some(a)).unwrap();
        let kids: Vec<&Arc> = arcs.iter().filter(|x| x.parent == a).collect();
        assert!(close(kids[0].start, parent.start));
        assert!(close(kids.last().unwrap().end, parent.end));
        assert!(close(
            kids[0].end - kids[0].start,
            (parent.end - parent.start) * 0.75
        ));
        assert!(close(kids[0].inner, parent.outer));
    }

    #[test]
    fn nothing_beyond_the_last_ring() {
        // A chain of nested folders ten deep.
        let mut b = TreeBuilder::new(dir("/r"));
        let mut at = 0;
        for i in 0..10 {
            at = b.add_children(at, [dir(&format!("d{i}"))]);
        }
        b.add_children(at, [file("leaf", 5)]);
        let arcs = sunburst(&b.finish(), 0, 120.);
        assert_eq!(arcs.len(), RINGS as usize);
        assert!(
            arcs.iter()
                .all(|x| x.depth <= RINGS && x.outer <= 120. + 1e-3)
        );
    }

    #[test]
    fn slivers_merge_into_one_remainder() {
        let (t, _) = sample(200);
        let arcs = sunburst(&t, 0, 120.);
        let rest: Vec<&Arc> = arcs.iter().filter(|x| x.node.is_none()).collect();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].parent, 0);
        assert!(close(rest[0].end, TAU));
    }

    #[test]
    fn hit_finds_the_centre_an_arc_or_nothing() {
        let (t, a) = sample(0);
        let arcs = sunburst(&t, 0, 120.);
        let ring = ring(120.);
        assert_eq!(hit(&arcs, ring, 0., 0.), Some(None));
        // Just right of 12 o'clock in the first ring: the start of `a`.
        let i = hit(&arcs, ring, 1., -(ring * 1.5)).unwrap().unwrap();
        assert_eq!(arcs[i].node, Some(a));
        assert_eq!(hit(&arcs, ring, 500., 0.), None);
    }
}
