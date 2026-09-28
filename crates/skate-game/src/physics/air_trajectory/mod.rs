//! Real stock-settings and BoardWorld adapter for the player trajectory selector.
mod grind;
mod settings;
use crate::grind_world::StaticProvider;
pub(crate) use grind::GrindContext;
use std::sync::Arc;
mod world;
use skate_core::{
    air::trajectory::{
        LaunchInfo, QueryRequest, QueryResult, SelectorInput, SelectorSettings, TrajectorySelector,
        query_trajectory,
    },
    physics::board_world::BoardWorld,
};
use skate_data::collections::Collections;

pub struct AirTrajectoryRuntime {
    pub selector: TrajectorySelector,
    pub settings: SelectorSettings,
    pending_results: Option<Vec<QueryResult>>,
    grind_settings: grind::Settings,
    grind_world: Option<Arc<StaticProvider>>,
    nearby_grinds: Vec<usize>,
}
impl AirTrajectoryRuntime {
    ///Full82E099A0 query shared by trajectory and Footplant callers.
    pub fn query(world: &BoardWorld, request: QueryRequest) -> Result<QueryResult, String> {
        query_trajectory(
            request,
            |start, end, radius| world::line(world, start, end, radius),
            |position, radius| world::nearby(world, position, radius),
        )
    }
    pub fn load(collections: &Collections) -> Result<Self, String> {
        Ok(Self {
            selector: TrajectorySelector::new(),
            settings: settings::load(collections)?,
            pending_results: None,
            grind_settings: grind::Settings::load(collections)?,
            grind_world: None,
            nearby_grinds: Vec::new(),
        })
    }
    pub fn launch(
        &mut self,
        info: LaunchInfo,
        input: SelectorInput,
        world: &BoardWorld,
    ) -> Result<bool, String> {
        let launched = self.selector.launch(info, input, &self.settings)?;
        if launched {
            self.submit(world)?;
        }
        Ok(launched)
    }
    pub fn bind_grind_world(&mut self, provider: Arc<StaticProvider>) {
        self.grind_world = Some(provider);
        self.nearby_grinds.clear();
    }
    pub fn update(
        &mut self,
        input: SelectorInput,
        world: &BoardWorld,
        context: GrindContext,
    ) -> Result<bool, String> {
        let Some(results) = self.pending_results.take() else {
            return Ok(self.selector.update_without_completion());
        };
        let provider = self
            .grind_world
            .as_deref()
            .ok_or("Trajectory static grind provider was not registered")?;
        let valid = self.selector.complete_batch(
            &results,
            input,
            &self.settings,
            |prediction, acquire| {
                self.grind_settings.evaluate(
                    prediction,
                    acquire,
                    world,
                    provider,
                    &mut self.nearby_grinds,
                    input.grind_lock_distance,
                    context,
                )
            },
            |start, end, radius| world::line(world, start, end, radius),
        )?;
        if self.selector.pending() {
            self.submit(world)?;
        }
        Ok(valid)
    }
    fn submit(&mut self, world: &BoardWorld) -> Result<(), String> {
        let results = self
            .selector
            .requests()
            .iter()
            .map(|&request| Self::query(world, request))
            .collect::<Result<Vec<_>, _>>()?;
        self.pending_results = Some(results);
        Ok(())
    }
}
