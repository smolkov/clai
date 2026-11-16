#!/bin/sh
# Token bench (RFC 0028): runs fixed tasks with `codeagent run` against a
# real model and prints, per task, whether it was done and what it used.
# Needs a configured model and costs money; run by hand, not in pipelines.
#
#   ci/bench.sh [--model PROVIDER/MODEL] [--runs N] [TASK...]
#
# Tasks are the directories of bench/tasks (all when none is named). Output:
# one tab-separated line per run, after a header:
#
#   task  run  ok  requests  input  cached  output
#
# $CODEAGENT is the binary to run (default: target/release/codeagent, built
# when missing). $CODEAGENT_BENCH_DIR keeps the workspaces there instead of
# removing them. $CODEAGENT_BENCH_TASKS replaces bench/tasks (tests).

. "$(dirname -- "$0")/common.sh"

model=""
runs=1
while [ $# -gt 0 ]; do
    case $1 in
        --model) [ $# -ge 2 ] || fail "--model needs a value"; model=$2; shift 2 ;;
        --runs) [ $# -ge 2 ] || fail "--runs needs a value"; runs=$2; shift 2 ;;
        --*) fail "unknown option $1" ;;
        *) break ;;
    esac
done
case $runs in
    '' | *[!0-9]* | 0) fail "--runs must be a number from 1" ;;
esac

tasks=${CODEAGENT_BENCH_TASKS:-$ROOT/bench/tasks}
if [ $# -eq 0 ]; then
    set -- $(ls "$tasks")
fi
for task in "$@"; do
    [ -f "$tasks/$task/task.md" ] || fail "no task $task in $tasks"
done

bin=${CODEAGENT:-}
if [ -z "$bin" ]; then
    bin=$ROOT/target/release/codeagent
    [ -x "$bin" ] || cargo build --release --locked >&2
fi

work=${CODEAGENT_BENCH_DIR:-}
if [ -z "$work" ]; then
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT INT TERM
fi

# number KEY FILE: the first `"KEY":N` in a JSON file, or 0.
number() {
    n=$(tr -d '\n ' <"$2" | sed -n "s/.*\"$1\":\([0-9][0-9]*\).*/\1/p" | head -n 1)
    printf '%s' "${n:-0}"
}

printf 'task\trun\tok\trequests\tinput\tcached\toutput\n'
for task in "$@"; do
    i=1
    while [ "$i" -le "$runs" ]; do
        ws=$work/$task-$i
        rm -rf "$ws"
        cp -R "$tasks/$task/repo" "$ws"
        (
            cd "$ws"
            git init -q
            git add -A
            git -c user.name=bench -c user.email=bench@localhost commit -qm start
        )
        if ! "$bin" run --yes --report=json -C "$ws" ${model:+--model "$model"} \
            <"$tasks/$task/task.md" >"$work/$task-$i.json" 2>"$work/$task-$i.err"; then
            say "$task run $i: codeagent failed: $(tail -n 3 "$work/$task-$i.err")"
        fi
        if (cd "$ws" && sh "$tasks/$task/check.sh") >/dev/null 2>&1; then
            result=yes
        else
            result=no
        fi
        report=$work/$task-$i.json
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$task" "$i" "$result" \
            "$(number requests "$report")" "$(number input_tokens "$report")" \
            "$(number cache_read_tokens "$report")" "$(number output_tokens "$report")"
        i=$((i + 1))
    done
done
