# Public package releases

The public repository keeps release automation manual and tag-bound. A release
operator creates a reviewed version change on `main`, pushes the package tag,
and dispatches exactly one workflow from that tag. Each release workflow requires the
literal `RELEASE` confirmation input and a protected environment approval; the
release workflows in this guide do not run from a branch push, pull request,
schedule, or development checkout.

## Operator inputs

Dispatch the workflow from the matching tag and enter the version copied from
the package metadata. The workflows fail before building if the tag, version,
commit, or Git worktree is inconsistent.

| Workflow | Tag | Version source | Confirmation |
| --- | --- | --- | --- |
| `Release TypeScript npm package` | `typescript-v<version>` | `clients/typescript/package.json` | `RELEASE` |
| `Release Python PyPI package` | `python-v<version>` | `clients/python/pyproject.toml` | `RELEASE` |
| `Release openkache-server crate` | `server-v<version>` | `server/Cargo.toml` | `RELEASE` |
| `Build C and C++ CMake archives` | `cmake-v<version>` | both CMake project files | `RELEASE` |

The CMake workflow creates source archives and does not upload to a package
registry. It still uses a protected `cmake-release` environment so an operator
can review the tag and checksums before sharing the Actions artifact.

## Required repository setup

Create protected environments named `npm-release`, `pypi-release`,
`crates-io-release`, and `cmake-release`. Require at least one reviewer for
each environment and restrict deployment branches/tags to the corresponding
release tag pattern. Configure only the credentials needed by the target
registry:

- `npm-release`: `NPM_TOKEN` repository secret. The workflow exposes it only as
  `NPM_CONFIG_TOKEN`; it is never written to a file or command line.
- `pypi-release`: a PyPI trusted publisher for this repository, workflow
  (`.github/workflows/publish-pypi.yml`), and environment. The workflow uses
  the short-lived GitHub OIDC exchange and stores no PyPI token.
- `crates-io-release`: `CARGO_REGISTRY_TOKEN` repository secret. Keep the token
  scoped to the `openkache-server` owner and rotate it outside this repository.
- `cmake-release`: no credential; the workflow only uploads an Actions
  artifact.

Do not enable a release workflow merely to test it. Review the generated
artifact with a manual run from a disposable tag or run the package-local
dry-run commands in the package documentation without supplying registry
credentials.

## Provenance and checksums

Each build records `source_commit`, `source_tag`, and `package_version` in
`RELEASE-METADATA`. It writes a `SHA256SUMS` file for every staged package,
crate, native adapter, or CMake archive. The publication job downloads those
artifacts, verifies the metadata and checksums, checks the complete expected
file set, and only then calls the existing package release command.

The source tag is the provenance boundary: the published npm package, PyPI
distributions, and crate are rebuilt from the checked-out commit named by that
tag. CMake archives use `git archive` with deterministic gzip timestamps and
carry the same commit metadata. Actions artifact retention is not a registry
backup; copy the checksums and release metadata into an approved release record
if long-term auditability is required.

## Recovery and rollback limits

Stop a run by rejecting the protected-environment approval or cancelling it
before the registry command starts. A failed preflight does not publish
anything. Once npm, PyPI, or crates.io accepts an artifact, that version is
immutable:

- npm versions cannot be overwritten; deprecate the version and publish a
  corrected higher version, then move the dist-tag if needed.
- PyPI files cannot be replaced or deleted through this workflow; yank the
  release where PyPI permits it and publish a corrected higher version.
- crates.io versions cannot be republished; yank the crate where policy permits
  it and publish a corrected higher version.

There is no registry rollback operation and this automation does not pretend
otherwise. The CMake Actions artifact can be deleted or allowed to expire, but
that does not retract archives already downloaded by consumers.

## Package-local dry runs

Use the package-local commands before creating a release tag. They do not
publish when credentials are absent:

```bash
# TypeScript
cd clients/typescript
bun install --frozen-lockfile
bun run typecheck
bun run release:dry-run

# Python
cd clients/python
python -m build --sdist --wheel --outdir dist
python -m twine check dist/*

# Server crate
cargo package --manifest-path server/Cargo.toml --locked --no-verify

# C and C++ CMake smoke checks
cmake -S clients/c -B _local/cmake-c
cmake -S clients/cpp -B _local/cmake-cpp
```

The `_local/` CMake directories are transient and must not be committed.
