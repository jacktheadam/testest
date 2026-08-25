# Aero — Agent Entry Point

> **Project:** Aero — a Windows 7 SP1 (32/64-bit) emulator that runs in the browser.
> **Repo:** greenfield, owned entirely by agents.

**The wiki is the source of truth. Start there:**

- `wiki/state/repo-state-and-structure.md` — **resume the active work here**:
  what is true now, what is verified, and what is still open
- `wiki/README.md` — the index; start here for anything else
- `wiki/overview.md` — the whole system in one page
- `wiki/state/repo-state-and-structure.md` — what the repo is and where it stands
- `wiki/areas/` — one page per subsystem: how it works, where it stands
- `wiki/specs/` — the normative contracts; these are the law
- `wiki/decisions/` — standing architectural decisions, including repo layout,
  host language, and guest driver language
- `wiki/areas/testing.md` — the Win7 milestone ladder and every verification layer
- `wiki/meta/working-agreements.md` — how we work (workflow, doc rules, git
  discipline, verification discipline)
- `wiki/meta/engineering-principles.md` — the adopted engineering constitution

Key operating rules (details in the wiki):

- **No automatic git mutations** (commits/pushes/etc.) without an explicit
  user request.
- **The wiki is the memory**: absorb decisions, findings, and state into it as
  you work — as gardening, not grafting.
- Never claim a gate green without running it; mark unverified things
  `[unverified]`.
- Tests: **make the weird normal** (`wiki/areas/testing.md`) — adversarial
  interaction edges, never volume.
- Natural-language names; no coded task identifiers. Design lives in docs,
  not code comments. Agents run on a headless Linux VM.

## Debugging & verification discipline (how we work)

Bring-up/debugging is **never open-loop**. The loop, proven across the Win7
CPU-bug chain (see `wiki/areas/debugging.md` for the full tooling reference):

1. **Reproduce + capture context first.** The `aero-machine` CLI prints RIP,
   `cs:ip`, linear addr, mode, bytes ±24, stack, GDTR + full GDT on any fault.
2. **Instrument, don't guess.** Windowed instruction trace (`AERO_TRACE_FROM/TO`
   /`AERO_TRACE_COMPACT`), memory-write watchpoints (`AERO_WATCH_WRITE=<addr>` /
   `AERO_WATCH_VALUE=<val>`) and memory-read watchpoints
   (`AERO_WATCH_READ=<addr,...>`), full write-stream (`AERO_WRITE_STREAM=<path>`),
   physical-region dump on any stop (`AERO_DUMP_MEM=<addr>:<len>[,...]`),
   periodic framebuffer dump (`--vga-png-interval`), snapshots for fast resume.
   Firmware-probe traces via env-gated eprintln (e.g. `AERO_PCI_BIOS_TRACE`).
3. **Verify the check *runs*, not just the data.** Before theorizing about what
   a guest validation computes, prove the validation actually happens — a read
   watchpoint on the structure it *should* read. Zero hits means the failure is
   upstream of the check entirely (a probe), which redirects the whole search.
   (This is how the `0xc0000225` wall was reframed: winload never reads the
   ACPI tables before bailing.)
4. **Compare against QEMU ground truth before fixing** — never assume. TCG
   `-d exec` for control-flow traces (KVM produces none), KVM for speed, and
   KVM+GDB hardware break/watchpoints + `monitor pmemsave` for physical RAM.
5. **Fix the rule, not the symptom**, with a regression test derived from the
   finding; re-probe; gate on the relevant test suites before claiming green.
6. **Absorb into the wiki as you go** — the milestone state, every root
   cause→test, surprises, and the method itself. An unwritten lesson is relearned
   by every future session; a written one is inherited by all of them.

Prefer forward first-divergence (diff the write stream / executed addresses
against a reference; the first mismatch is the cause) over long backward chains.

## Resource limits (mandatory)

Use `bash ./scripts/safe-run.sh <command>` for all non-trivial builds/tests —
it enforces timeouts and memory ceilings. Wrap anything long-running with
`timeout -k <grace> <limit>` (always include `-k`). Full guidance:
`wiki/areas/testing.md` (resource limits doctrine) and
`wiki/meta/working-agreements.md`.

```bash
AERO_TIMEOUT=3600 AERO_CARGO_BUILD_JOBS=8 bash ./scripts/safe-run.sh cargo test --workspace --locked
```

Troubleshooting: if `git status` shows mode-only churn or partially-checked-out
files, restore with `git checkout -- .` (careful: discards working-tree
changes). If a `git pull`/`checkout` is interrupted and leaves the tree
half-updated, prefer `git stash` → `git fetch origin && git reset --hard
origin/main` → re-apply — but only with the user's go-ahead.

## Windows 7 test media

A Win7 SP1 x64 ISO lives at `/root/aero-images/` on the dev box (extracted
from the user-supplied archive). **Never commit ISOs or disk images** —
`scripts/ci/check-repo-policy.sh` enforces this.

---

*The sprint-era coordination document that used to live here (workstreams,
milestone tables, document index) was retired with the docs absorption; the
wiki supersedes it. History: `wiki/state/repo-state-and-structure.md`.*
