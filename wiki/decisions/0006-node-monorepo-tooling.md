# Node monorepo tooling

**Decision (0006).** One JavaScript monorepo managed by **pnpm workspaces**, with
`pnpm-lock.yaml` as the only lockfile and `pnpm-workspace.yaml` as the only
workspace declaration.

> Supersedes the original form of this decision, which chose npm workspaces with
> a single root `package-lock.json`. That reasoning and the conditions that
> changed it are recorded below, because a decision that hides its own history
> cannot be re-examined.

## Context

The repository holds many JavaScript packages — the browser host, two network
services, several tools, the benchmark harness, shared libraries. Each once
carried its own lockfile and was installed independently, which produced version
drift within one repo (several TypeScript and Playwright versions at once), slow
repeated installs, and a confusing local workflow where the right directory to
run an install in depended on what you were doing.

One workspace, one lockfile, one install fixes all three. The only real question
is which package manager.

## Decision

pnpm, because two of its properties matter here specifically:

**Filtering by path.** Scripts address packages as `pnpm -C <path> run <script>`,
which stays correct when a package moves. A workspace-name-based invocation
silently stops resolving when the declaration it depends on changes — and the
failure surfaces far from the change that caused it.

**An isolated `node_modules`.** Only declared dependencies are reachable, so a
package cannot accidentally rely on something a sibling happened to hoist. The
cost is that the layout is unfamiliar: the real packages live in
`node_modules/.pnpm` and the top level holds only a handful of links, which
looks like a broken install to anyone expecting npm's flat tree. That
misreading is common enough to be worth stating in the state page.

Membership is declared once, in `pnpm-workspace.yaml`: the browser host, the two
services under `services/`, the tools that carry their own package manifests,
`packages/*`, and the benchmark harness.

The runtime is pinned in `.nvmrc` and enforced by `scripts/check-node-version.mjs`;
every manifest's `engines` range matches that pin. A mismatch is a warning by
default and an error under `AERO_ENFORCE_NODE_MAJOR`.

## What changed from the npm decision

The original decision was sound on its own terms — npm workspaces are adequate,
the repo already used npm, and it integrated with the CI and bot-update
configuration of the time. Two of those three supports have since gone:

- **The CI and bot integration no longer exists.** The cleanup doctrine purges
  and bans CI automation and bot dependency bumps, so "integrates cleanly with
  existing CI and dependabot" stopped being an argument for anything.
- **The target architecture chose pnpm on its own research**, alongside Node 24,
  for install speed and for path-scoped command filtering.

What remained was migration cost — and a half-finished migration is worse than
either end state. For a period this repo carried *both* lockfiles and *both*
workspace declarations, `engines` pinned a Node major that nothing used, and the
npm workspace invocations broke the moment the npm workspace declaration was
removed.

## Consequences

- `pnpm install` at the root is the only supported install.
- Package-scoped commands use `pnpm -C <path> run <script>`. The npm workspace
  flag does not work here and must not reappear.
- `xtask` spawns pnpm; the `justfile` installs with
  `pnpm install --frozen-lockfile`.
- Shared tooling versions are aligned by construction.
- The single-package-manager property is enforced by a test, not left to
  convention: `tests/repo_hygiene.contract.test.js` fails if a second lockfile
  appears, if `package.json` reintroduces a `workspaces` key, or if any script
  invokes npm.

## Alternatives

**Yarn Berry** — strong workspaces, constraints, plug'n'play. Rejected: a larger
behavioural change than the problem justifies, with no advantage over pnpm for
this repo's shape.

**Staying on npm workspaces** — zero migration cost, and the status quo until the
toolchain research that produced this decision. Rejected once its supporting arguments lapsed;
adequate is not a reason to stay when the move is already half-made.
