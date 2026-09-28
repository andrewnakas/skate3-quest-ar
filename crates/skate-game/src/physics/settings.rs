//! Stock skater XML values bound to the recovered physical calculations.
use skate_core::{
    math::Vector3,
    physics::{
        board_step::BoardStepSettings,
        contact::RetailContactMaterial,
        drive_frames::{
            AuthoredTransformInputs, RetailAffineTransform, RetailTruckTransformInputs,
            authored_body_transforms, calculate_truck_transforms,
        },
        drive_parameters::{RetailTruckDriveSettings, retail_truck_drive_dynamics},
        mass::{
            DeckGeometry, DeckGeometrySettings, MassShape, TruckMassSettings, WheelMassSettings,
            deck_mass_properties, truck_mass_input, truck_mass_properties, wheel_mass_properties,
        },
        rigid_body::{RetailBodyMassProperties, RetailSimulationStep},
    },
};
use skate_data::collections::Collections;

pub(crate) struct PhysicsSettings {
    pub step: BoardStepSettings,
    pub masses: [RetailBodyMassProperties; 7],
    pub authored: [RetailAffineTransform; 7],
    pub deck_geometry: DeckGeometry,
    pub truck_shape: MassShape,
    pub truck_collisions: bool,
    pub truck_material: RetailContactMaterial,
    pub deck_material: RetailContactMaterial,
    pub wheel_radius: f32,
    pub wheel_material: RetailContactMaterial,
    pub standard_wheel_material: RetailContactMaterial,
    pub floor_material: RetailContactMaterial,
    pub input_magnitude_threshold: f32,
}

impl PhysicsSettings {
    pub fn load(data: &Collections) -> Result<Self, String> {
        let f = |class, field| data.float(class, "default", field);
        let b = |class, field| data.boolean(class, "default", field);
        let mass_factor = f("physics_world", "SkateboardMassFactor")?;
        let wheel_radius = f("physicswheels", "WheelRadius")?;
        let wheel_x_distance = f("physicswheels", "WheelXDist")?;
        let geometry = AuthoredTransformInputs {
            deck_mid_length: f("physicsdeck", "DeckMidLength")?,
            wheel_x_distance,
            truck_z_position_front: f("physicstrucks", "TruckZPosFront")?,
            truck_z_position_back: f("physicstrucks", "TruckZPosBack")?,
            truck_y_position: f("physicstrucks", "TruckYPos")?,
        };
        let wheel = wheel_mass_properties(WheelMassSettings {
            radius: wheel_radius,
            mass: f("physicswheels", "WheelMass")?,
            mass_factor,
        });
        let truck_settings = TruckMassSettings {
            wheel_radius,
            wheel_x_distance,
            mass_factor,
            radius_scalar: f("physicstrucks", "RadiusScalar")?,
            half_height_scalar: f("physicstrucks", "HalfHeightScalar")?,
            mass: f("physicstrucks", "TruckMass")?,
        };
        let truck = truck_mass_properties(truck_settings);
        let gravity = data
            .words::<4>("physics", "default", "WorldGravity")?
            .map(f32::from_bits);
        let simulation = RetailSimulationStep::fixed_60_hz(
            0, // Our host keeps its single board active; sleeping is not scheduled.
            f("physics", "FreezingEnergy")?,
            Vector3::new(gravity[0], gravity[1], gravity[2]),
        );
        let deck_geometry = DeckGeometry::new(DeckGeometrySettings {
            width: f("physicsdeck", "DeckWidth")?,
            mid_length: geometry.deck_mid_length,
            thickness: f("physicsdeck", "DeckThickness")?,
            back_end_size: f("physicsdeck", "DeckBackEndSize")?,
            front_end_angle_degrees: f("physicsdeck", "DeckFrontEndAngle")?,
            back_end_angle_degrees: f("physicsdeck", "DeckBackEndAngle")?,
            end_capsule_count: data.integer("physicsdeck", "default", "DeckEndCapsules")? as i32,
            enable_deck_volume_collisions: b("physicsdeck", "DeckEnableDeckVolumeCollisions")?,
            enable_end_volume_collisions: b("physicsdeck", "DeckEnableEndVolumeCollisions")?,
        });
        let deck = deck_mass_properties(
            &deck_geometry,
            f("physicsdeck", "DeckMass")? * mass_factor,
            f("physicsdeck", "DeckAngularDrag")? * simulation.frequency,
        );
        let step = BoardStepSettings {
            simulation,
            iterations: data.integer("physics", "default", "RWMaxIterations")?,
            base_truck_transforms: calculate_truck_transforms(RetailTruckTransformInputs {
                deck_mid_length: geometry.deck_mid_length,
                truck_z_position_front: geometry.truck_z_position_front,
                truck_z_position_back: geometry.truck_z_position_back,
                truck_y_position: geometry.truck_y_position,
                truck_rotation_axis_angle_degrees: f("physicstrucks", "TruckRotationAxisAngle")?,
            }),
            truck_dynamics: retail_truck_drive_dynamics(RetailTruckDriveSettings {
                use_linear_drives: b("physicstrucks", "UseTruckDrives")?,
                use_hard_linear_drives: b("physicsdeck", "UseHardDrives")?,
                angular_displacement: f("physicstrucks_drives", "Angular_Hard_Displacement")?,
                angular_damping: f("physicstrucks_drives", "Angular_Hard_Damping")?,
                angular_strength: f("physicstrucks_drives", "Angular_Hard_Strength")?,
            }),
            force_point_y_offset: f("physicsdeck", "DeckForceYOffset")?,
        };
        if wheel_radius <= 0.0 || mass_factor <= 0.0 || step.iterations == 0 {
            return Err("Invalid stock board radius, mass factor or solver iterations".into());
        }
        Ok(Self {
            step,
            masses: [wheel, wheel, wheel, wheel, truck, truck, deck],
            authored: authored_body_transforms(geometry),
            deck_geometry,
            truck_shape: truck_mass_input(truck_settings).shape,
            truck_collisions: b("physicstrucks", "TruckEnableVolumeCollisions")?,
            truck_material: RetailContactMaterial {
                static_friction: f("physicstrucks", "TruckStaticFriction")?,
                dynamic_friction: f("physicstrucks", "TruckDynamicFriction")?,
                restitution: f("physicstrucks", "TruckRestitution")?,
            },
            deck_material: RetailContactMaterial {
                static_friction: f("physicsdeck", "DeckStaticFriction")?,
                dynamic_friction: f("physicsdeck", "DeckDynamicFriction")?,
                restitution: f("physicsdeck", "DeckRestitution")?,
            },
            wheel_radius,
            wheel_material: RetailContactMaterial {
                static_friction: f("physicswheels", "WheelStaticFriction")?,
                dynamic_friction: f("physicswheels", "WheelDynamicFriction")?,
                restitution: f("physicswheels", "WheelRestitution")?,
            },
            standard_wheel_material: RetailContactMaterial {
                static_friction: f("physicswheels", "WheelStaticFriction")?,
                dynamic_friction: f("physicswheels", "WheelDynamicFriction")?,
                restitution: f("physicswheels", "WheelRestitution")?,
            },
            floor_material: RetailContactMaterial {
                // Ground job8277C5D8 uses context83034F34/38/3C. The
                // max/max/min combine preserves the moving volume material.
                static_friction: 0.0,
                dynamic_friction: 0.0,
                restitution: 1.0,
            },
            input_magnitude_threshold: f("inputlistener", "StickMagnitudeMinToCountHeld")?,
        })
    }
}
