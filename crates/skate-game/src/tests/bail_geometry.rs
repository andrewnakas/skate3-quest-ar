//! Test-only measurements, not solver limits or substitute physical constraints.
use crate::physics::{GamePhysics, SkaterRuntime};
use skate_core::physics::{assembly::BodySnapshot, joint_records::default_joint_records};

#[derive(Default, Debug)]
pub(super) struct Geometry {
    pub board_anchor_peak: (f64, usize, usize),
    pub leg_anchor_peak: (f64, usize, usize),
    pub basis_gram_peak: (f64, usize),
}

impl Geometry {
    pub fn observe(&mut self, physics: &GamePhysics, skater: &SkaterRuntime, tick: usize) {
        for (i, joint) in default_joint_records().iter().enumerate() {
            let distance = separation(
                &physics.board.bodies()[joint.live_body_a().index()],
                &physics.board.bodies()[joint.live_body_b().index()],
                &joint.frames.words,
            );
            if distance > self.board_anchor_peak.0 {
                self.board_anchor_peak = (distance, tick, i);
            }
        }
        for joint in &skater.skeleton_joints.records {
            if !(15..=22).contains(&joint.child) {
                continue;
            }
            let distance = separation(
                &skater.skeleton.bodies()[joint.child],
                &skater.skeleton.bodies()[joint.parent],
                &joint.frames.words,
            );
            if distance > self.leg_anchor_peak.0 {
                self.leg_anchor_peak = (distance, tick, joint.child);
            }
        }
        for body in physics
            .board
            .bodies()
            .iter()
            .chain(skater.skeleton.bodies())
        {
            let columns = body.rates.basis.columns;
            for i in 0..3 {
                for j in 0..3 {
                    let dot: f64 = (0..3)
                        .map(|k| columns[i][k] as f64 * columns[j][k] as f64)
                        .sum();
                    let error = (dot - if i == j { 1.0 } else { 0.0 }).abs();
                    if error > self.basis_gram_peak.0 {
                        self.basis_gram_peak = (error, tick);
                    }
                }
            }
        }
    }
}

fn separation(a: &BodySnapshot, b: &BodySnapshot, frames: &[u32; 20]) -> f64 {
    let point = |body: &BodySnapshot, offset: usize| -> [f64; 3] {
        let p = body.rates.position;
        let translation = [p.x, p.y, p.z];
        core::array::from_fn(|i| {
            translation[i] as f64
                + (0..3)
                    .map(|j| {
                        body.rates.basis.columns[j][i] as f64
                            * f32::from_bits(frames[offset + j]) as f64
                    })
                    .sum::<f64>()
        })
    };
    let pa = point(a, 4);
    let pb = point(b, 12);
    (0..3).map(|i| (pb[i] - pa[i]).powi(2)).sum::<f64>().sqrt()
}
