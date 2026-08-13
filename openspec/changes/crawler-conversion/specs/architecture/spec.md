# architecture spec — measurement-first conversion workstream

## Scope

Two observability workstreams (failure taxonomy + announce funnel) that run on the stable `--scale 3`, 4-instance, `--min-seen 1` build with no behavioral change.

## Requirements

- Phases 1-2 ship as pure observability: no change to verified/hr or unique/hr semantics (regression check in task 5.3).
- Phase 3 (announce yield fix) and Phase 4 (selectivity tuning) are gated on Phase 1-2 measurements and land as follow-up changes.
- The stable build keeps running during Phases 1-2 so we have a clean memory + performance baseline (`allocated` flat ~113-115 MB, RSS plateau ~135-140 MB) to compare against.

## Acceptance

- Memory trend stays flat during Phases 1-2 (no regression from added counters — atomics only).
- Dashboard (`benchmark/liveness.sh`) exposes the new failure buckets and the announce funnel.
- A written recommendation (from Phase 2 audit + Phase 1 taxonomy) that specifies the single highest-leverage Phase-3 fix.
