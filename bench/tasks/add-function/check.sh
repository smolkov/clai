# Hidden tests against the new function.
mkdir -p tests
cat > tests/bench_median.rs <<'T'
use stats::median;
#[test]
fn median_cases() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
}
T
cargo test -q --offline >/dev/null 2>&1
