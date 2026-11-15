use ra_gotchas::{Catalogue, Conditions, Verdict};

fn long_output_rewrites(command: &str) -> bool {
    matches!(Catalogue::seed().command(command, Conditions::default()), Verdict::Rewrite { id, .. } if id == "long-output")
}

#[test]
fn a_cargo_test_naming_one_test_binary_runs_as_written() {
    assert!(!long_output_rewrites("cargo test -q -p ra-console --test long_output_single_binary"));
    assert!(!long_output_rewrites("cargo test -p ra-console --test=format"));
}

#[test]
fn a_cargo_test_without_a_test_binary_still_gets_its_tail() {
    assert!(long_output_rewrites("cargo test -p ra-console -q"));
    assert!(long_output_rewrites("cargo test -q"));
    assert!(long_output_rewrites("cargo build -p ra-console"));
}
