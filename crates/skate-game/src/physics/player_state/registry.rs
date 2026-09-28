//! Coordinator-owned physical state registry.
//!
//! TU3 has one lifecycle object per PhysicalPlayer state. Keeping the
//! support table here makes selection, transition, and diagnostics agree on
//! which owners are actually connected. Task-B-owned states remain explicit
//! unsupported entries until their native adapters are integrated.

use skate_core::player::state::PhysicalStateId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StateCapability {
    pub id: PhysicalStateId,
    pub supported: bool,
    pub has_enter: bool,
    pub has_exit: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StateRegistry;

impl StateRegistry {
    pub(crate) const fn new() -> Self {
        Self
    }

    pub(crate) fn capability(self, id: PhysicalStateId) -> StateCapability {
        let supported = id.is_grind()
            || id == PhysicalStateId::Nonspecific
            || matches!(
                id,
                PhysicalStateId::Sleeping
                    | PhysicalStateId::PhysicsGround
                    | PhysicalStateId::PhysicsAir
                    | PhysicalStateId::PhysicsAirSecondary
                    | PhysicalStateId::FootPlant
                    | PhysicalStateId::Boneless
                    | PhysicalStateId::HandPlant
                    | PhysicalStateId::RevertGround
                    | PhysicalStateId::KnownAir
                    | PhysicalStateId::BipedAir
                    | PhysicalStateId::BipedGround
                    | PhysicalStateId::OffBoardPushing
                    | PhysicalStateId::GroundAnimation
                    | PhysicalStateId::SlideGround
                    | PhysicalStateId::WipeoutGround
                    | PhysicalStateId::Teleporting
                    | PhysicalStateId::LandingOnDeck
            );
        StateCapability {
            id,
            supported,
            has_enter: supported && !matches!(id, PhysicalStateId::Sleeping),
            has_exit: supported,
        }
    }

    pub(crate) fn can_transition(
        self,
        current: PhysicalStateId,
        requested: PhysicalStateId,
    ) -> bool {
        if !self.capability(current).supported || !self.capability(requested).supported {
            return false;
        }
        if current != PhysicalStateId::Sleeping
            && (current.is_grind()
                || requested.is_grind()
                || current == PhysicalStateId::Nonspecific
                || requested == PhysicalStateId::Nonspecific)
        {
            return true;
        }
        matches!(
            (current, requested),
            (PhysicalStateId::Sleeping, PhysicalStateId::PhysicsGround)
                | (
                    PhysicalStateId::PhysicsGround
                        | PhysicalStateId::PhysicsAir
                        | PhysicalStateId::PhysicsAirSecondary
                        | PhysicalStateId::FootPlant
                        | PhysicalStateId::Boneless
                        | PhysicalStateId::HandPlant
                        | PhysicalStateId::RevertGround
                        | PhysicalStateId::KnownAir
                        | PhysicalStateId::BipedAir
                        | PhysicalStateId::BipedGround
                        | PhysicalStateId::OffBoardPushing
                        | PhysicalStateId::GroundAnimation
                        | PhysicalStateId::SlideGround
                        | PhysicalStateId::WipeoutGround
                        | PhysicalStateId::Teleporting
                        | PhysicalStateId::LandingOnDeck,
                    PhysicalStateId::PhysicsGround
                        | PhysicalStateId::PhysicsAir
                        | PhysicalStateId::PhysicsAirSecondary
                        | PhysicalStateId::FootPlant
                        | PhysicalStateId::Boneless
                        | PhysicalStateId::HandPlant
                        | PhysicalStateId::RevertGround
                        | PhysicalStateId::KnownAir
                        | PhysicalStateId::BipedAir
                        | PhysicalStateId::BipedGround
                        | PhysicalStateId::OffBoardPushing
                        | PhysicalStateId::GroundAnimation
                        | PhysicalStateId::SlideGround
                        | PhysicalStateId::WipeoutGround
                        | PhysicalStateId::Teleporting
                        | PhysicalStateId::LandingOnDeck
                )
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_connected_biped_state_lifecycles() {
        let registry = StateRegistry::new();
        assert!(
            registry
                .capability(PhysicalStateId::PhysicsGround)
                .supported
        );
        assert!(
            registry.can_transition(PhysicalStateId::PhysicsGround, PhysicalStateId::PhysicsAir)
        );
        assert!(!registry.can_transition(PhysicalStateId::Sleeping, PhysicalStateId::PhysicsAir));
        assert!(registry.capability(PhysicalStateId::BipedAir).supported);
    }

    /// The states whose native adapters are still missing, and what each one costs.
    ///
    /// This list is deliberately exhaustive rather than a `!supported` filter: an unported
    /// state is only discovered when the selector asks for it mid-play, and `transition::set`
    /// turns that into a fatal error. Pinning the set here makes adding or removing one a
    /// reviewed edit instead of a crash somebody hits in a playtest.
    ///
    /// * `Skitching` (104) -- holding onto a vehicle.
    /// * `FollowPath` (105) -- scripted/living-world movement.
    #[test]
    fn the_unported_states_are_exactly_the_documented_two() {
        let registry = StateRegistry::new();
        let unported: Vec<PhysicalStateId> = PhysicalStateId::ALL
            .into_iter()
            .filter(|id| !registry.capability(*id).supported)
            .collect();
        assert_eq!(
            unported,
            vec![PhysicalStateId::Skitching, PhysicalStateId::FollowPath],
            "unported state set changed; update this list and docs/engine-defects.md"
        );
    }
}
