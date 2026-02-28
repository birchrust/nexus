//! Plugin trait for composable system and resource registration.

use crate::scheduler::SchedulerBuilder;
use crate::world::WorldBuilder;

/// Composable unit of system and resource registration.
///
/// Plugins register resources into a [`WorldBuilder`] and systems
/// into a [`SchedulerBuilder`]. The runtime is assembled by composing
/// plugins via [`App`](crate::App).
///
/// # Examples
///
/// ```ignore
/// struct TradingPlugin;
///
/// impl Plugin for TradingPlugin {
///     fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder) {
///         world.register(PriceCache::new());
///         world.register_default::<Events<TradeSignal>>();
///
///         let sys = scheduler.add_system(update_prices, world.registry());
///         // ...
///     }
/// }
/// ```
pub trait Plugin {
    /// Register resources and systems.
    fn build(&self, world: &mut WorldBuilder, scheduler: &mut SchedulerBuilder);
}
