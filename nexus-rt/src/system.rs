//! Reconciliation systems with boolean propagation.
//!
//! [`System`] is the dispatch trait for per-frame reconciliation logic.
//! Unlike [`Handler`](crate::Handler) (reactive, per-event, no return
//! value), systems return `bool` to control DAG traversal in a
//! [`SystemScheduler`](crate::scheduler::SystemScheduler).
//!
//! Systems are converted from plain functions via [`IntoSystem`], using
//! the same HRTB double-bound pattern as [`IntoHandler`](crate::IntoHandler).
//! The function signature is `fn(params...) -> bool` — no event parameter.

use crate::handler::Param;
use crate::world::{Registry, World};

// =============================================================================
// System trait
// =============================================================================

/// Object-safe dispatch trait for reconciliation systems.
///
/// Returns `bool` to control downstream propagation in a DAG scheduler.
/// `true` means "my outputs changed, run downstream systems."
/// `false` means "nothing changed, skip downstream."
///
/// # Difference from [`Handler`](crate::Handler)
///
/// | | Handler | System |
/// |---|---------|--------|
/// | Trigger | Per-event | Per-scheduler-pass |
/// | Event param | Yes (`E`) | No |
/// | Return | `()` | `bool` |
/// | Purpose | React | Reconcile |
pub trait System: Send {
    /// Run this system. Returns `true` if downstream systems should run.
    fn run(&mut self, world: &mut World) -> bool;

    /// Returns the system's name for diagnostics.
    fn name(&self) -> &'static str {
        "<unnamed>"
    }
}

// =============================================================================
// SystemFn — concrete dispatch wrapper
// =============================================================================

/// Concrete system wrapper produced by [`IntoSystem`].
///
/// Stores the function, pre-resolved parameter state, and a diagnostic
/// name. Users rarely name this type directly — use `Box<dyn System>`
/// for type-erased storage, or let inference handle the concrete type.
pub struct SystemFn<F, Params: Param> {
    f: F,
    state: Params::State,
    name: &'static str,
}

// =============================================================================
// IntoSystem — conversion trait
// =============================================================================

/// Converts a plain function into a [`System`].
///
/// The function signature is `fn(params...) -> bool` — no event parameter.
/// Parameters are resolved from a [`Registry`] at conversion time.
///
/// # Closures vs named functions
///
/// Zero-parameter systems (`fn() -> bool`) accept closures. For
/// parameterized systems (one or more [`Param`] arguments), Rust's
/// HRTB + GAT inference fails on closures — use named functions.
/// Same limitation as [`IntoHandler`](crate::IntoHandler).
///
/// # Examples
///
/// ```
/// use nexus_rt::{WorldBuilder, Res, ResMut, IntoSystem, System};
///
/// fn reconcile(val: Res<u64>, mut flag: ResMut<bool>) -> bool {
///     if *val > 10 {
///         *flag = true;
///         true
///     } else {
///         false
///     }
/// }
///
/// let mut builder = WorldBuilder::new();
/// builder.register::<u64>(42);
/// builder.register::<bool>(false);
/// let mut world = builder.build();
///
/// let mut sys = reconcile.into_system(world.registry());
/// assert!(sys.run(&mut world));
/// assert!(*world.resource::<bool>());
/// ```
///
/// # Panics
///
/// Panics if any [`Param`](crate::Param) resource is not registered in
/// the [`Registry`].
pub trait IntoSystem<Params> {
    /// The concrete system type produced.
    type System: System + 'static;

    /// Convert this function into a system, resolving parameters from the registry.
    fn into_system(self, registry: &Registry) -> Self::System;
}

// =============================================================================
// Arity 0: fn() -> bool
// =============================================================================

impl<F: FnMut() -> bool + Send + 'static> IntoSystem<()> for F {
    type System = SystemFn<F, ()>;

    fn into_system(self, registry: &Registry) -> Self::System {
        SystemFn {
            f: self,
            state: <() as Param>::init(registry),
            name: std::any::type_name::<F>(),
        }
    }
}

impl<F: FnMut() -> bool + Send + 'static> System for SystemFn<F, ()> {
    fn run(&mut self, _world: &mut World) -> bool {
        (self.f)()
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

// =============================================================================
// Macro-generated impls (arities 1-8)
// =============================================================================

macro_rules! impl_into_system {
    ($($P:ident),+) => {
        impl<F: Send + 'static, $($P: Param + 'static),+> IntoSystem<($($P,)+)> for F
        where
            for<'a> &'a mut F: FnMut($($P,)+) -> bool
                              + FnMut($($P::Item<'a>,)+) -> bool,
        {
            type System = SystemFn<F, ($($P,)+)>;

            fn into_system(self, registry: &Registry) -> Self::System {
                let state = <($($P,)+) as Param>::init(registry);
                {
                    #[allow(non_snake_case)]
                    let ($($P,)+) = &state;
                    registry.check_access(&[
                        $(
                            (<$P as Param>::resource_id($P),
                             std::any::type_name::<$P>()),
                        )+
                    ]);
                }
                SystemFn {
                    f: self,
                    state,
                    name: std::any::type_name::<F>(),
                }
            }
        }

        impl<F: Send + 'static, $($P: Param + 'static),+> System
            for SystemFn<F, ($($P,)+)>
        where
            for<'a> &'a mut F: FnMut($($P,)+) -> bool
                              + FnMut($($P::Item<'a>,)+) -> bool,
        {
            #[allow(non_snake_case)]
            fn run(&mut self, world: &mut World) -> bool {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($P),+>(
                    mut f: impl FnMut($($P),+) -> bool,
                    $($P: $P,)+
                ) -> bool {
                    f($($P),+)
                }

                // SAFETY: state was produced by init() on the same registry
                // that built this world. Single-threaded sequential dispatch
                // ensures no mutable aliasing across params.
                let ($($P,)+) = unsafe {
                    <($($P,)+) as Param>::fetch(world, &mut self.state)
                };
                call_inner(&mut self.f, $($P),+)
            }

            fn name(&self) -> &'static str {
                self.name
            }
        }
    };
}

macro_rules! all_tuples {
    ($m:ident) => {
        $m!(P0);
        $m!(P0, P1);
        $m!(P0, P1, P2);
        $m!(P0, P1, P2, P3);
        $m!(P0, P1, P2, P3, P4);
        $m!(P0, P1, P2, P3, P4, P5);
        $m!(P0, P1, P2, P3, P4, P5, P6);
        $m!(P0, P1, P2, P3, P4, P5, P6, P7);
    };
}

all_tuples!(impl_into_system);

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Local, Res, ResMut, WorldBuilder};

    // -- Arity 0 ----------------------------------------------------------

    fn always_true() -> bool {
        true
    }

    #[test]
    fn arity_0_system() {
        let mut world = WorldBuilder::new().build();
        let mut sys = always_true.into_system(world.registry());
        assert!(sys.run(&mut world));
    }

    // -- Single param -----------------------------------------------------

    fn check_threshold(val: Res<u64>) -> bool {
        *val > 10
    }

    #[test]
    fn single_param_system() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        let mut sys = check_threshold.into_system(world.registry());
        assert!(sys.run(&mut world));
    }

    #[test]
    fn single_param_system_false() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(5);
        let mut world = builder.build();

        let mut sys = check_threshold.into_system(world.registry());
        assert!(!sys.run(&mut world));
    }

    // -- Two params -------------------------------------------------------

    fn reconcile(val: Res<u64>, mut flag: ResMut<bool>) -> bool {
        if *val > 10 {
            *flag = true;
            true
        } else {
            false
        }
    }

    #[test]
    fn two_param_system() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        builder.register::<bool>(false);
        let mut world = builder.build();

        let mut sys = reconcile.into_system(world.registry());
        assert!(sys.run(&mut world));
        assert!(*world.resource::<bool>());
    }

    // -- Box<dyn System> --------------------------------------------------

    #[test]
    fn box_dyn_system() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(42);
        let mut world = builder.build();

        let mut boxed: Box<dyn System> = Box::new(check_threshold.into_system(world.registry()));
        assert!(boxed.run(&mut world));
    }

    // -- Access conflict detection ----------------------------------------

    #[test]
    #[should_panic(expected = "conflicting access")]
    fn system_access_conflict_panics() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let world = builder.build();

        fn bad(a: Res<u64>, b: ResMut<u64>) -> bool {
            let _ = (*a, &*b);
            true
        }

        let _sys = bad.into_system(world.registry());
    }

    // -- Local<T> in systems ----------------------------------------------

    fn counting_system(mut count: Local<u64>, mut val: ResMut<u64>) -> bool {
        *count += 1;
        *val = *count;
        *count < 3
    }

    #[test]
    fn local_in_system() {
        let mut builder = WorldBuilder::new();
        builder.register::<u64>(0);
        let mut world = builder.build();

        let mut sys = counting_system.into_system(world.registry());
        assert!(sys.run(&mut world)); // count=1 < 3
        assert!(sys.run(&mut world)); // count=2 < 3
        assert!(!sys.run(&mut world)); // count=3, not < 3
        assert_eq!(*world.resource::<u64>(), 3);
    }

    // -- Name -------------------------------------------------------------

    #[test]
    fn system_has_name() {
        let world = WorldBuilder::new().build();
        let sys = always_true.into_system(world.registry());
        assert!(sys.name().contains("always_true"));
    }
}
