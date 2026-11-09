# RFCs

An RFC (request for comments) proposes a change to `clai` before it is
built: a new feature, a milestone, or a decision the spec leaves open. It
gives one place to agree on the direction, so the work that follows is
predictable for everyone involved.

## Place among the project files

| File | Says |
|------|------|
| [`spec/`](../spec/readme.md) | What `clai` does. Source of truth for behaviour. |
| [`implementation.md`](../implementation.md) | How it is built, and in which order (milestones). |
| `rfcs/` | How one step is built, and which decisions it makes. |
| [`todo.md`](../todo.md) | What is done and what is left. |

An RFC never overrides the spec. A decision about behaviour is written into
the spec file of its topic, in the same change as the RFC or when the RFC is
accepted; the RFC records why.

## When to write one

- Before a milestone of `implementation.md`, or a large part of one.
- For a change of behaviour that touches several spec files, the protocol
  or the saved session format.
- For a new dependency, or a choice between designs with real trade-offs.

Not needed for bug fixes, refactoring without behaviour change, tests, or
wording in the spec. The plan the agent writes before each task (see
[`agent.md`](../agent.md), "Workflow") is enough there.

## Process

1. Write `rfcs/NNNN-short-name.md` from the [template](#template), with
   status `draft`. `NNNN` is the next free number, four digits; the
   first line of the list under the title repeats it: `- RFC: NNNN`.
2. Discuss and change it until the open questions are answered or moved
   out of scope.
3. The user accepts it: status `accepted`. Changes after that need a new
   approval and a note in the RFC.
4. Build it, step by step as its implementation plan says. When done,
   status `done`, and the milestone in `implementation.md` and `todo.md`
   are updated.

An RFC that will not be built gets `rejected` or `replaced by NNNN`, and
stays in the directory with its reason.

## Statuses

| Status | Meaning |
|--------|---------|
| `draft` | Being written or discussed. |
| `accepted` | Approved; may be built. |
| `done` | Built and merged. |
| `rejected` | Will not be built; the RFC says why. |
| `replaced by NNNN` | Another RFC took its place. |

## Template

```markdown
# Title

- RFC: NNNN
- Status: draft
- Milestone: name in implementation.md, if any
- Spec: spec files and sections it is based on or changes

## Summary

One paragraph: what changes.

## Motivation

Why now; what is missing or broken without it.

## Scope

In and out of this RFC.

## Design

Modules, types, data flow, decisions. Mark choices the spec does not make
with **(proposal)**.

## Dependencies

New crates and why (omit if none).

## Spec changes

Which spec files change and how.

## Testing

What is tested and how.

## Implementation plan

Steps in build order. Each step leaves a working, tested state. Its
status is `todo`, `in progress` or `done`; it is updated in the same change
that builds the step, and a step is `done` only when its tests pass.

1. **Name**
   - Status: todo
   - Files: Files created or changed
   - Result: What works after it
   - Tests: What is tested

## Alternatives

Other designs and why they were not chosen.

## Open questions

Decisions still needed from the user.
```

Write like the spec: short sentences, plain words, no numbers in headings.
Sections that do not apply may be left out.

## Index

| RFC | Title | Status |
|-----|-------|--------|
| [0001](0001-tui-core.md) | TUI core | accepted |
| [0002](0002-clai-without-command.md) | `clai` without a command opens the TUI | accepted |
| [0003](0003-session-improvements.md) | Session improvements | done |
| [0004](0004-tui-command-list.md) | TUI command list | done |
| [0005](0005-session-switching.md) | Session switching | done |
| [0006](0006-tui-settings.md) | TUI settings | done |
| [0007](0007-status-view.md) | Status view | done |
| [0008](0008-trim-old-tool-results.md) | Trim old tool results | draft |
| [0009](0009-command-layer-refactoring.md) | Refactoring the command layer | done |
| [0010](0010-security-hardening.md) | Security hardening | done |
| [0011](0011-session-list-delete-and-filter.md) | Session list: delete, marks and filter | draft |
