//! Coordinator boundary only. No copied ZIP query, admission or family priority.
use skate_core::physics::grind_contact::{Primitive, families::Contact};

/// The investigator owns admission, state priority and fallback proximity.
/// Publish this only for an admitted physical contact; a proximity-only result
/// stays with the manager and must not be converted into an active grind here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AcquiredContact {
    /// Includes the manager's seven-valued entry_kind, independently of kind.
    pub contact: Contact,
    /// Exact winning primitive endpoints/owner, not fabricated truck endpoints.
    pub primitive: Primitive,
}
