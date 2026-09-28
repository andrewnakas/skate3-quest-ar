//! Contacts for every board primitive against the host triangle world.
//! Rust owns geometry storage and traversal. Recovered query8277B720,
//! triangle fixup82AD3130, material combine82763078 and contact retention
//! determine the physical result, in world-triangle then moving-volume order.
mod broadphase;
mod query_index;
pub mod query_metadata;
use crate::math::Vector3;
use broadphase::{conservative_bounds, primitive_bounds};
use query_metadata::{Bounds, QueryMetadata};

use super::{
    board::BodyId,
    board_runtime::BoardRuntime,
    board_step::{BoardCollision, CollisionBody},
    collision::{Sphere, Triangle, WorldContactSettings},
    contact::{RetailContactInput, RetailContactMaterial, combine_contact_materials},
    triangle_query::{TriangleLineHit, triangle_segment},
    world_contact::{
        ContactBuffer, ContactPrimitive, ContactRecord, primitive_triangle_world_contacts,
        triangle_from_volume,
    },
};

#[derive(Clone, Copy, Debug)]
pub struct WorldTriangle {
    pub triangle: Triangle,
    pub material: RetailContactMaterial,
    pub tag: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldLineHit {
    pub geometry: TriangleLineHit,
    pub tag: u32,
}

impl WorldTriangle {
    /// Host geometry preparation, with explicit adjacency and sidedness.
    /// Uses the native Volume constructor and its authored winding. Invalid
    /// host geometry is rejected without substituting a repaired triangle.
    pub fn from_vertices(
        vertices: [Vector3; 3],
        material: RetailContactMaterial,
        tag: u32,
        flags: u32,
        edge_cosines: [f32; 3],
        fatness: f32,
    ) -> Option<Self> {
        if vertices
            .iter()
            .any(|v| !v.x.is_finite() || !v.y.is_finite() || !v.z.is_finite())
        {
            return None;
        }
        let triangle = triangle_from_volume(vertices, fatness, edge_cosines, flags);
        let normal = triangle.feature.normal;
        if triangle
            .edge_lengths
            .iter()
            .any(|&v| !v.is_finite() || v <= 0.0)
            || !normal.x.is_finite()
            || !normal.y.is_finite()
            || !normal.z.is_finite()
            || normal == Vector3::ZERO
        {
            return None;
        }
        Some(Self {
            triangle,
            material,
            tag,
        })
    }
}

/// Caller-resolved TempContactBuffer settings. Retention changes solver rows,
/// so its thresholds/capacity remain explicit instead of using tuned defaults.
#[derive(Clone, Copy, Debug)]
pub struct ContactRetentionSettings {
    pub capacity: u32,
    pub duplicate_distance_squared: f32,
    pub deferred_reduction: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WheelWorldSettings {
    /// Sphere shared by wheel definitions in82C0AA78.
    pub radius: f32,
    pub material: RetailContactMaterial,
    pub query: WorldContactSettings,
    pub retention: ContactRetentionSettings,
}

/// One enabled moving primitive, with its owning solver body and material.
#[derive(Clone, Copy, Debug)]
pub struct BoardWorldVolume {
    pub body: CollisionBody,
    pub primitive: ContactPrimitive,
    pub linear_velocity: Vector3,
    pub material: RetailContactMaterial,
}

/// World geometry remains in supplied order. Each query reuses output storage;
/// the returned contacts are valid until the next mutable call.
pub struct BoardWorld {
    triangles: Vec<WorldTriangle>,
    triangle_bounds: Vec<Bounds>,
    query_metadata: Option<QueryMetadata>,
    query_index: query_index::QueryIndex,
    maximum_fatness: f32,
    maximum_triangle_margin: f32,
    contacts: Vec<BoardCollision>,
    buffer: ContactBuffer,
}

impl BoardWorld {
    pub fn new(triangles: Vec<WorldTriangle>) -> Self {
        let triangle_bounds: Vec<_> = triangles
            .iter()
            .map(|t| {
                Bounds::from_points(t.triangle.vertices).unwrap_or(Bounds {
                    min: Vector3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY),
                    max: Vector3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY),
                })
            })
            .collect();
        let maximum_fatness = triangles
            .iter()
            .map(|t| {
                if t.triangle.fatness.is_finite() && t.triangle.fatness >= 0. {
                    t.triangle.fatness
                } else {
                    f32::INFINITY
                }
            })
            .fold(0., f32::max);
        // The current thin-triangle leaf accepts barycentric weights down to
        // -epsilon. Two negative weights can extend an axis by 2*epsilon*span.
        let maximum_triangle_margin = triangle_bounds
            .iter()
            .map(|b| {
                let span = (b.max.x - b.min.x)
                    .max(b.max.y - b.min.y)
                    .max(b.max.z - b.min.z);
                span * (2. * broadphase::THIN_MARGIN)
            })
            .fold(0., f32::max);
        Self {
            triangles,
            triangle_bounds,
            query_metadata: None,
            query_index: query_index::QueryIndex::default(),
            maximum_fatness,
            maximum_triangle_margin,
            contacts: Vec::new(),
            buffer: ContactBuffer {
                count: 0,
                flushed: 0,
                capacity: 0,
                dropped: 0,
                distance_squared_threshold: 0.0,
                allow_flush: 1,
                deferred_reduction: 0,
                full: 0,
                records: [[0; 64]; 50],
            },
        }
    }

    pub fn with_query_metadata(
        triangles: Vec<WorldTriangle>,
        metadata: QueryMetadata,
    ) -> Result<Self, &'static str> {
        metadata.validate(&triangles)?;
        let mut world = Self::new(triangles);
        world.query_index = query_index::QueryIndex::new(&metadata.meshes);
        world.query_metadata = Some(metadata);
        Ok(world)
    }

    pub fn query_metadata(&self) -> Result<&QueryMetadata, &'static str> {
        self.query_metadata
            .as_ref()
            .ok_or("Canonical world has no authored query metadata")
    }

    pub fn triangles(&self) -> &[WorldTriangle] {
        &self.triangles
    }

    /// Board wheel segments use zero query radius; rounded world triangles
    /// still select the native swept branch through their own fatness.
    pub fn query_thin_line(
        &self,
        start: Vector3,
        end: Vector3,
    ) -> Result<Option<WorldLineHit>, &'static str> {
        self.query_swept_line(start, end, 0.0)
    }

    /// Ordered nearest-hit traversal for wheel and camera queries. The host
    /// owns traversal; each candidate uses the complete TU3 triangle dispatcher.
    pub fn query_swept_line(
        &self,
        start: Vector3,
        end: Vector3,
        radius: f32,
    ) -> Result<Option<WorldLineHit>, &'static str> {
        if !radius.is_finite() || radius < 0.0 {
            return Err("line query radius must be finite and nonnegative");
        }
        let direction = Vector3::new(end.x - start.x, end.y - start.y, end.z - start.z);
        let mut nearest: Option<WorldLineHit> = None;
        for (_, entry) in self.line_candidates(start, end, radius) {
            let mut geometry = TriangleLineHit {
                position: Vector3::ZERO,
                normal: Vector3::ZERO,
                fraction: 0.0,
                volume_parameter: [0.0; 3],
            };
            if triangle_segment(
                &mut geometry,
                start,
                direction,
                entry.triangle.vertices,
                radius,
                entry.triangle.fatness,
            ) {
                if nearest
                    .as_ref()
                    .is_none_or(|hit| geometry.fraction < hit.geometry.fraction)
                {
                    nearest = Some(WorldLineHit {
                        geometry,
                        tag: entry.tag,
                    });
                }
            }
        }
        Ok(nearest)
    }

    /// Sphere-only convenience for callers querying wheels. The game uses
    /// query_primitives with all enabled deck/truck/wheel children.
    pub fn query(
        &mut self,
        board: &BoardRuntime,
        settings: WheelWorldSettings,
    ) -> &[BoardCollision] {
        let poses = board.part_transforms();
        let volumes: Vec<_> = BodyId::ORDER[..4]
            .iter()
            .filter_map(|&id| {
                let body = &board.bodies()[id.index()];
                (body.state_flags != 1).then_some(BoardWorldVolume {
                    body: CollisionBody::Board(id),
                    primitive: ContactPrimitive::Sphere(Sphere {
                        center: poses[id.index()].translation,
                        radius: settings.radius,
                    }),
                    linear_velocity: body.rates.linear_velocity,
                    material: settings.material,
                })
            })
            .collect();
        self.query_primitives(&volumes, settings.query, settings.retention)
    }

    /// World-space shapes come from the live part poses and authored children.
    /// Cluster bounds include shape radii and the maximum predictive padding.
    /// Candidate pairs retain world-triangle then moving-volume order.
    pub fn query_primitives(
        &mut self,
        volumes: &[BoardWorldVolume],
        query: WorldContactSettings,
        retention: ContactRetentionSettings,
    ) -> &[BoardCollision] {
        self.contacts.clear();
        self.buffer.count = 0;
        self.buffer.flushed = 0;
        self.buffer.dropped = 0;
        self.buffer.full = 0;
        self.buffer.capacity = retention.capacity;
        self.buffer.distance_squared_threshold = retention.duplicate_distance_squared;
        self.buffer.deferred_reduction = u8::from(retention.deferred_reduction);
        let padding =
            if query.volume_padding.is_finite() && query.maximum_separating_distance.is_finite() {
                query.volume_padding.max(0.) + query.maximum_separating_distance.max(0.)
            } else {
                f32::INFINITY
            };
        let volume_bounds: Vec<_> = volumes
            .iter()
            .map(|v| {
                primitive_bounds(v.primitive)
                    .map(|b| conservative_bounds(b, padding + self.maximum_fatness))
            })
            .collect();
        let bounds: Option<Vec<_>> = volume_bounds.iter().copied().collect();
        let bounds = bounds
            .and_then(|b| Bounds::from_points(b.iter().flat_map(|b| [b.min, b.max])))
            .map(|b| b.expanded(padding));
        let ranges = self.candidate_ranges(bounds);
        let output = &mut self.contacts;
        let mut publish = |records: &[ContactRecord]| {
            output.extend(records.iter().map(collision_from_record));
        };
        for index in ranges.into_iter().flatten() {
            let entry = &self.triangles[index];
            for (volume, volume_bounds) in volumes.iter().zip(&volume_bounds) {
                if self.query_metadata.is_some()
                    && volume_bounds.is_some_and(|b| !self.triangle_bounds[index].overlaps(b))
                {
                    continue;
                }
                let Some(manifold) = primitive_triangle_world_contacts(
                    volume.primitive,
                    entry.triangle,
                    volume.linear_velocity,
                    query,
                ) else {
                    continue;
                };
                let material = combine_contact_materials(volume.material, entry.material);
                for pair in &manifold.points[..manifold.count] {
                    let contact = RetailContactInput {
                        position_on_a: pair.a,
                        position_on_b: pair.b,
                        normal: manifold.normal,
                        restitution: material.restitution,
                        static_friction: material.static_friction,
                        dynamic_friction: material.dynamic_friction,
                        tag: entry.tag,
                    };
                    let Some(slot) = self.buffer.allocate(&mut publish) else {
                        self.buffer.flush(&mut publish);
                        return &self.contacts;
                    };
                    self.buffer.records[slot] = retention_record(volume.body, contact);
                    //8277C23C removes the most recent record on rejection.
                    if self.buffer.last_is_duplicate() {
                        self.buffer.count -= 1;
                    }
                }
            }
        }
        self.buffer.flush(&mut publish);
        &self.contacts
    }

    pub fn contacts(&self) -> &[BoardCollision] {
        &self.contacts
    }

    pub fn dropped_contacts(&self) -> u32 {
        self.buffer.dropped
    }
}

/// Retention reads only contact header geometry, body IDs and material/tag.
/// BoardStep builds the actual body workspaces after this frame's force queue
/// has been applied, preventing stale acceleration copies in solver contacts.
fn retention_record(id: CollisionBody, contact: RetailContactInput) -> ContactRecord {
    let mut record = [0; 64];
    for (offset, v) in [
        (0, contact.position_on_a),
        (4, contact.position_on_b),
        (8, contact.normal),
    ] {
        record[offset..offset + 3].copy_from_slice(&[v.x, v.y, v.z].map(f32::to_bits));
    }
    record[3] = id.contact_id();
    record[7] = u32::MAX;
    record[11] = contact.restitution.to_bits();
    record[15] = contact.static_friction.to_bits();
    record[19] = contact.dynamic_friction.to_bits();
    record[23] = contact.tag;
    record
}

fn collision_from_record(record: &ContactRecord) -> BoardCollision {
    let f = |i| f32::from_bits(record[i]);
    let vector = |i| Vector3::new(f(i), f(i + 1), f(i + 2));
    BoardCollision {
        body_a: CollisionBody::from_contact_id(record[3]),
        body_b: CollisionBody::StaticWorld,
        contact: RetailContactInput {
            position_on_a: vector(0),
            position_on_b: vector(4),
            normal: vector(8),
            restitution: f(11),
            static_friction: f(15),
            dynamic_friction: f(19),
            tag: record[23],
        },
    }
}

#[cfg(test)]
#[path = "tests/board_world.rs"]
mod tests;

#[cfg(test)]
#[path = "board_world/tests.rs"]
mod broadphase_tests;
