//! Stateless score and bump lifecycles using the same MotionHost publication.
use super::*;
impl MotionHost {
    pub(super) fn execute_score_or_bump(
        &mut self,
        operation: &MotionOperation,
        phase: u8,
    ) -> Result<bool, String> {
        match operation {
            MotionOperation::ScoringTrick(operation) => {
                operation.execute(
                    &mut self.score_packet.trick_names,
                    &mut self.score_packet.flags,
                    phase,
                );
                // Endless Tricks: an extra rung re-enters the authored cycle state, so it
                // republishes the authored `<Trick>3`/`<Trick>4`. Rewrite those to the rung
                // actually being flown. `published_name` returns None on every retail path --
                // and for every other trick even while a ladder is running -- so this changes
                // nothing unless a mod turned the feature on.
                if phase == 1 {
                    if let Some(name) = self
                        .score_packet
                        .trick_names
                        .first
                        .and_then(|published| self.endless.published_name(published))
                    {
                        self.score_packet.trick_names.first = Some(name);
                        self.score_packet.trick_names.second = Some(name);
                    }
                }
            }
            MotionOperation::SetBumpCoefficients { x, y } => {
                if phase == 0 {
                    let values = self.bump_settings.coefficients(
                        self.bump_acceleration
                            .ok_or("SetBumpCoefficients requires completed acceleration")?,
                        self.animation
                            .skater_animation_flags
                            .ok_or("SetBumpCoefficients requires actual stance")?
                            & 0x40000000
                            != 0,
                    );
                    for (name, value) in [*x, *y].into_iter().zip(values) {
                        self.animation.set_attribute(SettableAttribute {
                            name,
                            value,
                            normalized: false,
                            sequence_id: -1,
                        });
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
