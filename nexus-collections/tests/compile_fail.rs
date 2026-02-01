//! Compile-fail tests using trybuild.
//!
//! These tests verify that the API prevents misuse at compile time.
//! Each .rs file in tests/ui/ should fail to compile with a specific error.

#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
