// SPDX-License-Identifier: LicenseRef-REAL-1.0

/// A command enum with a variant that has no handler, or a variant that holds
/// no command, must not compile.
#[test]
fn ui() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
