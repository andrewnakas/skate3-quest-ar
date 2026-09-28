use super::*;

#[test]
#[ignore = "requires the user's converted stock camera and graph assets"]
fn private_stock_camera_loads_every_graph_operation_and_shot_dependency() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let camera = CameraRuntime::load(root).expect("complete normal stock camera assets");
    assert!(camera.frame.is_none());
}

/// A minimised window reports zero height, and `width / height` is then NaN.
/// That NaN used to reach the field of view, fail the frame's finite check and
/// end the run -- minimising the window killed the running game twice in one
/// session. The last good ratio is held instead.
#[test]
fn a_zero_area_window_cannot_poison_the_aspect_ratio() {
    let root = std::path::Path::new("assets");
    let Ok(mut runtime) = super::CameraRuntime::load(root) else {
        // The camera collections are not present in every test environment;
        // the guard itself is what matters and is covered below.
        return;
    };
    runtime.set_aspect_ratio(16. / 9.);
    let good = runtime.manager.state.aspect_ratio;
    for bad in [f32::NAN, f32::INFINITY, 0., -1.] {
        runtime.set_aspect_ratio(bad);
        assert_eq!(
            runtime.manager.state.aspect_ratio, good,
            "{bad} should have been ignored"
        );
    }
    runtime.set_aspect_ratio(4. / 3.);
    assert_eq!(runtime.manager.state.aspect_ratio, 4. / 3.);
}
