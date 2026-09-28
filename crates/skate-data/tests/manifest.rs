use skate_data::GameAssets;

fn manifest(path: &str, version: u32) -> String {
    serde_json::json!({
        "version": version,
        "character_scene": path,
        "initial_animation": "R_IDLE_HCOM_000",
        "action_graph": "private/stock/data/state/ActionGraph_OnBoard.stategraph",
        "motion_graph": "private/stock/data/state/MotionGraph_OnBoard.stategraph"
    })
    .to_string()
}
#[test]
fn validates_contract_without_loading_private_assets() {
    assert_eq!(
        GameAssets::parse(&manifest("private/skater.glb", 1))
            .unwrap()
            .initial_animation,
        "R_IDLE_HCOM_000"
    );
}
#[test]
fn rejects_incompatible_versions_and_ambiguous_paths() {
    assert!(GameAssets::parse(&manifest("private/skater.glb", 2)).is_err());
    for path in [
        "../skater.glb",
        "/skater.glb",
        "C:/skater.glb",
        "private\\skater.glb",
        "private/skater.glb#Scene0",
        "private/skater.json",
        "",
    ] {
        assert!(
            GameAssets::parse(&manifest(path, 1)).is_err(),
            "accepted {path}"
        );
    }
}

#[test]
fn requires_relative_compiled_stock_graph_paths() {
    let mut value: serde_json::Value =
        serde_json::from_str(&manifest("private/skater.glb", 1)).unwrap();
    for field in ["action_graph", "motion_graph"] {
        for path in ["../graph.stategraph", "/graph.stategraph", "graph.xml", ""] {
            value[field] = path.into();
            assert!(
                GameAssets::parse(&serde_json::to_string(&value).unwrap()).is_err(),
                "accepted {field}={path}"
            );
        }
        value[field] = format!("private/{field}.stategraph").into();
    }
}
#[test]
fn reports_missing_data_as_an_error() {
    let root = std::env::temp_dir().join(format!("skate-missing-assets-{}", std::process::id()));
    let error = GameAssets::load(&root).unwrap_err().to_string();
    assert!(error.contains("private") && error.contains("Prepare-Assets"));
}
