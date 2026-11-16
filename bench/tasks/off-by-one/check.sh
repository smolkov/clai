# The tests pass, and no line with an assertion was changed.
cargo test -q --offline >/dev/null 2>&1 || exit 1
! git diff -U0 -- src/lib.rs | grep -E '^[-+][^-+]' | grep -q assert
