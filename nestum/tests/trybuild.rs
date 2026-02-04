#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass_basic.rs");
    t.pass("tests/ui/pass_external.rs");
    t.pass("tests/ui/pass_external_cross_file.rs");
    t.pass("tests/ui/pass_match.rs");
    t.pass("tests/ui/pass_match_alias.rs");
    t.compile_fail("tests/ui/fail_enum_args.rs");
    t.compile_fail("tests/ui/fail_variant_attr.rs");
    t.compile_fail("tests/ui/fail_external_not_tuple.rs");
    t.compile_fail("tests/ui/fail_external_mismatch.rs");
    t.compile_fail("tests/ui/fail_external_not_found.rs");
}
