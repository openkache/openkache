# Contributing to OpenKache

**English** · [한국어](./CONTRIBUTING.ko.md)

OpenKache is a performance-focused cache server, and the work that moves it
forward reflects that: a reproducible bug report, a benchmark that isolates a
regression, a storage or protocol fix, a client binding, or documentation that
helps people run it correctly. Contributions of all of these are welcome.

This guide explains where to ask questions, how to report problems, and how to
get a change reviewed and merged. Please also read the
[Community Guidelines](./COMMUNITY_GUIDELINES.md), which serve as the project's
[code of conduct](./CODE_OF_CONDUCT.md).

## Where to ask questions

For installation, usage, and current support boundaries, start with the
[README](./README.md), the [getting-started guide](./docs/getting-started.md),
and the [FAQ](./docs/faq.md). Most "how do I…" questions are answered there.

GitHub issues are for **reproducible bugs and specific, actionable feature
requests** — not general questions or usage help. Blank issues are disabled;
please use one of the [issue templates](https://github.com/openkache/openkache/issues/new/choose).

## Reporting a bug

Search [existing issues](https://github.com/openkache/openkache/issues) first,
then open a [bug report](https://github.com/openkache/openkache/issues/new/choose)
with the smallest case that reproduces the problem. Include:

- the version or commit, and the platform (Linux kernel and `io_uring`
  availability, macOS preview, or WSL2);
- the exact commands, configuration, and CPU assignment used to start the
  server and drive the load;
- what you expected, what happened, and any server logs or client errors.

**Performance reports get extra scrutiny.** OpenKache makes specific throughput
and latency claims, so a report of a regression or a slow path needs enough to
reproduce the measurement: the hardware, the load tool and its parameters, key
and value sizes, and the numbers you observed. See
[BENCHMARK.md](./BENCHMARK.md) for the methodology we use.

Do not put vulnerability details in a public issue. Report them privately
through a [GitHub security advisory](https://github.com/openkache/openkache/security/advisories/new);
see [SECURITY_MODEL.md](./SECURITY_MODEL.md) for the project's trust boundaries.

## Proposing a change

For a **substantial change** — new storage or wire-format behavior, a protocol
addition, an operational default, or anything that affects compatibility,
performance, or security — open an issue first and agree on the approach before
writing code. Describe the problem and why it matters; a use case is what makes
a feature worth accepting. This saves you from writing a change that has to be
redesigned in review.

**Small documentation fixes and clear, focused bug fixes** can skip that step
and go straight to a pull request.

Either way, keep a change small enough to review and keep unrelated work out of
it. Create a topic branch from `main`, and do not commit credentials, private
data, build output, or generated artifacts that belong to a release process.

## Development setup

OpenKache is a Rust workspace (protocol, server, shared client core, Rust SDK,
CLI, and the native TypeScript adapter) under one lockfile. Building the server
uses the Clang/LLVM toolchain; see the [getting-started
guide](./docs/getting-started.md) for the full environment.

Install the git hooks once per clone:

```bash
./scripts/install-hooks.sh
```

The `pre-commit` hook enforces trilingual documentation sync (English, Korean,
Chinese). If you edit a document that ships in all three languages — such as
`README.md` — the hook rejects the commit unless the `.ko` and `.zh` versions
are updated to match. See [scripts/README.md](./scripts/README.md) for the
registered document sets and the emergency `--no-verify` bypass.

## Checking your work

Run these from the repository root before opening a pull request:

```bash
cargo fmt --all --check
cargo check --locked
cargo test --locked --package openkache-server
```

If you change the TypeScript client:

```bash
bun install --cwd clients/typescript --frozen-lockfile
bun run --cwd clients/typescript build
```

If you change the protocol or generated client contracts, regenerate them and
confirm the snapshots are clean, matching what CI checks:

```bash
OPENKACHE_GENERATION_TARGET=rust-snapshots OPENKACHE_GENERATION_CHECK=1 ./clients/generate.ts
```

If a check cannot run in your environment, say so in the pull request rather
than skipping it silently. Add whatever validation is relevant to the change —
in particular, a reproduction for a bug fix or a measurement for a performance
claim.

## Commit messages

Write a short imperative subject that says what changed. The history uses
[Conventional Commits](https://www.conventionalcommits.org/) prefixes
(`feat`, `fix`, `docs`, `ci`, `chore`, …), often with an optional scope, for
example `fix(server): recover the segment index on restart`. Keep the body for
the reasoning and any tradeoffs.

## Opening a pull request

A pull request that is easy to merge has:

- a short title describing the change;
- a description of the problem, the intended result, and the approach;
- a link to the issue or discussion when one exists;
- the validation commands you ran and their results;
- notes on documentation, compatibility, operational risk, and security when
  they apply.

Draft pull requests are welcome when early feedback would save rework. Review
focuses on correctness, clear behavior, safe operation, and whether the change
solves the problem it set out to solve. Treat review comments as part of the
design: respond to the substance and update the description when the direction
changes.

## Contributor License Agreement

An automated CLA check runs on pull requests. If it asks you to sign, follow
its link before the contribution is merged; it uses the Apache Contributor
License Agreement template. Sign only for work you have the right to submit,
and if you contribute on behalf of an organization, confirm you are authorized
to do so. The CLA works alongside — it does not replace — the license on the
files you change.

## Licensing

Contributions are accepted under the license that applies to the files they
change. The server is licensed under the GNU Affero General Public License
v3.0-or-later; the client SDKs under [`clients/`](./clients/) and the shared
protocol under [`protocol/`](./protocol/) use the Apache License 2.0 stated in
their directories. By submitting a contribution, you confirm you have the right
to submit it under the applicable license.
