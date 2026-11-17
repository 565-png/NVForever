//! Existing activation-range publication, factored without adding source events.
use crate::game::Game;
use fnv_sim::FormId;

impl Game<'_> {
    pub fn set_crosshair_ref(&mut self, now: Option<FormId>) {
        if let Some(r) = now.filter(|_| now != self.senses.crosshair_ref) {
            self.jg8_crosshair(r);
        }
        self.senses.crosshair_ref = now;
    }
}
