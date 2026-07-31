# Contributing to OpenKache

Thanks for taking a look. OpenKache has useful work at several levels: a
reproducible bug report, a documentation fix, a review that catches a risky
assumption, and changes to storage, operations, or client behavior.

## Before you start

Read the [README](./README.md) and [roadmap](./README.md#roadmap) first. Search
[existing issues](https://github.com/openkache/openkache/issues) before opening
another one.

For a bug, include the version or commit, environment, reproduction steps, and
what you expected to happen. For a feature or behavior change, describe the
problem and the reason it matters. Open an issue before starting a substantial
change so the scope and direction can be discussed. Small documentation fixes
and clear bug fixes can go straight to a pull request.

## Prepare a change

Keep a change focused enough to review. Explain the important tradeoffs and
call out effects on compatibility, performance, security, and operations. Keep
user-facing documentation in step with behavior.

Do not commit credentials, private data, build output, or generated files that
belong in a release process. If a change needs a new dependency or a different
storage or wire-format contract, explain why in the issue and pull request.
Create a topic branch from `main` and keep unrelated work out of the pull
request.

## Check your work

Run these commands from the repository root before opening a pull request:

```bash
cargo fmt --all --check
cargo check --locked
```

If you change the TypeScript client, also run:

```bash
bun install --cwd clients/typescript --frozen-lockfile
bun run --cwd clients/typescript build
```

If a check cannot run in your environment, say so in the pull request. Include
any additional validation that is relevant to the change, especially a
reproduction or measurement for a bug or performance claim.

## Open a pull request

A useful pull request has:

- A short title that says what changed.
- A description of the problem, the intended result, and the approach.
- A link to the issue or discussion when one exists.
- Validation commands and their results.
- Notes about documentation, compatibility, operational risk, and security
  when they are relevant.

Draft pull requests are welcome when early design feedback would save rework.
Expect review to focus on correctness, clear behavior, safe operation, and
whether the change solves the problem it set out to solve. Review comments are
part of the design process; respond to the substance and update the
description when the direction changes.

## Licensing

Contributions are accepted under the license that applies to the files they
change. The server is licensed under the GNU Affero General Public License
v3.0-or-later; the client SDKs and shared protocol use the licenses stated in
their directories. By submitting a contribution, you confirm that you have the
right to submit it under the applicable license.

For behavior and reporting expectations, see the
[Community Guidelines](./COMMUNITY_GUIDELINES.md).
