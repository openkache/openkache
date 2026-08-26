# Release OpenKache client packages

This guide covers the Python package on PyPI, the Rust crate on crates.io,
and the TypeScript package on npm. Every registry version is immutable.

The release has two phases:

1. **Prepare the release** — build and inspect everything, create the package
   tags, and dispatch the workflows. This phase stops while each workflow is
   waiting for its protected-environment approval.
2. **Publish the release** — review the staged artifacts, approve the
   protected environment, and verify the published versions.

Do not dispatch a workflow until its protected environment requires a reviewer.
Without that gate, dispatching can publish immediately.

The workflow file is read from the selected `--ref`, not from `main`. Merge
workflow fixes before creating release tags. If a tag already exists and its
version is still unused, move the tag only after confirming all three registry
checks return `404`; never move a tag after a registry has accepted that
version.

## Current release

The currently staged client versions are:

| Package | Version source | Version | Tag | Workflow |
| --- | --- | --- | --- | --- |
| Python | `clients/python/pyproject.toml` | `0.1.3` | `python-v0.1.3` | [`publish-pypi.yml`](.github/workflows/publish-pypi.yml) |
| Rust | `clients/rust/Cargo.toml` | `0.1.3` | `client-v0.1.3` | [`publish-crates.yml`](.github/workflows/publish-crates.yml) |
| TypeScript | `clients/typescript/package.json` | `0.1.3` | `typescript-v0.1.3` | [`publish-npm.yml`](.github/workflows/publish-npm.yml) |

All three client packages use the same version. By default, increment the
shared version by `0.0.1` for the next release. If the change is breaking,
stop and ask the user to choose the version before updating any manifest or
tag. Never reuse a version that was published, even if it was later yanked.

## 1. Prepare the release (stop before publication)

All commands in this section run from a clean checkout of the public
`openkache` repository, not from the private monorepo.

### 1.1 Check the source and versions

Start from the public `main` branch after the client changes have been merged:

```bash
git fetch origin main --tags
git switch main
git pull --ff-only
test -z "$(git status --porcelain=v1 --untracked-files=all)"
```

Confirm that the release workflow files on `origin/main` are the ones you
intend to run before creating tags:

```bash
git show origin/main:.github/workflows/publish-pypi.yml >/dev/null
git show origin/main:.github/workflows/publish-crates.yml >/dev/null
git show origin/main:.github/workflows/publish-npm.yml >/dev/null
```

The release commit is the provenance boundary. Do not build or publish from a
feature branch, detached checkout, or dirty worktree. If the Rust version
changed, `Cargo.lock` and the docs.rs URL in `clients/rust/src/lib.rs` must be
updated in the same reviewed change.

Before creating tags, confirm the manifest versions and that the registry
versions are still unused:

```bash
python3 - <<'PY'
import json
import pathlib
import tomllib

print("python", tomllib.loads(
    pathlib.Path("clients/python/pyproject.toml").read_text()
)["project"]["version"])
print("rust", tomllib.loads(
    pathlib.Path("clients/rust/Cargo.toml").read_text()
)["package"]["version"])
print("typescript", json.loads(
    pathlib.Path("clients/typescript/package.json").read_text()
)["version"])
PY

check_unused() {
  local url="$1"
  local label="$2"
  local status
  status="$(
    curl --silent --show-error --location --retry 2 --retry-all-errors \
      --connect-timeout 10 --max-time 30 \
      --user-agent "openkache-release-preflight" \
      --output /dev/null --write-out '%{http_code}' "$url"
  )"
  case "$status" in
    404) echo "$label is available" ;;
    200) echo "$label is already published" >&2; return 1 ;;
    *) echo "$label registry check returned HTTP $status" >&2; return 1 ;;
  esac
}

check_unused \
  "https://pypi.org/pypi/openkache/0.1.3/json" \
  "PyPI openkache@0.1.3"
check_unused \
  "https://registry.npmjs.org/openkache/0.1.3" \
  "npm openkache@0.1.3"
check_unused \
  "https://crates.io/api/v1/crates/openkache/0.1.3" \
  "crates.io openkache@0.1.3"
```

Before dispatching, confirm that `pypi-release`, `crates-io-release`, and
`npm-release` exist as protected GitHub environments with required reviewers.
Also confirm the registry credential for the package you are about to publish:
PyPI and npm use Trusted Publishing; crates.io needs its bootstrap token for
the first publication and can use Trusted Publishing after that first version
exists. See [Review and credentials](#21-review-the-run-and-credentials).

If any check returns `200`, choose the next unused shared patch version and
update all three manifests, tags, and workflow inputs together.

### 1.2 Build and inspect the packages

Keep generated artifacts outside the repository or in ignored directories.
The commands below validate the artifacts that can be built on the current
host:

```bash
set -euo pipefail

# Rust: verify metadata and inspect the crate archive.
cargo metadata \
  --manifest-path clients/rust/Cargo.toml \
  --locked \
  --no-deps \
  --format-version 1
cargo package --manifest-path clients/rust/Cargo.toml --locked
tar -tzf target/package/openkache-0.1.3.crate \
  | grep -Fx "openkache-0.1.3/Cargo.toml"
tar -tzf target/package/openkache-0.1.3.crate \
  | grep -F "openkache-0.1.3/src/lib.rs"

# Python: build an sdist and the wheel for the current host.
python3 -m venv /tmp/openkache-release-venv
/tmp/openkache-release-venv/bin/python -m pip install \
  --disable-pip-version-check --upgrade build twine
rm -rf /tmp/openkache-python-dist
mkdir -p /tmp/openkache-python-dist
(
  cd clients/python
  /tmp/openkache-release-venv/bin/python -m build \
    --sdist --wheel --outdir /tmp/openkache-python-dist
)
/tmp/openkache-release-venv/bin/python -m twine check \
  /tmp/openkache-python-dist/*

# TypeScript: build JavaScript, declarations, native adapters, and the archive.
(
  cd clients/typescript
  bun install --frozen-lockfile
  bun run build
  bun run typecheck
  bun run build:native
  bun pm pack --dry-run
)
```

The release workflows build the complete platform matrix on clean GitHub
hosted runners:

- Python: one sdist plus Linux x86_64/aarch64, macOS x86_64/arm64, and Windows
  x86_64/arm64 wheels.
- Rust: one `openkache-<version>.crate` archive.
- TypeScript: JavaScript and declarations plus Linux x64/arm64 and Apple
  Silicon macOS native adapters.

Python wheel jobs compile the native adapter from the generated Rust snapshots
already included in the source distribution. They intentionally remove the
repository generator inputs before compiling, so the cross-platform release
matrix does not require Bun or the Smithy CLI on every runner.

Linux can build the two Linux TypeScript adapters locally. The Darwin adapter
is built on Apple Silicon macOS, and the Python macOS/Windows wheels are built
by their workflow matrix. A local TypeScript `release:dry-run` is therefore a
host check; the dispatched workflow is the complete cross-platform check.

After collecting all platform artifacts, the workflow runs the final checks
again on its tagged checkout:

- Python runs `twine check` over one sdist and six wheels.
- Rust recreates and inspects the exact crate archive.
- TypeScript assembles all three native adapters, runs `release:verify`, and
  runs `release:smoke`.

Each workflow records `RELEASE-METADATA` (source commit, source tag, and
package version) and `SHA256SUMS` for the artifacts. Keep both files with the
release review.

### 1.3 Tag the source and queue the workflows

Only tag the commit that is on public `main` and passed the checks above. Make
sure none of these tags already exists:

```bash
git tag python-v0.1.3
git tag client-v0.1.3
git tag typescript-v0.1.3
git push origin python-v0.1.3 client-v0.1.3 typescript-v0.1.3
```

If a release workflow was fixed after a tag was created, first rerun the
registry checks above. Only when the version is still unused may you move that
unused tag to the fixed `main` commit:

```bash
git tag -f python-v0.1.3 <fixed-main-commit>
git push --force-with-lease origin <fixed-main-commit>:refs/tags/python-v0.1.3
```

Repeat for the other client tags as needed. Never move a tag after its registry
has accepted the version; create the next shared patch release instead.

Dispatch one workflow for each package from its matching tag. The `RELEASE`
value is case-sensitive:

```bash
gh workflow run publish-pypi.yml \
  --repo openkache/openkache \
  --ref python-v0.1.3 \
  -f version=0.1.3 \
  -f confirm=RELEASE

gh workflow run publish-crates.yml \
  --repo openkache/openkache \
  --ref client-v0.1.3 \
  -f package=client \
  -f version=0.1.3 \
  -f confirm=RELEASE

gh workflow run publish-npm.yml \
  --repo openkache/openkache \
  --ref typescript-v0.1.3 \
  -f version=0.1.3 \
  -f confirm=RELEASE
```

The web UI can dispatch the same workflows: choose the matching tag as the
branch, enter the version from the manifest, and enter `RELEASE` in the
confirmation field.

At this point, wait for `guard`, build, and artifact-collection jobs to pass.
Then stop. The `publish` job should show **Waiting for approval** for
`pypi-release`, `crates-io-release`, or `npm-release`. Do not select **Review
deployments** or **Approve and deploy** in this phase.

Record each run ID and cancel stale runs before retrying a failed preparation:

```bash
gh run list --repo openkache/openkache --limit 20 \
  --json databaseId,workflowName,status,conclusion,headBranch,createdAt
gh run cancel <stale-run-id> --repo openkache/openkache
gh run watch <current-run-id> --repo openkache/openkache --exit-status
```

Wait until the cancelled run is actually `completed` before dispatching its
replacement. A queued or in-progress run for the same tag can make a retry
look stale or consume the protected-environment approval.

## 2. Publish the release

### 2.1 Review the run and credentials

Before approving a package, open its workflow run and verify:

- the run is on the expected package tag and source commit;
- the manifest version matches the workflow input;
- the registry availability guard passed;
- the complete artifact set was uploaded;
- `RELEASE-METADATA` names the expected source tag and commit; and
- `SHA256SUMS` validates every staged artifact.

Configure the protected environments before the first release:

- **`pypi-release`** — configure a PyPI Trusted Publisher for owner
  `openkache`, repository `openkache`, workflow
  `.github/workflows/publish-pypi.yml`, and environment `pypi-release`.
- **`npm-release`** — configure an npm Trusted Publisher for package
  `openkache`, repository `openkache`, workflow `publish-npm.yml`, and
  environment `npm-release`.
- **`crates-io-release`** — for the first `openkache` publication, store a
  narrowly scoped `CARGO_REGISTRY_TOKEN` environment secret. crates.io
  Trusted Publishing cannot bootstrap a crate that has never been published.
  After the first `openkache` version exists, configure its
  [Trusted Publisher](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html) for owner
  `openkache`, repository `openkache`, workflow `publish-crates.yml`, and
  environment `crates-io-release`, then remove the bootstrap secret.

PyPI and npm use GitHub OIDC short-lived credentials. The Rust workflow uses
the bootstrap secret when present and otherwise obtains a short-lived
crates.io credential through GitHub Actions OIDC. Never commit registry
credentials, print them in chat or shell history, or put them in release
artifacts.

### 2.2 Approve and publish

For each package, select **Review deployments**, choose its protected
environment, and approve only after the artifact review above. The workflow
then performs the registry mutation on the tagged checkout:

| Registry | Workflow publish step |
| --- | --- |
| PyPI | `pypa/gh-action-pypi-publish` uploads the sdist and six wheels. |
| crates.io | `rust-lang/crates-io-auth-action` (after bootstrap) or the protected bootstrap secret supplies a short-lived token to `cargo publish --manifest-path clients/rust/Cargo.toml --locked`. |
| npm | `bun run release:publish` publishes the assembled package with the `latest` dist-tag. |

There is no separate local publish command in the normal release path. The
protected workflow rechecks provenance, checksums, package contents, and
registry availability immediately before its publish step.

### 2.3 Verify consumers and record the release

After each workflow succeeds, verify the exact registry version and install it
from a clean consumer environment:

```bash
# Registry metadata
curl --fail --silent --show-error \
  https://pypi.org/pypi/openkache/0.1.3/json >/dev/null
npm view openkache@0.1.3 version dist.integrity
curl --fail --silent --show-error \
  https://crates.io/api/v1/crates/openkache/0.1.3 >/dev/null

# Python
python3 -m venv /tmp/openkache-consumer-python
/tmp/openkache-consumer-python/bin/python -m pip install \
  --index-url https://pypi.org/simple openkache==0.1.3

# Rust
cargo new /tmp/openkache-consumer-rust
(
  cd /tmp/openkache-consumer-rust
  cargo add openkache@0.1.3
  cargo check
)

# TypeScript
mkdir -p /tmp/openkache-consumer-typescript
(
  cd /tmp/openkache-consumer-typescript
  npm init --yes
  npm install openkache@0.1.3
)
```

Download the workflow artifacts before their retention period expires and keep
`RELEASE-METADATA` and `SHA256SUMS` in the release record.

## 3. Recovery

Fix the cause in `main`, merge it, and create or move an unused release tag
only after the registry check returns `404`. A workflow run reads its YAML
from the tag, so rerunning an old tag does not pick up a later workflow fix.

| Symptom | Action |
| --- | --- |
| A macOS Python wheel is named `universal2` or fails the expected architecture check | Keep the wheel architecture-specific, verify the bundled dylib with `lipo -archs`, merge the workflow/setup fix, then move the still-unused `python-v<version>` tag. |
| A wheel build reports missing Bun or Smithy | Use the generated contract snapshots in the sdist and remove generator inputs before the native build; verify the fixed workflow is present on the release tag. |
| A macOS runner stays queued or is unavailable | Use a currently supported runner label in the workflow, merge it, and retag only while the version remains unused. |
| crates.io reports a missing token | For the first crate version, add `CARGO_REGISTRY_TOKEN` to `crates-io-release` without exposing it. For later versions, configure the crates.io Trusted Publisher and let the OIDC step mint the credential. |
| Artifact collection fails or the count is wrong | Do not approve publication. Inspect the failed matrix job and artifact names, fix the build, and rerun preparation from a tag containing the fix. |
| A stale or duplicate run is queued | Cancel the stale run, wait for `completed`, then dispatch one replacement and watch that run ID. |
| A registry check returns `200` | Stop. The version is consumed and cannot be republished; choose the next shared patch version. |

If a workflow fails before its registry command, fix the cause and rerun the
tagged workflow. If a registry has accepted a version, that version is
consumed even when a later workflow step fails: versions cannot be overwritten
or republished. Publish a corrected higher patch version and deprecate or yank
the broken release where the registry supports it.
