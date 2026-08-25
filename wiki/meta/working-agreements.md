# Working Agreements (meta)

> Canonical home of the repo's working agreements — moved here from
> the working agreements (2026-07-23, the meta-docs location decision). A thin
> pointer remains at `AGENTS.md` (root) for harness auto-injection.
>
> The engineering-principles constitution lives in
> [engineering-principles.md](./engineering-principles.md).

## Part 1 — Repo facts & working agreements

### 1.1 Ownership

- **This is a greenfield repository owned entirely by agents.** There are no
 human-owned zones, no third-party contribution queue, and no external
 maintainers to route around. Act with **full ownership** everywhere in the
 tree (see Part 2, "Zones and authority").
- Full ownership does not suspend §1.3: git *mutations* still happen only on
 explicit user request. Ownership of the code is not ownership of the user's
 git history.

**What this repo is.** A Windows 7 emulator that runs in the browser: the Rust
emulator core, the browser host, the Windows guest drivers, and the minimum
services and tooling needed to develop and run those. Everything else is guilty
until proven innocent.

It is an **agent-owned engineering project, not a simulated open-source
foundation**. Anything performing *project theater* — presenting the repo as a
governed public project with users, contributors, legal exposure, and release
infrastructure it does not have — is cruft, because it misleads every future
reader about what this repo actually is. Equally, anything outside the project's
scope justifies its existence or goes, however well built it is: **noise is not
neutral**, and every file a future reader must read past is a tax on
comprehension.

**The ban list.** These are purged *and* forbidden to return:

1. **CI/CD automation** — workflow directories, action definitions, code
   scanning config, dependency-bot config, coverage services. Verification here
   is run-by-agent, not run-by-platform.
2. **Bot dependency bumps.** Dependency updates are deliberate, agent-executed
   migrations. Six-month-stale bot branches caused real breakage here once
   already.
3. **Project-theater files** — codes of conduct, contributing guides, security
   disclosure pages, DMCA/terms/privacy/trademark templates, authors and notice
   roll-calls, governance documents, licence files.
4. **Production-ization configs for imaginary deployments** — Terraform, k8s
   manifests, CDN configs, multi-host compose stacks, release pipelines. Local
   dev serving configs for the emulator itself are fine and stay.
5. **Duplicate stacks** — once a legacy parallel is purged it does not come
   back: a second device stack, a second gateway, a second AeroGPU executor, a
   second documentation tree outside `wiki/`.

This is a check rather than a convention: `tests/repo_hygiene.contract.test.js`
fails if a banned path reappears, if a second documentation tree or lockfile
appears, if `package.json` reintroduces a `workspaces` key, or if a script
invokes npm. It earned its place immediately by catching a script the pnpm
migration had missed.

**Design first, from first principles.** When restructuring, design the target
from what the project actually needs and what the modern form of each component
is — not by incrementally reshaping whatever happens to be in the tree, and not
from a training-snapshot memory of how these tools worked. Check current versions
and current practice; the tree's existing shape is evidence about history, not
about what is right.

**Absorb, not bulldoze — for code as well as docs.** Nothing is mass-deleted.
When a duplicate is resolved the superior implementation is picked and the
other's unique content is merged into it *first*; only residue is purged, and it
leaves a graveyard note. A purge list is a verdict about where things should
live, never a shortcut around reviewing them. Removed material goes to a
gitignored `.attic/` rather than being destroyed — that is removal from the
project, not destruction, and emptying it is a separate decision to take once
nobody has wanted anything back for a while.

### 1.2 Source of truth & workflow

1. **`wiki/` is the living source of truth**, and the only documentation tree —
 a second one is a hygiene-test failure. Where it disagrees with the root
 `AGENTS.md` or with a comment in the code, fix the disagreement rather than
 leaving both standing.
2. **Absorb as you work — the wiki is the memory.** Every investigation,
 probe, decision, discovery, surprise, and state change is folded into the
 relevant wiki doc *in the same breath* as the work itself — never left in
 chat, never left as untracked knowledge. Absorption means **gardening**:
 integrate new material into the existing structure so the whole stays
 coherent — no grafted-on fragments, no duplicate or overlapping pages, no
 bulldozed context. If a fact belongs on an existing page, extend that
 page; a new page needs durable scope and an index link. The conversation
 is ephemeral; if it isn't in the wiki, it didn't happen.
3. **Docs before action.** For cleanup/modernization work, iterate on
 `wiki/**` first: a decision is recorded in the wiki *before* (or as) it is
 executed. No silent rewrites of reality.
4. **Evergreen docs.** Write every doc so it stays true: present tense,
 current fact. Dated entries go only into designated "History / decision
 log" sections, which are append-only. When reality changes, update the
 affected section in the same change — never leave a doc describing last
 month's state as if it were today's.
5. **Coherence over coverage.** Extend the existing doc when the content
 belongs to it. Do not scatter small disjoint notes; a new doc needs a
 durable scope and a link from `wiki/README.md`.
6. **Verify or mark.** Every claim either cites a path in the tree or is
 marked **[unverified]**. Aspirational content must be labeled as such.
7. **Label canon.** Mark components *canonical*, *active*, *legacy*,
 *historical/tombstone*. Never present legacy as current.
8. **Graveyard practice.** When something is removed, superseded, or
 deliberately not carried, record what it was, why, and where its content
 lives on (commit, `refs/pull/*`, or nowhere) — in the same wiki edit.
9. **Fallout home.** Content that falls out during absorption but is still
 interesting (ideas, sketches, speculative designs, "maybe someday") goes to
 `wiki/notes/` — clearly labeled as non-normative — rather than being
 deleted or left in the tree.
10. **Natural-language names only.** No coded identifiers as primary names —
 no task codes (`A1`, `W2`, `PF-008`), no decision IDs (`D4`), no numbered
 workstreams — in code, tests, filenames, docs, or the wiki. Named things
 get natural-language names and are referenced by them ("the L2-tunnel
 networking decision", "the guest-CPU benchmark suite"). Navigational
 numbering (ordered lists, section numbers, dates) is fine; existing
 numbered source docs (ADRs) are referenced by title, with the number kept
 only as an alias.
11. **Design lives in docs, not in code comments.** Code comments are terse
 why-notes at hazard sites (see Part 2, "Knowledge"). Larger design
 discussion belongs in `wiki/`; a comment may point at a wiki topic **by
 natural-language name** ("see wiki: storage trait consolidation") —
 never by URL or fragile path. During absorption, oversized design
 comments move to the wiki and leave a one-line pointer behind.
12. **Runtime environment: always a headless Linux VM.** No GUI, no Windows,
 no browser UI on the box. Anything requiring Windows (WDK driver builds),
 a display, or non-headless interaction is marked as such and deferred or
 delegated; browser tests run headless.

### 1.3 Git discipline

- **No automatic git mutations.** Do not `git commit`, `push`, `reset`,
 `rebase`, `merge`, delete branches, or otherwise mutate git state unless the
 user explicitly asks for that specific action. (User directive, 2026-07-22.)
- When asked to commit: follow the repo's conventional-commit style
 (`type(scope): subject`), no backticks in `-m` messages, keep commits atomic
 and reviewable.
- Report git state honestly: what is committed, what is only in the working
 tree, what is unpushed.

### 1.4 Verification discipline (repo bindings)

- **Never claim green without running the gate.** Rust gates:
 `bash ./scripts/safe-run.sh cargo check --workspace --locked` and
 `cargo test --workspace --locked --no-run`. There is one Rust workspace;
 `fuzz/` is the only standalone one and gates separately.
- JS/TS gates: `pnpm -w run typecheck` and `pnpm -w run test:unit`, under the
 pinned Node (24) and pnpm (11) — **never npm**, which the hygiene contract test
 rejects. The pinned toolchain is invisible until activated; if the environment
 cannot provide it, say the area is unverified rather than implying it passed.
 See [../areas/build-and-tooling.md](../areas/build-and-tooling.md).
- Fix-forward, honestly: if a merge/bump breaks the build, either fix it and
 re-run the gate, or revert it and record the decision in the wiki. No red
 main presented as green.
- Use `scripts/safe-run.sh` (timeout + memory limit) for all non-trivial
 builds/tests, per the root `AGENTS.md`.
- **Make the weird normal** when adding tests (`wiki: testing`): adversarial
 depth at interaction edges, never a pad of same-path cases. See
 [engineering-principles.md](./engineering-principles.md) (Testing policy).
- **Evidence a claim rests on does not live in scratch space.** This rule was
 bought at a real price: a large body of bring-up findings cited proof captures,
 screenshots and snapshots under a temporary directory, and when that directory
 went away every claim whose only support was in it became unverifiable again.
 The claims had been true when written; they were no longer *checkable*, which
 for a wiki that must be trustworthy is nearly the same thing. Durable artefacts
 go in a durable location — snapshots under `$AERO_IMAGES_DIR/snapshots/` — and
 a proof capture small enough to inline belongs in the page that cites it. A
 verification that can evaporate is not a verification, it is a memory.
- **Distinguish what the system did from what you made it do.** Bring-up work
 leans on scaffolding — memory pokes, stubbed functions, forced flags, injected
 input, skipped waits. A result obtained under scaffolding answers a genuine
 question ("if this were right, would it proceed?") but it is *not* evidence the
 guest reaches that state by itself. Record which scaffolding was in play, and
 record separately what has been reproduced without any. When these blur, a
 project can believe it is further along than it is.

### 1.5 Engineering style (repo bindings)

- Minimal, reviewable diffs; match surrounding code conventions; no drive-by
 refactors; finish what you start.
- Seasoned-engineer tone in docs and replies: candid about state, no cheer
 language, no flattery.
- When reality and a doc disagree, reality wins — then fix the doc.

### 1.6 Current canon (quick reference)

- Source of truth: `wiki/` (start: `wiki/state/repo-state-and-structure.md`)
- Canonical VM: `aero-machine` (the canonical VM core decision) · Canonical
 host: `apps/web/` (the repo-layout decision) · Canonical backend:
 `services/gateway`
- Legacy, do not extend: `crates/legacy/aero-d3d9-shader` (kept only because the
 fuzz workspace uses it) and `apps/web/bringup.html` (the manual Windows 7
 bring-up shell, which goes when the canonical shell can do that job). The other
 legacy parallels — the old server tree, the old gateway, the second device
 stack, the prototype and prototype-adjacent directories — are **purged**, and
 the ban list above stops them coming back; the inventory is in
 [../history/retirements.md](../history/retirements.md).

---
