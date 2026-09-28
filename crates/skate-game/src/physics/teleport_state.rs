//! Host checkpoint response to original actor reset callback82592518.
//! Shared scheduling preserves request -> next input reply ->702 output -> reset.
use skate_core::{
    animation::output::actor_packet::ExternalReset,
    physics::skeleton_animation_record::AnimationPartTransform,
    player::{
        input_phase::{PhysicalPlayerInput, ProcessedPhysicsInput},
        teleport_state::{Output, Target, TeleportState, Update},
    },
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Checkpoint {
    pub transform: AnimationPartTransform,
    /// Original actor82592518 inverts checkpoint byte68. Its ground validator
    ///82BFBC18 sets that byte for surface category8. Authored rideable spawn
    ///surfaces supply true; an off-board checkpoint must explicitly supply false.
    pub on_board: bool,
}

pub(crate) struct Runtime {
    state: TeleportState,
    checkpoint: Checkpoint,
    pending_reply: Option<Target>,
    manual_on_board: Option<bool>,
    vehicle_ejection: Option<([f32; 3], [f32; 3])>,
}
impl Runtime {
    /// Explicit manual return uses the actor-reset publication without replacing
    /// the automatic recovery checkpoint owned by this runtime.
    pub fn request_manual(&mut self, transform: AnimationPartTransform, on_board: bool) {
        self.vehicle_ejection = None;
        self.pending_reply = Some(Target {
            transform: transform.map(|r| r.map(f32::to_bits)),
            on_board,
        });
        self.manual_on_board = Some(on_board);
    }
    pub fn request_vehicle_ejection(
        &mut self,
        transform: AnimationPartTransform,
        velocity: [f32; 3],
        angular: [f32; 3],
    ) {
        self.request_manual(transform, false);
        self.vehicle_ejection = Some((velocity, angular));
    }
    pub fn take_vehicle_ejection(&mut self) -> Option<([f32; 3], [f32; 3])> {
        self.vehicle_ejection.take()
    }
    pub fn take_manual_on_board(&mut self) -> Option<bool> {
        self.manual_on_board.take()
    }
    pub(super) fn set_checkpoint(&mut self, checkpoint: Checkpoint) {
        self.checkpoint = checkpoint;
        self.pending_reply = None;
    }
    pub fn new(checkpoint: Checkpoint) -> Self {
        Self {
            state: TeleportState::default(),
            checkpoint,
            pending_reply: None,
            manual_on_board: None,
            vehicle_ejection: None,
        }
    }
    pub fn enter(&mut self) {
        self.state.enter();
    }
    /// Update82D431F0 calls the actor only without Processed2468 bit1.
    ///825926F8 stores the reply and marks it for the next input publication.
    pub fn update(&mut self, input: &ProcessedPhysicsInput) -> bool {
        self.state
            .update(input.flags_2468, input.matrix_1536, input.byte_1600)
            == Update::RequestCheckpoint
    }
    /// Host response for an independently verified actor-reset request.
    pub fn request_checkpoint(&mut self) {
        self.reply(self.checkpoint);
    }
    pub fn reply(&mut self, checkpoint: Checkpoint) {
        self.pending_reply = Some(Target {
            transform: checkpoint.transform.map(|row| row.map(f32::to_bits)),
            on_board: checkpoint.on_board,
        });
    }
    /// Consume at the next actor-reset publication boundary. Publish this as
    ///ExternalReset plus Actor1904 bit29, then use the ordinary animation-packet
    ///mapper to produce1536/1600 and2468 bit1. Never reset physical bodies here.
    pub fn take_reply(&mut self) -> Option<ExternalReset> {
        self.pending_reply.take().map(|reply| ExternalReset {
            transform: reply.transform,
            byte64: u8::from(reply.on_board),
        })
    }
    /// Call after ordinary physical publication so702 owns State8/61 and
    ///the returned board0 transform/board272 publication for this tick.
    pub fn publish_output(&self, physical: &mut PhysicalPlayerInput) -> Option<Output> {
        let output = self.state.output()?;
        physical.teleport_output = Some(output);
        physical.state.identifier_8 = output.next_state;
        physical.state.flag_61 = output.state_61;
        Some(output)
    }
}
#[cfg(test)]
mod vehicle_tests {
    use super::*;
    fn matrix() -> AnimationPartTransform {
        [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [3., 4., 5., 0.],
        ]
    }
    #[test]
    fn vehicle_momentum_survives_reset_reply_and_is_consumed_once() {
        let mut state = Runtime::new(Checkpoint {
            transform: matrix(),
            on_board: true,
        });
        state.request_vehicle_ejection(matrix(), [12., 2., -3.], [0., 1., 0.]);
        assert!(state.take_reply().is_some());
        assert_eq!(state.take_manual_on_board(), Some(false));
        assert_eq!(
            state.take_vehicle_ejection(),
            Some(([12., 2., -3.], [0., 1., 0.]))
        );
        assert_eq!(state.take_vehicle_ejection(), None);
        state.request_vehicle_ejection(matrix(), [12., 2., -3.], [0., 1., 0.]);
        state.request_manual(matrix(), true);
        assert_eq!(state.take_vehicle_ejection(), None);
    }
}
