//! Global-shortcut repeat filtering (M0 T0.3). OS key-repeat delivers repeated
//! "pressed" events while a chord is held; only the first press after a
//! release is accepted, so holding the chord can never start and stop.

#[derive(Debug, Default)]
pub struct ChordFilter {
    held: bool,
}

impl ChordFilter {
    /// Returns true if this event is an accepted (non-repeat) press.
    pub fn on_event(&mut self, pressed: bool) -> bool {
        if pressed {
            let accept = !self.held;
            self.held = true;
            accept
        } else {
            self.held = false;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChordFilter;

    #[test]
    fn held_chord_starts_once() {
        let mut f = ChordFilter::default();
        let accepted: Vec<bool> = [true, true, true, false, true, false]
            .iter()
            .map(|p| f.on_event(*p))
            .collect();
        assert_eq!(accepted, [true, false, false, false, true, false]);
    }
}
