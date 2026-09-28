//! Original S3 grab-scene records and execution, separate from pending queries.
//! Ground owns request readiness, validation, selection and binding.
mod math;
mod qualify;
mod query;
pub mod record;
pub mod registry;
use super::{
    ground_query::QueryContext,
    ground_sync::{BoardLimits, Bounds},
};
pub use qualify::{best_spline, closest_point, qualify};
pub use query::query;
pub use record::{AssemblyData, Geometry, RecordInput};
pub use registry::{Object, Provider, Registry, Spline};
pub type Vector = [f32; 4];
pub type Frame = [Vector; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub kind: u32,
    pub id: u32,
}

/// Lossless native288-byte value. Word196 is a geometry identity, not readiness.
#[derive(Clone, Debug, PartialEq)]
pub struct Record(pub [u32; 72], pub std::sync::Arc<Geometry>);
impl Record {
    pub fn descriptor(&self) -> Descriptor {
        Descriptor {
            kind: self.0[47],
            id: self.0[48],
        }
    }
    pub fn endpoints(&self) -> [Vector; 2] {
        [self.vector(64), self.vector(80)]
    }
    pub fn assembly(&self) -> Option<u32> {
        (self.0[52] != 0).then_some(self.0[52])
    }
    pub fn valid(&self) -> bool {
        self.0[49] != 0
    }
    pub fn points(&self) -> &[Vector] {
        &self.1.points
    }
    pub fn vector(&self, offset: usize) -> Vector {
        std::array::from_fn(|i| f32::from_bits(self.0[offset / 4 + i]))
    }
    pub fn scalar(&self, offset: usize) -> f32 {
        f32::from_bits(self.0[offset / 4])
    }
    pub fn byte(&self, offset: usize) -> u8 {
        self.0[offset / 4].to_be_bytes()[offset % 4]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Query {
    pub position: Vector,
    ///82760508 sorts at+96, distinct from provider position+80.
    pub sort_position: Vector,
    pub bounds: Bounds,
    pub limits: BoardLimits,
    pub mode: u32,
    pub capacity: usize,
    pub context: QueryContext,
}

#[derive(Clone, Copy, Debug)]
pub struct Line {
    pub start: Vector,
    pub end: Vector,
    pub radius: f32,
    pub group: i32,
    pub reject_flags: u32,
    ///Batch source pools: validation82D744F8 uses7; interactable8275FD00 uses2.
    pub source_pool_mask: u8,
    ///Actor virtual56: NOT a collision-mesh rejection mask.
    pub selection_flags: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub fraction: f32,
    pub assembly: Option<u32>,
}
