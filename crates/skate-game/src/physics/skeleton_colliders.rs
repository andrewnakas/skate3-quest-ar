//! Bind original physical skater shapes to the bodies receiving solver reactions.
//! The collision owner selects enabled parts and materials; shapes and local-Z
//! capsule axes come from the same stock definition used to construct each part.
use skate_core::{
    math::{Basis3, Vector3},
    physics::{
        board_step::CollisionBody,
        board_world::BoardWorldVolume,
        collision::Sphere,
        mass::MassShape,
        skeleton_body::{SkeletonBody, SkeletonCollisionMode},
        world_contact::ContactPrimitive,
    },
};

pub(super) fn world_volumes(
    skeleton: &SkeletonBody,
    collision: &SkeletonCollisionMode,
) -> Result<Vec<BoardWorldVolume>, String> {
    let mut volumes = enabled_volumes(skeleton, collision)?;
    retain_world_volumes(&mut volumes, collision);
    Ok(volumes)
}

/// Volume+84 group4 disables world contacts only. Keep these shapes available
/// to the separate SkaterSkaterCollisionPipeline and its own pair filters.
pub(super) fn retain_world_volumes(
    volumes: &mut Vec<BoardWorldVolume>,
    collision: &SkeletonCollisionMode,
) {
    volumes.retain(|volume| match volume.body {
        CollisionBody::Attached(part) => collision.parts[part].volume_group != 4,
        _ => unreachable!("skeleton volumes must reference attached bodies"),
    });
}

pub(super) fn enabled_volumes(
    skeleton: &SkeletonBody,
    collision: &SkeletonCollisionMode,
) -> Result<Vec<BoardWorldVolume>, String> {
    let transforms = skeleton.part_transforms();
    let mut volumes = Vec::new();
    for (index, (part, state)) in skeleton
        .definition
        .parts
        .iter()
        .zip(&collision.parts)
        .enumerate()
    {
        if !state.enabled {
            continue;
        }
        if !matches!(state.volume_group, 0 | 4) {
            return Err(format!(
                "Skater part{index} requires volume group{} world filtering",
                state.volume_group
            ));
        }
        if part.hat.is_some() {
            return Err("The equipped hat requires its original cylinder collision query".into());
        }
        let frame = transforms[index];
        let xyz = |value: [f32; 4]| Vector3::new(value[0], value[1], value[2]);
        let center = xyz(frame[3]);
        let primitive = match part.shape {
            MassShape::Sphere { radius } => ContactPrimitive::Sphere(Sphere { center, radius }),
            MassShape::Capsule {
                radius,
                half_length,
            } => ContactPrimitive::Capsule {
                center,
                axis: xyz(frame[2]),
                half_length,
                radius,
            },
            MassShape::RoundedBox {
                half_extents,
                radius,
            } => ContactPrimitive::RoundedBox {
                center,
                basis: Basis3 {
                    columns: std::array::from_fn(|i| [frame[i][0], frame[i][1], frame[i][2]]),
                },
                half_extents,
                radius,
            },
            shape => {
                return Err(format!(
                    "Skater part{index} has no recovered collision query for {shape:?}"
                ));
            }
        };
        volumes.push(BoardWorldVolume {
            body: CollisionBody::Attached(index),
            primitive,
            linear_velocity: skeleton.bodies()[index].rates.linear_velocity,
            material: state.material,
        });
    }
    Ok(volumes)
}
