#[test]
fn config() {
    let t = trybuild::TestCases::new();
    t.pass("tests/fixtures/fixture_configs.rs");
}
