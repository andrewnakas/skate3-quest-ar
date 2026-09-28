//! ZIP world broadphase helpers; recovered narrow-phase stays in its existing modules.
use super::{BoardWorld, ContactPrimitive, Vector3, WorldTriangle, query_metadata::Bounds};

// Positive magnitude of thin_triangle's existing 0xb727_c5ac tolerance.
pub(super) const THIN_MARGIN: f32 = f32::from_bits(0x3727_c5ac);

impl BoardWorld {
    /// Conservative cluster culling, retaining canonical traversal order and
    /// every triangle in intersecting clusters. Narrow-phase remains unchanged.
    pub fn candidate_ranges(&self, bounds: Option<Bounds>) -> Vec<std::ops::Range<usize>> {
        let (Some(metadata), Some(bounds)) = (&self.query_metadata, bounds.filter(|b| b.valid()))
        else {
            return vec![0..self.triangles.len()];
        };
        let scale = [
            bounds.min.x,
            bounds.min.y,
            bounds.min.z,
            bounds.max.x,
            bounds.max.y,
            bounds.max.z,
        ]
        .into_iter()
        .map(f32::abs)
        .fold(1., f32::max);
        let bounds = bounds.expanded(
            self.maximum_fatness + self.maximum_triangle_margin + scale * (8. * f32::EPSILON),
        );
        self.query_index
            .query(bounds, &metadata.meshes)
            .into_iter()
            .map(|i| metadata.meshes[i].triangle_range.clone())
            .collect()
    }

    pub fn line_candidates(
        &self,
        start: Vector3,
        end: Vector3,
        radius: f32,
    ) -> impl Iterator<Item = (usize, &WorldTriangle)> {
        let bounds = self.line_candidate_bounds(start, end, radius);
        self.candidate_ranges(bounds)
            .into_iter()
            .flatten()
            .filter(move |&i| {
                self.query_metadata.is_none()
                    || bounds.is_none_or(|b| self.triangle_bounds[i].overlaps(b))
            })
            .map(|i| (i, &self.triangles[i]))
    }

    /// Bounds shared with adapters that retain mesh pool/group/identity filtering.
    /// Includes the current thin-leaf endpoint and barycentric margins.
    pub fn line_candidate_bounds(
        &self,
        start: Vector3,
        end: Vector3,
        radius: f32,
    ) -> Option<Bounds> {
        if !radius.is_finite() || radius < 0. {
            return None;
        }
        let span = (end.x - start.x)
            .abs()
            .max((end.y - start.y).abs())
            .max((end.z - start.z).abs());
        Bounds::from_points([start, end]).map(|b| {
            conservative_bounds(
                b,
                radius + self.maximum_fatness + self.maximum_triangle_margin + span * THIN_MARGIN,
            )
        })
    }

    /// ZIP hierarchy results in canonical mesh order, for current metadata consumers.
    /// Pool, matching group, rejection mask and identity stay caller-owned.
    pub fn candidate_mesh_indices(
        &self,
        bounds: Option<Bounds>,
    ) -> Result<Vec<usize>, &'static str> {
        let metadata = self.query_metadata()?;
        let Some(bounds) = bounds.filter(|b| b.valid()) else {
            return Ok((0..metadata.meshes.len()).collect());
        };
        Ok(self.query_index.query(
            conservative_bounds(bounds, self.maximum_fatness + self.maximum_triangle_margin),
            &metadata.meshes,
        ))
    }
}

pub(super) fn primitive_bounds(primitive: ContactPrimitive) -> Option<Bounds> {
    let (center, radius) = match primitive {
        ContactPrimitive::Sphere(s) => (s.center, s.radius),
        ContactPrimitive::Capsule {
            center,
            axis,
            half_length,
            radius,
        } => {
            if !radius.is_finite() {
                return None;
            }
            let offset = Vector3::new(
                axis.x * half_length,
                axis.y * half_length,
                axis.z * half_length,
            );
            return Bounds::from_points([
                Vector3::new(
                    center.x - offset.x,
                    center.y - offset.y,
                    center.z - offset.z,
                ),
                Vector3::new(
                    center.x + offset.x,
                    center.y + offset.y,
                    center.z + offset.z,
                ),
            ])
            .map(|b| b.expanded(radius.abs()));
        }
        ContactPrimitive::RoundedBox {
            center,
            basis,
            half_extents,
            radius,
        } => {
            if !radius.is_finite() {
                return None;
            }
            let half = [half_extents.x, half_extents.y, half_extents.z];
            let extent: [f32; 3] = std::array::from_fn(|axis| {
                radius.abs()
                    + basis
                        .columns
                        .iter()
                        .zip(half)
                        .map(|(column, h)| h.abs() * column[axis].abs())
                        .sum::<f32>()
            });
            return Bounds::from_points([
                Vector3::new(
                    center.x - extent[0],
                    center.y - extent[1],
                    center.z - extent[2],
                ),
                Vector3::new(
                    center.x + extent[0],
                    center.y + extent[1],
                    center.z + extent[2],
                ),
            ]);
        }
        ContactPrimitive::Triangle(t) => {
            if !t.fatness.is_finite() {
                return None;
            }
            return Bounds::from_points(t.vertices).map(|b| b.expanded(t.fatness.abs()));
        }
    };
    if !radius.is_finite() {
        return None;
    }
    Bounds::from_points([center]).map(|b| b.expanded(radius.abs()))
}

pub(super) fn conservative_bounds(bounds: Bounds, padding: f32) -> Bounds {
    let scale = [
        bounds.min.x,
        bounds.min.y,
        bounds.min.z,
        bounds.max.x,
        bounds.max.y,
        bounds.max.z,
    ]
    .into_iter()
    .map(f32::abs)
    .fold(1., f32::max);
    bounds.expanded(padding + scale * (8. * f32::EPSILON))
}
