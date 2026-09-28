use crate::app::FrameSet;
use bevy::{asset::LoadState, gltf::Gltf, prelude::*};
use skate_data::GameAssets;

#[derive(Resource)]
pub(crate) struct AssetManifest(pub GameAssets);
#[derive(Resource)]
pub(crate) struct CharacterAsset(pub Handle<Gltf>);
#[derive(Resource, Default, PartialEq, Eq)]
pub(crate) enum AssetStatus {
    #[default]
    Loading,
    Ready,
    Failed,
}

pub(crate) struct GameAssetsPlugin;
impl Plugin for GameAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AssetStatus>()
            .add_systems(Startup, load)
            .add_systems(Update, check.in_set(FrameSet::Assets));
    }
}
fn load(mut commands: Commands, server: Res<AssetServer>, manifest: Res<AssetManifest>) {
    commands.insert_resource(CharacterAsset(
        server.load(manifest.0.character_scene.clone()),
    ));
}
fn check(
    server: Res<AssetServer>,
    character: Res<CharacterAsset>,
    mut status: ResMut<AssetStatus>,
    mut exit: MessageWriter<AppExit>,
) {
    if *status != AssetStatus::Loading {
        return;
    }
    if let Some(LoadState::Failed(error)) = server.get_load_state(character.0.id()) {
        error!("Character asset failed to load: {error}");
        *status = AssetStatus::Failed;
        exit.write(AppExit::error());
    } else if server.is_loaded_with_dependencies(character.0.id()) {
        *status = AssetStatus::Ready;
    }
}
