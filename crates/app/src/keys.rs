//! Keystrokes to actions (spec §3.9), including the two-key `gg`. Later plans add handlers
//! for actions that parse here already.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Down,
    Up,
    Top,
    Bottom,
    HalfPageDown,
    HalfPageUp,
    Open,
    Back,
    CycleSort,
    ToggleMark,
    UnmarkAll,
    Trash,
    Search,
    NextMatch,
    PrevMatch,
    Rescan,
    ScanCloud,
    SwitchVisual,
    SwitchColour,
    Reveal,
    Help,
    Cancel,
    Quit,
}

#[derive(Debug, Default)]
pub struct Keymap {
    pending_g: bool,
}

impl Keymap {
    /// One keystroke: `key` is the typed character (`G` for shift-g) or a key name such as
    /// `enter`, `backspace`, `escape`, `down`; `ctrl` is whether control was held.
    pub fn feed(&mut self, key: &str, ctrl: bool) -> Option<Action> {
        let was_g = std::mem::take(&mut self.pending_g);
        if ctrl {
            return match key {
                "d" => Some(Action::HalfPageDown),
                "u" => Some(Action::HalfPageUp),
                _ => None,
            };
        }
        Some(match key {
            "g" if was_g => Action::Top,
            "g" => {
                self.pending_g = true;
                return None;
            }
            "j" | "down" => Action::Down,
            "k" | "up" => Action::Up,
            "G" | "end" => Action::Bottom,
            "home" => Action::Top,
            "pagedown" => Action::HalfPageDown,
            "pageup" => Action::HalfPageUp,
            "l" | "enter" | "right" => Action::Open,
            "h" | "backspace" | "left" => Action::Back,
            "s" => Action::CycleSort,
            " " | "space" => Action::ToggleMark,
            "u" => Action::UnmarkAll,
            "d" => Action::Trash,
            "/" => Action::Search,
            "n" => Action::NextMatch,
            "N" => Action::PrevMatch,
            "r" => Action::Rescan,
            "c" => Action::ScanCloud,
            "v" => Action::SwitchVisual,
            "a" => Action::SwitchColour,
            "o" => Action::Reveal,
            "?" => Action::Help,
            "escape" => Action::Cancel,
            "q" => Action::Quit,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(keys: &[&str]) -> Vec<Option<Action>> {
        let mut k = Keymap::default();
        keys.iter().map(|key| k.feed(key, false)).collect()
    }

    #[test]
    fn gg_goes_to_the_top() {
        assert_eq!(feed(&["g", "g"]), [None, Some(Action::Top)]);
    }

    #[test]
    fn a_lone_g_is_dropped_by_the_next_key() {
        assert_eq!(feed(&["g", "j"]), [None, Some(Action::Down)]);
        assert_eq!(feed(&["g", "x", "g"]), [None, None, None]);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_page() {
        let mut k = Keymap::default();
        assert_eq!(k.feed("d", true), Some(Action::HalfPageDown));
        assert_eq!(k.feed("u", true), Some(Action::HalfPageUp));
        assert_eq!(k.feed("j", true), None);
    }

    #[test]
    fn ctrl_clears_a_pending_g() {
        let mut k = Keymap::default();
        k.feed("g", false);
        k.feed("d", true);
        assert_eq!(k.feed("g", false), None);
    }

    #[test]
    fn single_keys() {
        let cases = [
            ("j", Action::Down),
            ("k", Action::Up),
            ("G", Action::Bottom),
            ("l", Action::Open),
            ("enter", Action::Open),
            ("h", Action::Back),
            ("backspace", Action::Back),
            ("s", Action::CycleSort),
            ("o", Action::Reveal),
            ("?", Action::Help),
            ("q", Action::Quit),
            ("escape", Action::Cancel),
            (" ", Action::ToggleMark),
            ("v", Action::SwitchVisual),
            ("a", Action::SwitchColour),
            ("down", Action::Down),
        ];
        for (key, action) in cases {
            assert_eq!(Keymap::default().feed(key, false), Some(action), "{key}");
        }
    }

    #[test]
    fn unknown_keys_do_nothing() {
        assert_eq!(feed(&["x", "Z", "f1"]), [None, None, None]);
    }
}
