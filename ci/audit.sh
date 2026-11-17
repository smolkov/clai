#!/bin/sh
# Dependency audit (RFC 0037): known advisories of the RustSec database,
# licences, banned crates and sources, with `cargo deny`; and the review
# dates of ignored advisories.
#
#   ci/audit.sh               the whole audit
#   ci/audit.sh dates [FILE]  only the review dates (FILE: deny.toml)
#
# Run it when Cargo.lock or deny.toml changes; the pipelines run it daily
# and before every release. CODEAGENT_TODAY (YYYY-MM-DD) replaces today's
# date, for tests.

. "$(dirname -- "$0")/common.sh"

# Each `ignore` entry of FILE has a reason ending in `review YYYY-MM-DD`,
# and that date has not passed.
review_dates() {
    file=$1
    [ -f "$file" ] || fail "$file not found"
    today=${CODEAGENT_TODAY:-$(date +%F)}
    problems=$(awk -v today="$today" '
        /^\[/ { in_adv = ($0 == "[advisories]") }
        in_adv && /id *= *"/ {
            id = $0; sub(/.*id *= *"/, "", id); sub(/".*/, "", id)
            if (match($0, /review [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]/)) {
                date = substr($0, RSTART + 7, 10)
                if (date < today) print id ": review date " date " has passed; check the reason, then move the date or remove the entry"
            } else {
                print id ": the reason has no review date (end it with `review YYYY-MM-DD`)"
            }
        }' "$file")
    if [ -n "$problems" ]; then
        printf '%s\n' "$problems" | while IFS= read -r line; do
            printf 'ci: error: %s\n' "$line" >&2
        done
        fail "ignored advisories need a review"
    fi
}

case "${1:-}" in
    dates)
        review_dates "${2:-deny.toml}"
        say "review dates ok"
        ;;
    "")
        review_dates deny.toml
        command -v cargo-deny >/dev/null 2>&1 \
            || fail "cargo-deny is not installed: cargo install --locked cargo-deny@$CARGO_DENY_VERSION"
        say "cargo deny"
        cargo deny --locked check advisories bans licenses sources
        ;;
    *) fail "usage: ci/audit.sh [dates [FILE]]" ;;
esac
