/// Compile-fail diagnostics tests.
///
/// These test that our `#[diagnostic::on_unimplemented]` annotations produce
/// helpful error messages. The `.stderr` files are generated with default
/// features only. Optional features (timer, mio, smartptr) change the
/// compiler's suggestion list, causing stderr mismatches. Skip when
/// any optional feature is active.
#[test]
#[cfg(not(any(
    feature = "timer",
    feature = "mio",
    feature = "smartptr",
    feature = "signals"
)))]
fn diagnostics() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
