use bevy::prelude::*;

#[test]
fn actual_source_walking_stair_contact_consumes_each_segment_once() {
    use avian3d::prelude::{LinearVelocity, Position, SpatialQueryFilter};
    use bevy_als::als::character::{AlsCmcAdapterRuntime, AlsMovementParamsHandle};
    use bevy_als::als::gait::AlsMaxAllowedGait;
    use bevy_als::{AlsMovementParams, Gait, LocomotionState, RotationMode};

    const MIN_FLOOR_DISTANCE: f32 = 0.019;
    const MAX_FLOOR_DISTANCE: f32 = 0.024;
    use bevy::ecs::system::RunSystemOnce;
    for (mesh_name, source, mesh_origin, center, input, gait, speed) in [
        (
            "SM_Als_Stairs_01",
            &include_bytes!("../../assets/ALSExtras/Environment/Meshes/SM_Als_Stairs_01.glb")[..],
            Vec3::new(-3.3333, 0.18, -30.0),
            Vec3::new(0.0016979529, 1.2816019, -30.404602),
            Vec2::Y,
            Gait::Walking,
            1.75,
        ),
        (
            "SM_Als_Stairs_03",
            &include_bytes!("../../assets/ALSExtras/Environment/Meshes/SM_Als_Stairs_03.glb")[..],
            Vec3::new(-30.0, 0.36, -30.0),
            Vec3::new(-26.67559, 13.69611, -42.672752),
            Vec2::NEG_Y,
            Gait::Walking,
            1.75,
        ),
        (
            "SM_Als_Stairs_03",
            &include_bytes!("../../assets/ALSExtras/Environment/Meshes/SM_Als_Stairs_03.glb")[..],
            Vec3::new(-30.0, 0.36, -30.0),
            Vec3::new(-26.67559, 13.69611, -42.672752),
            Vec2::NEG_Y,
            Gait::Sprinting,
            6.5,
        ),
    ] {
        let model = bevy_gltf::gltf::binary::Glb::from_slice(source).unwrap();
        let data: serde_json::Value = serde_json::from_slice(&model.json).unwrap();
        let bytes = model.bin.unwrap();
        let primitive = &data["meshes"][0]["primitives"][0];
        let accessor_bytes = |index: usize, width: usize| {
            let accessor = &data["accessors"][index];
            let view = &data["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            assert_eq!(
                view["byteStride"].as_u64().unwrap_or(width as u64) as usize,
                width
            );
            &bytes[offset..offset + accessor["count"].as_u64().unwrap() as usize * width]
        };
        let vertices: Vec<Vec3> = accessor_bytes(
            primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
            12,
        )
        .chunks_exact(12)
        .map(|point| {
            Vec3::from_array(std::array::from_fn(|i| {
                f32::from_le_bytes(point[i * 4..i * 4 + 4].try_into().unwrap())
            }))
        })
        .collect();
        let triangles: Vec<[u32; 3]> =
            accessor_bytes(primitive["indices"].as_u64().unwrap() as usize, 2)
                .chunks_exact(6)
                .map(|triangle| {
                    std::array::from_fn(|i| {
                        u16::from_le_bytes(triangle[i * 2..i * 2 + 2].try_into().unwrap()) as u32
                    })
                })
                .collect();
        let (mut app, character, _) = crate::test_support::loaded_character();
        crate::playground_collision::PlaygroundCollisionPlugin.build(&mut app);
        let mut mesh = Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vertices
                .iter()
                .map(|vertex| vertex.to_array())
                .collect::<Vec<_>>(),
        );
        mesh.insert_indices(bevy::render::mesh::Indices::U32(
            triangles.into_iter().flatten().collect(),
        ));
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let source_transform = Transform::from_translation(mesh_origin);
        let stairs = app
            .world_mut()
            .spawn((
                crate::playground::PlaygroundCollisionSource::new(mesh_name),
                source_transform,
                GlobalTransform::from(source_transform),
            ))
            .id();
        app.world_mut().spawn((
            ChildOf(stairs),
            Mesh3d(mesh),
            Transform::IDENTITY,
            GlobalTransform::from(source_transform),
        ));
        app.world_mut().entity_mut(character).insert((
            Position(center),
            Transform::from_translation(center),
            LinearVelocity::ZERO,
        ));
        for _ in 0..8 {
            app.update();
        }
        assert!(
            app.world()
                .entity(stairs)
                .contains::<crate::playground_collision::PlaygroundMeshColliderBuilt>()
        );
        let handle = app
            .world()
            .get::<AlsMovementParamsHandle>(character)
            .unwrap()
            .0
            .clone();
        let cmc = app
            .world()
            .resource::<Assets<AlsMovementParams>>()
            .get(&handle)
            .unwrap()
            .cmc
            .clone();
        let center = if gait == Gait::Sprinting {
            let source_cmc = cmc.clone();
            app.world_mut()
                .run_system_once(
                    move |spatial: bevy_als::extensions::SceneQuery,
                          candidates: Query<
                        bevy_als::als::motion::CharacterCollisionCandidates,
                    >| {
                        let (floor, _, _) = bevy_als::als::motion::find_ground_floor(
                            &spatial,
                            center,
                            0.9,
                            0.3,
                            &source_cmc,
                            LocomotionState::Grounded,
                            &SpatialQueryFilter::from_excluded_entities([character]),
                            character,
                            &candidates,
                        )
                        .unwrap();
                        center.with_y(floor + (MIN_FLOOR_DISTANCE + MAX_FLOOR_DISTANCE) * 0.5)
                    },
                )
                .unwrap()
        } else {
            center
        };
        app.world_mut()
            .get_mut::<bevy_als::als::gait::AlsDesiredGait>(character)
            .unwrap()
            .0 = gait;
        app.world_mut()
            .get_mut::<bevy_als::als::gait::AlsMaxAllowedGait>(character)
            .unwrap()
            .0 = gait;
        if gait == Gait::Sprinting {
            *app.world_mut().get_mut::<RotationMode>(character).unwrap() =
                RotationMode::VelocityDirection;
            app.world_mut().entity_mut(character).insert(
                bevy_als::als::aim::AlsLastFreeRotationMode(RotationMode::VelocityDirection),
            );
        }
        app.world_mut().entity_mut(character).insert((
            Position(center),
            Transform::from_translation(center),
            LinearVelocity(Vec3::NEG_Z * input.y * speed),
            crate::input::AlsInputCommand {
                move_dir: input,
                ..default()
            },
        ));
        if gait == Gait::Sprinting {
            for _ in 0..3 {
                app.update();
            }
            assert_eq!(
                app.world().get::<AlsMaxAllowedGait>(character).unwrap().0,
                Gait::Sprinting
            );
            app.world_mut().entity_mut(character).insert((
                Position(center),
                Transform::from_translation(center),
                LinearVelocity(Vec3::NEG_Z * input.y * speed),
            ));
        }
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.07051209),
        ));
        for frame in 0..8 {
            let before = app.world().get::<Position>(character).unwrap().0;
            let cmc = cmc.clone();
            app.world_mut().run_system_once(move |spatial: bevy_als::extensions::SceneQuery,
                candidates: Query<bevy_als::als::motion::CharacterCollisionCandidates>| {
                let filter = SpatialQueryFilter::from_excluded_entities([character]);
                let mut source_center = before;
                for step in 0..=2 {
                    if step > 0 { source_center += Vec3::NEG_Z * input.y * (speed * 0.07051209 * 0.5); }
                    let (floor, _, entity) = bevy_als::als::motion::find_ground_floor(&spatial, source_center, 0.9, 0.3,
                        &cmc, LocomotionState::Grounded, &filter, character, &candidates)
                        .expect("Source current and following walking steps must keep a valid stair floor");
                    assert_eq!(entity, stairs);
                    let distance = source_center.y - floor;
                    if step > 0 && !(MIN_FLOOR_DISTANCE..=MAX_FLOOR_DISTANCE).contains(&distance) {
                        source_center.y = floor + (MIN_FLOOR_DISTANCE + MAX_FLOOR_DISTANCE) * 0.5;
                    }
                }
            }).unwrap();
            app.update();
            let after = app.world().get::<Position>(character).unwrap().0;
            let velocity = app.world().get::<LinearVelocity>(character).unwrap().0;
            let runtime = app.world().get::<AlsCmcAdapterRuntime>(character).unwrap();
            let result = app
                .world()
                .get::<bevy_als::als::motion::AlsCmcFrameResult>(character)
                .unwrap();
            assert!(runtime.simulation_substeps > 1);
            assert!(
                (after - before).with_y(0.0).length() <= speed * 0.07051209 + 0.003,
                "Source each walking segment advances once: gait={gait:?} frame={frame} before={before:?} after={after:?} velocity={velocity:?} source_tail={:?}",
                result.grounded_collision_velocity
            );
            assert_eq!(
                app.world().get::<LocomotionState>(character),
                Some(&LocomotionState::Grounded),
                "Source valid stair floors must stay grounded: gait={gait:?} frame={frame} before={before:?} after={after:?} velocity={velocity:?}"
            );
            assert!(
                (velocity.with_y(0.0).length() - speed).abs() < 0.03,
                "Source walkable stair contact must preserve horizontal speed: gait={gait:?} frame={frame} before={before:?} after={after:?} velocity={velocity:?} source_tail={:?}",
                result.grounded_collision_velocity
            );
        }
    }
}
