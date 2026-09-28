//! Native MotionGraph transition hooks, dispatched by the stock controller.
use skate_core::animation::playback::TransitionSettings;
use skate_data::state_graph::attributes::Attributes;

#[derive(Clone, Debug, PartialEq)]
pub enum MotionHook {
    ///82BBBB68 copies the authored transition into the next-play override.
    Override(TransitionSettings),
    MongoPushToAntic {
        animation: String,
    },
    ///82BBB848, factory82BCA6A8 default right=true.
    GrabSlide {
        right: bool,
    },
    /// Endless Tricks. **Not retail** -- fires on the synthesised loop transition to count the
    /// rung the skater has just started. See `graph_host::endless_flip`.
    EndlessFlipAdvance {
        trick: String,
        base_rung: u32,
    },
}
impl MotionHook {
    pub fn parse(a: &Attributes<'_>) -> Option<Self> {
        match a.text("name")? {
            "EndlessFlipAdvance" => Some(Self::EndlessFlipAdvance {
                trick: a.text("trick").unwrap_or("").into(),
                base_rung: a.text("baseRung").and_then(|v| v.parse().ok()).unwrap_or(4),
            }),
            "GrabSlide" => Some(Self::GrabSlide {
                right: a.boolean_byte("right", 1) != 0,
            }),
            "OverideNextAnimTransitionHook" => {
                Some(Self::Override(super::motion_nodes::transition(a)))
            }
            "MongoPushToAntic" => Some(Self::MongoPushToAntic {
                animation: a.text("anim").unwrap_or("").into(),
            }),
            _ => None,
        }
    }
}
