//! Components container prototype.
//!
//! This example demonstrates the runtime component storage and system dispatch
//! design that will become the foundation of `nexus-rt`. It proves the full
//! flow from registration through dispatch with real types.
//!
//! # Architecture
//!
//! The design follows Bevy's ECS resource model adapted for a single-threaded,
//! event-driven runtime where components are singleton resources (not per-entity).
//!
//! ## Lifecycle
//!
//! ```text
//! ┌─────────────┐     ┌─────────────┐     ┌─────────────────────┐
//! │  Register    │────▶│    Build     │────▶│      Dispatch       │
//! │  (cold)      │     │  (cold)     │     │      (hot)          │
//! │              │     │             │     │                     │
//! │ add_component│     │ Resolve     │     │ for system in systems│
//! │ add_system   │     │ TypeId→Idx  │     │   system(&components)│
//! │              │     │ Cache in    │     │                     │
//! │              │     │ closure env │     │ Each call:          │
//! │              │     │             │     │   get_unchecked(id) │
//! │              │     │             │     │   cast *mut u8 → &T │
//! │              │     │             │     │   call user fn      │
//! └─────────────┘     └─────────────┘     └─────────────────────┘
//! ```
//!
//! ## Key Design Decisions
//!
//! **Dense ComponentId indexing (Bevy's approach):**
//! Each component type is assigned a dense sequential index at registration.
//! `TypeId → HashMap → ComponentId(usize)` happens once at build time.
//! Dispatch uses `Vec::get_unchecked(id)` — a single pointer + offset.
//!
//! **No cached pointers in systems:**
//! Systems don't cache `*mut u8` pointers to components. They cache
//! `ComponentId` indices and resolve pointers at dispatch time via the
//! component `Vec`. This is simpler (no stale pointer risk) and costs
//! ~3 cycles per fetch vs ~2 for cached pointers — negligible.
//!
//! **Box::into_raw for stable storage:**
//! Components are heap-allocated via `Box::new`, then `Box::into_raw`
//! transfers ownership to the container. The raw pointer is stored in
//! `Vec<*mut u8>`. A parallel `Vec<DropFn>` remembers how to reconstruct
//! and drop each Box. No `dyn Any`, no double storage.
//!
//! **Closure-based system dispatch:**
//! Each system is a `Box<dyn FnMut(&Components)>` closure that captures
//! its resolved `ComponentId` values. The macro generates these closures
//! from the user's function signature. Dispatch is: iterate Vec → vtable
//! call → read captured IDs → `get_unchecked` → cast → call user fn.
//!
//! **Build-time validation:**
//! All `ComponentId` lookups happen at build time via `HashMap<TypeId, ComponentId>`.
//! If a system requests a component that wasn't registered, it panics at
//! build — not at dispatch. After build, the HashMap is never touched.
//! `get_unchecked` at dispatch is sound because all indices were validated.
//!
//! ## Invariants
//!
//! 1. After `build()`, component storage is frozen — no inserts, no removes.
//! 2. All `ComponentId` values returned by `register()` are valid indices
//!    into `ptrs` for the lifetime of the `Components` container.
//! 3. Pointers in `ptrs` are stable (heap-allocated via Box) and valid
//!    for the lifetime of the `Components` container.
//! 4. Systems run sequentially. No two systems execute concurrently.
//!    Mutable aliasing is safe because only one system accesses a component
//!    at a time — enforced by the single-threaded dispatch loop.
//! 5. `drop_fns[i]` is the correct destructor for `ptrs[i]`. These are
//!    monomorphized at registration and must only be called once (in Drop).
//!
//! ## What a proc macro would generate
//!
//! Given a user function:
//! ```rust,ignore
//! fn update_prices(prices: &mut PriceCache, venues: &VenueState) { ... }
//! ```
//!
//! The macro generates a builder that:
//! 1. Inspects the function signature for parameter types and mutability
//! 2. Resolves `ComponentId` for each parameter type from the registry
//! 3. Returns a closure capturing those IDs
//! 4. At dispatch, the closure indexes into `Components`, casts, and calls
//!
//! This prototype hand-writes what the macro would generate.
//!
//! ## Performance (measured in perf_fetch.rs)
//!
//! | Strategy                           | p50 | p99 | p999 |
//! |------------------------------------|-----|-----|------|
//! | Direct struct field (compile-time) |   2 |   5 |    9 |
//! | Vec + get_unchecked (this design)  |   3 |   5 |    7 |
//! | Vec + bounds check                 |   3 |   8 |   17 |
//! | Box<dyn Any> + downcast            |   8 |  13 |   24 |
//!
//! Run with:
//! ```bash
//! cargo run --release -p nexus-rt --example proto_components
//! ```

use std::any::TypeId;
use std::collections::HashMap;

// =============================================================================
// Core types
// =============================================================================

/// Dense index identifying a component type within a [`Components`] container.
///
/// Assigned sequentially at registration (0, 1, 2, ...). Used as a direct
/// index into `Vec<*mut u8>` at dispatch time — no hashing, no searching.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct ComponentId(usize);

/// Type-erased drop function. Monomorphized at registration time so we
/// can reconstruct and drop the original `Box<T>` from a `*mut u8`.
type DropFn = unsafe fn(*mut u8);

/// Singleton component storage.
///
/// Components are heap-allocated, type-erased, and indexed by dense
/// [`ComponentId`]. The `HashMap` is only used at build time for
/// `TypeId → ComponentId` resolution. Dispatch uses `get_unchecked`.
struct Components {
    /// Build-time lookup: `TypeId` → dense index.
    indices: HashMap<TypeId, ComponentId>,
    /// Dispatch-time storage: dense array of type-erased pointers.
    /// Each pointer was produced by `Box::into_raw`.
    ptrs: Vec<*mut u8>,
    /// Parallel array of drop functions for cleanup.
    /// `drop_fns[i]` knows how to reconstruct and drop the `Box<T>`
    /// behind `ptrs[i]`.
    drop_fns: Vec<DropFn>,
}

/// Reconstruct and drop a `Box<T>` from a raw pointer.
///
/// # Safety
///
/// `ptr` must have been produced by `Box::into_raw(Box::new(value))`
/// where `value: T`. Must only be called once per pointer.
unsafe fn drop_component<T>(ptr: *mut u8) {
    // SAFETY: ptr was produced by Box::into_raw(Box::new(value))
    // where value: T. Called exactly once in Components::drop.
    unsafe {
        let _ = Box::from_raw(ptr as *mut T);
    }
}

impl Components {
    fn new() -> Self {
        Self {
            indices: HashMap::new(),
            ptrs: Vec::new(),
            drop_fns: Vec::new(),
        }
    }

    /// Register a component. Returns its dense [`ComponentId`].
    ///
    /// The value is heap-allocated via `Box` and ownership is transferred
    /// to the container. The pointer is stable for the lifetime of the
    /// container (heap allocation does not move).
    ///
    /// # Panics
    ///
    /// Panics if a component of the same type is already registered.
    fn register<T: 'static>(&mut self, value: T) -> ComponentId {
        let type_id = TypeId::of::<T>();
        assert!(
            !self.indices.contains_key(&type_id),
            "component already registered"
        );

        let ptr = Box::into_raw(Box::new(value)) as *mut u8;
        let id = ComponentId(self.ptrs.len());
        self.indices.insert(type_id, id);
        self.ptrs.push(ptr);
        self.drop_fns.push(drop_component::<T>);
        id
    }

    /// Resolve the [`ComponentId`] for a type. Build-time only.
    ///
    /// # Panics
    ///
    /// Panics if the component type was not registered.
    fn id<T: 'static>(&self) -> ComponentId {
        *self
            .indices
            .get(&TypeId::of::<T>())
            .expect("component not registered")
    }

    /// Fetch a component pointer by pre-validated index. Dispatch-time.
    ///
    /// # Safety
    ///
    /// `id` must have been returned by [`register()`](Components::register)
    /// on this container. The caller must ensure no mutable aliasing
    /// (only one `&mut T` reference active at a time per component).
    #[inline(always)]
    unsafe fn get_ptr(&self, id: ComponentId) -> *mut u8 {
        // SAFETY: caller guarantees id was returned by register() on this
        // container, so id.0 < self.ptrs.len().
        unsafe { *self.ptrs.get_unchecked(id.0) }
    }
}

impl Drop for Components {
    fn drop(&mut self) {
        for (ptr, drop_fn) in self.ptrs.iter().zip(&self.drop_fns) {
            // SAFETY: each (ptr, drop_fn) pair was created together in
            // register(). drop_fn is the monomorphized destructor for the
            // concrete type behind ptr. Called exactly once here.
            unsafe {
                drop_fn(*ptr);
            }
        }
    }
}

// =============================================================================
// System dispatch
// =============================================================================

/// Type-erased system. Each system is a closure that captures its resolved
/// [`ComponentId`] values and knows how to fetch + cast + call.
type SystemFn = Box<dyn FnMut(&Components)>;

/// Builder that produces a system closure from resolved component IDs.
/// In the real implementation, a proc macro generates these from the
/// user's function signature.
type SystemBuilder = fn(&Components) -> SystemFn;

/// Application builder. Collects components and system builders,
/// then [`build()`](App::build) resolves everything into a dispatchable
/// [`Runtime`].
struct App {
    components: Components,
    builders: Vec<SystemBuilder>,
}

impl App {
    fn new() -> Self {
        Self {
            components: Components::new(),
            builders: Vec::new(),
        }
    }

    fn add_component<T: 'static>(&mut self, value: T) -> ComponentId {
        self.components.register(value)
    }

    fn add_system(&mut self, builder: SystemBuilder) {
        self.builders.push(builder);
    }

    /// Freeze components and resolve all systems.
    ///
    /// After this, the component set is immutable. All system builders
    /// run against the final component registry, caching their
    /// [`ComponentId`] values in closure environments.
    ///
    /// # Panics
    ///
    /// Panics if any system builder requests a component that was not
    /// registered. This is the build-time validation guarantee — if
    /// `build()` succeeds, all dispatch-time fetches are valid.
    fn build(self) -> Runtime {
        let systems = self
            .builders
            .iter()
            .map(|builder| builder(&self.components))
            .collect();
        Runtime {
            components: self.components,
            systems,
        }
    }
}

/// Frozen runtime. Components are immutable, systems are resolved.
/// Call [`dispatch()`](Runtime::dispatch) to run all systems sequentially.
struct Runtime {
    components: Components,
    systems: Vec<SystemFn>,
}

impl Runtime {
    /// Run all systems sequentially. Each system receives a reference to
    /// the component storage and uses its cached IDs to fetch what it needs.
    fn dispatch(&mut self) {
        for system in &mut self.systems {
            (system)(&self.components);
        }
    }
}

// =============================================================================
// User-defined components
// =============================================================================

struct PriceCache {
    mid_price: f64,
    bid: f64,
    ask: f64,
    update_count: u64,
}

#[allow(dead_code)]
struct VenueState {
    connected: bool,
    sequence: u64,
    name: &'static str,
}

struct OrderBook {
    bids: Vec<(f64, f64)>,
    #[allow(dead_code)]
    asks: Vec<(f64, f64)>,
}

// =============================================================================
// User-defined system functions
//
// These are plain functions. The user writes these. A proc macro would
// inspect the signature and generate the corresponding builder.
// =============================================================================

/// Updates PriceCache from VenueState. Demonstrates mutable + immutable access.
fn update_prices(prices: &mut PriceCache, venues: &VenueState) {
    if venues.connected {
        prices.mid_price = f64::midpoint(prices.bid, prices.ask);
        prices.update_count += 1;
    }
}

/// Reads PriceCache and OrderBook to log state. Demonstrates read-only access
/// to multiple components.
fn log_state(prices: &PriceCache, book: &OrderBook) {
    println!(
        "  [log_state] venue mid={:.2} updates={} book_depth={}",
        prices.mid_price,
        prices.update_count,
        book.bids.len(),
    );
}

/// Mutates OrderBook. Demonstrates single mutable parameter.
fn update_book(book: &mut OrderBook) {
    // Simulate a new level arriving
    if book.bids.len() < 5 {
        let last_bid = book.bids.last().map_or(100.0, |b| b.0);
        book.bids.push((last_bid - 0.5, 10.0));
    }
}

// =============================================================================
// Macro-generated system builders
//
// In the real implementation, a proc macro inspects the function signature
// and generates these. The pattern is mechanical:
//   1. Resolve ComponentId for each parameter type
//   2. Return a closure that fetches + casts + calls
//
// Each builder below is exactly what the macro would expand to.
// =============================================================================

/// Generated for: `fn update_prices(prices: &mut PriceCache, venues: &VenueState)`
fn build_update_prices(components: &Components) -> SystemFn {
    let price_id = components.id::<PriceCache>();
    let venue_id = components.id::<VenueState>();

    Box::new(move |components: &Components| {
        // SAFETY: IDs were validated at build time (id() would have panicked).
        // Single-threaded sequential dispatch ensures no concurrent access.
        // PriceCache is &mut, VenueState is & — no aliasing conflict.
        let prices = unsafe { &mut *(components.get_ptr(price_id) as *mut PriceCache) };
        let venues = unsafe { &*(components.get_ptr(venue_id) as *const VenueState) };
        update_prices(prices, venues);
    })
}

/// Generated for: `fn log_state(prices: &PriceCache, book: &OrderBook)`
fn build_log_state(components: &Components) -> SystemFn {
    let price_id = components.id::<PriceCache>();
    let book_id = components.id::<OrderBook>();

    Box::new(move |components: &Components| {
        // SAFETY: both parameters are shared references — no aliasing concern.
        let prices = unsafe { &*(components.get_ptr(price_id) as *const PriceCache) };
        let book = unsafe { &*(components.get_ptr(book_id) as *const OrderBook) };
        log_state(prices, book);
    })
}

/// Generated for: `fn update_book(book: &mut OrderBook)`
fn build_update_book(components: &Components) -> SystemFn {
    let book_id = components.id::<OrderBook>();

    Box::new(move |components: &Components| {
        // SAFETY: single &mut parameter, no aliasing possible.
        let book = unsafe { &mut *(components.get_ptr(book_id) as *mut OrderBook) };
        update_book(book);
    })
}

// =============================================================================
// Main — demonstrates the full lifecycle
// =============================================================================

fn main() {
    println!("COMPONENTS PROTOTYPE");
    println!("====================\n");

    // ---- Registration (cold path) ----
    println!("Phase 1: Registration");
    let mut app = App::new();

    app.add_component(PriceCache {
        mid_price: 0.0,
        bid: 100.0,
        ask: 101.0,
        update_count: 0,
    });
    app.add_component(VenueState {
        connected: true,
        sequence: 0,
        name: "Coinbase",
    });
    app.add_component(OrderBook {
        bids: vec![(100.0, 10.0), (99.5, 20.0)],
        asks: vec![(101.0, 15.0), (101.5, 25.0)],
    });
    println!("  Registered: PriceCache, VenueState, OrderBook");

    app.add_system(build_update_prices);
    app.add_system(build_update_book);
    app.add_system(build_log_state);
    println!("  Systems: update_prices, update_book, log_state");

    // ---- Build (cold path) ----
    println!("\nPhase 2: Build");
    println!("  Resolving TypeId → ComponentId for all system parameters...");
    let mut runtime = app.build();
    println!("  Build complete. Components frozen, systems resolved.\n");

    // ---- Dispatch (hot path) ----
    println!("Phase 3: Dispatch (3 ticks)\n");
    for tick in 0..3 {
        println!("--- tick {} ---", tick);
        runtime.dispatch();
        println!();
    }

    println!("Done. Components will be dropped via stored drop_fns.");
}
