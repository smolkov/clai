#!/bin/sh
# Fails when code outside src/startup.rs reads the environment of the
# process (RFC 0026): every command gets it from its Context.
#
#   ci/env-check.sh [DIR]
#
# Test code is exempt: files named tests.rs, files below a tests/ directory,
# and the rest of a file after its first #[cfg(test)].

. "$(dirname -- "$0")/common.sh"

src=${1:-src}
[ -d "$src" ] || fail "$src: not a directory"
pattern='std::env::(var|var_os|vars|vars_os|current_dir|temp_dir|home_dir|set_var|remove_var)\b|(^|[^:a-z_])env::(var|var_os|vars|vars_os|current_dir|temp_dir)\(|EnvFilter::(try_from_env|from_default_env|from_env)'
found=$(find "$src" -name '*.rs' ! -name tests.rs ! -path '*/tests/*' ! -path "$src/startup.rs" | sort |
    while read -r file; do
        sed '/^[[:space:]]*#\[cfg(test)\]/,$d' "$file" |
            grep -nE "$pattern" |
            grep -vE '^[0-9]+:[[:space:]]*//' |
            sed "s|^|$file:|"
    done)
if [ -n "$found" ]; then
    printf '%s\n' "$found" >&2
    fail "the environment is read outside src/startup.rs; take it from the Context (RFC 0026)"
fi
say "the environment is read only in $src/startup.rs"
