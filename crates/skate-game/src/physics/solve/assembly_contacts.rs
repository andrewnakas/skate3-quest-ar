//! Original single-player SkaterSkaterCollisionPipeline8276556C.
//! Board construction precedes Skeleton construction in82DB18A0. Preserve
//! that assembly order: board self (all culled), board/rider, rider self.
use skate_core::physics::{
    board_step::{BoardCollision, CollisionBody},
    board_world::BoardWorldVolume,
    contact::{RetailContactInput, combine_contact_materials},
    skeleton_body::SkeletonCollisionMode,
    world_contact::{PrimitivePairSettings, primitive_pair_contacts},
};

pub(super) fn append(
    contacts: &mut Vec<BoardCollision>,
    board: &[BoardWorldVolume],
    rider: &[BoardWorldVolume],
    board_group: u32,
    collision: &SkeletonCollisionMode,
) -> Result<(), String> {
    //8277AF90 first culls the assembly pair;8277A6C0 then culls each
    //primitive pair using its part group, not Volume84's world-query group.
    if board_group >= 21
        || collision.assembly_group >= 21
        || collision.parts.iter().any(|part| part.part_group >= 21)
    {
        return Err("Skater collision group is outside the original21x21 table".into());
    }
    if group_pair_allowed(board_group, collision.assembly_group) {
        for a in board {
            for b in rider {
                let CollisionBody::Attached(part) = b.body else {
                    unreachable!()
                };
                //82DC3BD0/82DC4260 copy Part92 into VolumeData212
                //(assembly96 + volume212 = assembly308). Skate2's
                //82E0B690 names this producer part.m_userData. Biped assigns
                //hands5, legs17, torso18 and controller capsules20.
                if !group_pair_allowed(board_group, collision.parts[part].part_group) {
                    continue;
                }
                append_pair(contacts, a, b);
            }
        }
    }
    //8277AE80 suppresses the reversed assembly pair. The same-assembly
    //branch8277B2C0 instead visits both directed primitive pairs and uses
    //the skeleton's part bitmap. Board82C0B560 culls all7x7 self pairs.
    for a in rider {
        let CollisionBody::Attached(part_a) = a.body else {
            unreachable!()
        };
        for b in rider {
            let CollisionBody::Attached(part_b) = b.body else {
                unreachable!()
            };
            if part_a != part_b && !collision.self_culling[part_a][part_b] {
                append_pair(contacts, a, b);
            }
        }
    }
    Ok(())
}

fn group_pair_allowed(a: u32, b: u32) -> bool {
    //82765EF0..827679AC, Island.flags==3: inverted native culling bits.
    //Skate2 827B8C10 has19 groups and different pairs; do not import it.
    const ALLOWED: [u32; 21] = [
        0x1876fd, 0, 0x120001, 1, 0x79d1, 0x1079e1, 0x1079f1, 0x279f1, 0x1676f0, 0x7101, 0x5101,
        0x1008f0, 0x1d77f1, 0x233f1, 0xa57f1, 0, 0x1000, 0x6184, 0x1100, 0x5001, 0x1965,
    ];
    b < 21
        && ALLOWED
            .get(a as usize)
            .is_some_and(|row| row & (1 << b) != 0)
}

#[cfg(test)]
#[path = "assembly_contacts_tests.rs"]
mod tests;

pub(super) fn append_pair(
    contacts: &mut Vec<BoardCollision>,
    a: &BoardWorldVolume,
    b: &BoardWorldVolume,
) {
    //8277B0F0 and8277B2C0 enter the same8277A508 primitive walker with
    //identical padding/triangle settings. Keep A/B orientation and point order.
    let Some(manifold) = primitive_pair_contacts(
        a.primitive,
        b.primitive,
        PrimitivePairSettings::skater_self_collision(),
    ) else {
        return;
    };
    let material = combine_contact_materials(a.material, b.material);
    for points in &manifold.points[..manifold.count] {
        contacts.push(BoardCollision {
            body_a: a.body,
            body_b: b.body,
            contact: RetailContactInput {
                position_on_a: points.a,
                position_on_b: points.b,
                normal: manifold.normal,
                restitution: material.restitution,
                static_friction: material.static_friction,
                dynamic_friction: material.dynamic_friction,
                //8277A7B4/B8 packs original Volume88 tags, both zero for
                //stock skater/board constructors (not the part indices).
                tag: 0,
            },
        });
    }
}
