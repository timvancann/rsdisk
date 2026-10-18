//! Cell colours for the visuals (spec §3.6): file kind from the theme, or age on the
//! mockup's ramp.

use crate::settings::ColourMode;
use crate::theme::Theme;
use rsdisk_core::tree::Node;

const DAY: f32 = 86_400.;
/// Recent, middle and old: teal, sand, red.
const RAMP: [u32; 3] = [0x4fa3a5, 0xd8c27a, 0xd9534f];
/// The ramp ends at five years.
const OLD_DAYS: f32 = 1825.;

/// The fill for a cell: `None` is a folder's merged small items.
pub fn fill(node: Option<&Node>, mode: ColourMode, theme: &Theme, now: u64) -> u32 {
    let Some(n) = node else {
        return theme.small_items();
    };
    match mode {
        ColourMode::Kind => theme.kind(n.kind),
        ColourMode::Age if n.mtime == 0 => theme.dim,
        ColourMode::Age => age(now.saturating_sub(n.mtime as u64) as f32 / DAY),
    }
}

/// A colour on the ramp for an age in days, on a log scale from 1 day to five years.
pub fn age(days: f32) -> u32 {
    let t = (days.max(1.).ln() / OLD_DAYS.ln()).clamp(0., 1.);
    let (from, to, local) = if t < 0.5 {
        (RAMP[0], RAMP[1], t * 2.)
    } else {
        (RAMP[1], RAMP[2], (t - 0.5) * 2.)
    };
    let mix = |shift: u32| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        ((a + (b - a) * local).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// Legend entries for the age ramp.
pub const AGE_LEGEND: [(&str, f32); 5] = [
    ("today", 1.),
    ("1 month", 30.),
    ("6 months", 182.),
    ("1 year", 365.),
    ("5 years+", OLD_DAYS),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin_dark;
    use rsdisk_core::kind::Kind;

    const NOW: u64 = 1_800_000_000;

    fn node(kind: Kind, days_old: u64) -> Node {
        let mtime = if days_old == u64::MAX {
            0
        } else {
            (NOW - days_old * 86_400) as i64
        };
        Node::new("x", 1, mtime, kind, 0)
    }

    #[test]
    fn kind_mode_uses_the_theme_slot() {
        let t = builtin_dark();
        for kind in Kind::ALL {
            assert_eq!(
                fill(Some(&node(kind, 3)), ColourMode::Kind, &t, NOW),
                t.kind(kind)
            );
        }
    }

    #[test]
    fn age_runs_from_teal_through_sand_to_red() {
        assert_eq!(age(1.), RAMP[0]);
        assert_eq!(age(0.2), RAMP[0]);
        assert_eq!(age(OLD_DAYS.sqrt()), RAMP[1]);
        assert_eq!(age(3650.), RAMP[2]);
        let t = builtin_dark();
        assert_eq!(
            fill(Some(&node(Kind::Video, 3650)), ColourMode::Age, &t, NOW),
            RAMP[2]
        );
    }

    #[test]
    fn undated_nodes_and_remainders_are_neutral() {
        let t = builtin_dark();
        assert_eq!(
            fill(Some(&node(Kind::Video, u64::MAX)), ColourMode::Age, &t, NOW),
            t.dim
        );
        assert_eq!(fill(None, ColourMode::Kind, &t, NOW), t.small_items());
        assert_ne!(t.small_items(), t.dim);
        assert_ne!(t.small_items(), t.surface);
    }
}
