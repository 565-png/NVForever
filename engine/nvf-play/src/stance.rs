//! Sprint: the shared state of NVF Features › Advanced Movement's Sprint switch [design: the
//! owner 2026-11-18]. Off by default (`crate::inertia::MovementInertia::sprint`) and acting
//! only under the Global Toggle ([`MovementInertia::sprint_on`]). This module holds the
//! plumbing and the contract; the feature itself (speed, AP, out of breath) is
//! `crate::sprint`, its look nvforever's `sprint_view.rs`.
//!
//! # The contract
//!
//! **Switch**: read [`MovementInertia::sprint_on`] (`advanced && sprint`), never the bare
//! field, so the Global Toggle turns it off. Command line `--sprint` / `--no-sprint`; row
//! `sprint` (registered in `crate::player`).
//!
//! **Input** (`crate::control::read_controls`, `PlaySet::Input`; struck by the scripts'
//! locks in `crate::control::enforce_locks`, the frame's input already applied):
//! [`PlayerControl::sprint_held`]: the Sprint control (Left Shift by default) is held and
//! Sprint is on. Struck by the movement lock.
//!
//! **Keys** (`crate::bindings`): Sprint is a binding of its own (Controls › Movement), greyed
//! and not rebindable while its switch is off ([`binding_greyed`]); off, it acts on nothing.
//! While Sprint is on, Left Shift is Sprint's and Run acts on X, unless the player moved Run
//! off its default ([`Bindings::effective`], worked out each time, never saved). A rebind
//! never swaps Sprint and Run ([`crate::bindings::SHARED_PAIRS`]). [`sync_bindings`] keeps
//! [`Bindings::sprint_on`] on the switch.
//!
//! **Motion** ([`StanceMotion`], one resource the camera and animation code read): the
//! Sprint feature writes [`StanceMotion::sprinting`], [`StanceMotion::sprint_share`] and
//! [`StanceMotion::winded`]; nothing else writes them; everything may read them. With the
//! switch off the feature leaves its fields at rest (false, 0) after easing out.
//!
//! **Sprint and the perks**: the speed perks and stats already scale every gait
//! (`crate::player::PerkSpeed`, Modify Run Speed perks; `crate::player::StatSpeed`, Speed
//! Mult and armour; `crate::player::RunPace`): the Sprint feature multiplies the run speed
//! they give, so they apply to it, and spends AP through the same paths the game's AP costs
//! use (`crate::combat`, the AP actor value), so the AP perks apply.
//!
//! [`MovementInertia::sprint_on`]: crate::inertia::MovementInertia::sprint_on
//! [`PlayerControl::sprint_held`]: crate::control::PlayerControl::sprint_held
//! [`Bindings::effective`]: crate::bindings::Bindings::effective
//! [`Bindings::sprint_on`]: crate::bindings::Bindings::sprint_on

use crate::bindings::{Bindings, Control};
use crate::inertia::MovementInertia;
use bevy::prelude::*;

/// What the Sprint feature is doing now, for the camera and the animation code (module notes:
/// who writes which field).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct StanceMotion {
    /// The share of a sprint now, 0 (none) to 1 (a full sprint), eased in and out by the
    /// Sprint feature: what the camera, the arms and the body blend by.
    pub sprint_share: f32,
    /// Sprinting is the target state (the key held, moving forward, AP left).
    pub sprinting: bool,
    /// Out of breath after a sprint, 0 (not) to 1 (fully winded): the strength of the view's
    /// and the gun's breathing shake.
    pub winded: f32,
}

/// Why control `c`'s Controls row is greyed now (Sprint with its switch off, or the Global
/// Toggle off), in the words the page shows under the row; `None`: it is not. A greyed
/// binding cannot be rebound or cleared (`crate::bindings::rebind`). Without
/// [`MovementInertia`] nothing is greyed.
pub fn binding_greyed(world: &World, c: Control) -> Option<&'static str> {
    let m = world.get_resource::<MovementInertia>()?;
    let (own, why) = match c {
        Control::Sprint => (m.sprint, SPRINT_OFF),
        _ => return None,
    };
    if !own {
        Some(why)
    } else if !m.advanced {
        Some(GLOBAL_TOGGLE_OFF)
    } else {
        None
    }
}

/// What a greyed Sprint binding's note says ([`binding_greyed`]).
pub const SPRINT_OFF: &str = "Turn on Sprint in NVF Features > Advanced Movement first.";
pub const GLOBAL_TOGGLE_OFF: &str = "Turn on the Global Toggle in NVF Features > Advanced Movement first.";

/// Keep [`Bindings::sprint_on`] on the switch, so the controls act on the effective keys
/// (`crate::bindings::Bindings::effective`) from the next read.
pub fn sync_bindings(inertia: Option<Res<MovementInertia>>, bindings: Option<ResMut<Bindings>>) {
    let (Some(m), Some(mut b)) = (inertia, bindings) else { return };
    let now = m.sprint_on();
    if b.sprint_on != now {
        b.sprint_on = now;
        let run = b.effective(Control::Run);
        let moved = if run != b.get(Control::Run) { format!("; Run on {} while Sprint has {}", run.text(), b.get(Control::Run).text()) } else { String::new() };
        info!("controls: Sprint {}{moved}", if now { "on" } else { "off" });
    }
}

/// [`StanceMotion`] and the bindings kept on the switch ([`sync_bindings`], before the
/// frame's input).
pub struct StancePlugin;

impl Plugin for StancePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StanceMotion>().add_systems(PreUpdate, sync_bindings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bindings_follow_the_switch_and_a_greyed_row_says_why() {
        let mut app = App::new();
        app.init_resource::<Bindings>().init_resource::<MovementInertia>().add_plugins(StancePlugin);
        app.update();
        assert_eq!(*app.world().resource::<StanceMotion>(), StanceMotion::default());
        assert!(!app.world().resource::<Bindings>().sprint_on, "off by default");
        assert_eq!(binding_greyed(app.world(), Control::Sprint), Some(SPRINT_OFF));
        assert_eq!(binding_greyed(app.world(), Control::Run), None);
        app.world_mut().resource_mut::<MovementInertia>().sprint = true;
        app.update();
        let b = app.world().resource::<Bindings>();
        assert!(b.sprint_on);
        assert_eq!(b.effective(Control::Run).key, Some(crate::bindings::RUN_SPARE_KEY));
        assert_eq!(binding_greyed(app.world(), Control::Sprint), None);
        // The Global Toggle off: Sprint acts on nothing, greyed for it, and keeps its choice.
        app.world_mut().resource_mut::<MovementInertia>().advanced = false;
        app.update();
        let b = app.world().resource::<Bindings>();
        assert!(!b.sprint_on);
        assert_eq!(b.effective(Control::Run).key, Some(KeyCode::ShiftLeft), "Shift is Run's again");
        assert_eq!(binding_greyed(app.world(), Control::Sprint), Some(GLOBAL_TOGGLE_OFF));
        // A greyed binding is not rebound or cleared.
        app.world_mut().resource_mut::<MovementInertia>().advanced = true;
        app.world_mut().resource_mut::<MovementInertia>().sprint = false;
        crate::bindings::rebind(app.world_mut(), Control::Sprint, crate::bindings::Input::Key(KeyCode::KeyK));
        assert_eq!(app.world().resource::<Bindings>().get(Control::Sprint).key, Some(KeyCode::ShiftLeft));
        crate::bindings::clear_key(app.world_mut(), Control::Sprint);
        assert_eq!(app.world().resource::<Bindings>().get(Control::Sprint).key, Some(KeyCode::ShiftLeft));
        app.world_mut().resource_mut::<MovementInertia>().sprint = true;
        crate::bindings::rebind(app.world_mut(), Control::Sprint, crate::bindings::Input::Key(KeyCode::KeyK));
        assert_eq!(app.world().resource::<Bindings>().get(Control::Sprint).key, Some(KeyCode::KeyK), "Sprint is on: it rebinds");
    }
}
