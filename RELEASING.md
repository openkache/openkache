# Release OpenKache packages and server binaries

This guide covers the Python package on PyPI, the Rust client and server
crates on crates.io, the TypeScript package on npm, and the server archives
published as GitHub Release assets. Every registry version and server release
is immutable.

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
version is still unused, move only the affected package tag after confirming
that package's registry check returns `404`; never move a tag after that
registry has accepted the version.

If one registry already contains a version while the other registries return
`404`, treat it as a partial release: skip the package that is already
published and continue with the remaining packages at the same shared
version. Do not force a new version just because one package finished first.

## Release families

Client packages share one version. The Python package, Rust client crate, and
TypeScript package use `RELEASE_VERSION` and their own package tags.

The `openkache-server` crate and downloadable server archives share
`SERVER_RELEASE_VERSION`, the `server/Cargo.toml` version, and one
`server-v<version>` tag. The crate is published by `publish-crates.yml`; the
archives are published by `publish-server-binaries.yml`. They are separate
workflow runs and separate protected approvals, so you can release either one
or both from the same tag.

The binary workflow builds and checks immutable archives for Linux x86_64,
Linux aarch64, and Apple Silicon macOS. Linux archives are static musl
executables; the macOS archive is an arm64 executable.

## Choose the release version

The three client manifests must contain the same version:

- Python: `clients/python/pyproject.toml`
- Rust: `clients/rust/Cargo.toml`
- TypeScript: `clients/typescript/package.json`

For a normal client release, increment the shared version by `0.0.1`. If a
previous release was partial, keep its version and release only the packages
whose registry checks still return `404`. If the change is breaking, stop and
ask the user to choose the version before updating any manifest or tag. Never
republish a package version that a registry has accepted, even if it was later
yanked.

Set the version once and use it for every command below:

```bash
export RELEASE_VERSION=0.1.4  # replace with the chosen unused version
```

Set the independent server version only when preparing a server release:

```bash
export SERVER_RELEASE_VERSION=0.1.0
```

For a server-only release, skip the client manifest, registry, build, tag, and
consumer commands below. For a client-only release, leave
`SERVER_RELEASE_VERSION` unset.

## 1. Prepare the release (stop before publication)

All commands in this section run from a clean checkout of the public
`openkache` repository, not from the private monorepo.

### 1.1 Check the source and versions

Start from the public `main` branch after the release changes have been merged:

```bash
git fetch origin main --no-tags
git switch main
git pull --ff-only
test -z "$(git status --porcelain=v1 --untracked-files=all)"
```

Fetching `main` without all tags avoids a stale local tag preventing the
source update. Check remote tags explicitly when you are ready to create or
repair a package tag.

Confirm that the release workflow files on `origin/main` are the ones you
intend to run before creating tags:

```bash
git show origin/main:.github/workflows/publish-pypi.yml >/dev/null
git show origin/main:.github/workflows/publish-crates.yml >/dev/null
git show origin/main:.github/workflows/publish-npm.yml >/dev/null
git show origin/main:.github/workflows/publish-server-binaries.yml >/dev/null
```

The release commit is the provenance boundary. Do not build or publish from a
feature branch, detached checkout, or dirty worktree. If the Rust version
changed, `Cargo.lock` and the docs.rs URL in `clients/rust/src/lib.rs` must be
updated in the same reviewed change.

Before creating tags, confirm the manifest versions and that the registry
versions are still unused. For a client release, run:

#### Client registry checks

```bash
set -euo pipefail

python3 - <<'PY'
import json
import os
import pathlib
import tomllib

versions = {
    "python": tomllib.loads(
        pathlib.Path("clients/python/pyproject.toml").read_text()
    )["project"]["version"],
    "rust": tomllib.loads(
        pathlib.Path("clients/rust/Cargo.toml").read_text()
    )["package"]["version"],
    "typescript": json.loads(
        pathlib.Path("clients/typescript/package.json").read_text()
    )["version"],
}
for package, version in versions.items():
    print(package, version)
expected = os.environ["RELEASE_VERSION"]
if set(versions.values()) != {expected}:
    raise SystemExit(
        f"client manifests must all use RELEASE_VERSION={expected}"
    )
PY

check_registry() {
  local url="$1"
  local label="$2"
  local http_code
  if ! http_code="$(
      curl --silent --show-error --location --retry 2 --retry-all-errors \
        --connect-timeout 10 --max-time 30 \
        --user-agent "openkache-release-preflight" \
        --output /dev/null --write-out '%{http_code}' "$url"
    )"; then
    echo "$label registry check failed" >&2
    return 1
  fi
  case "$http_code" in
    404) echo "$label is not published; prepare its workflow" ;;
    200) echo "$label is already published; skip its workflow" ;;
    *) echo "$label registry check returned HTTP $http_code" >&2; return 1 ;;
  esac
}

check_registry \
  "https://pypi.org/pypi/openkache/${RELEASE_VERSION}/json" \
  "PyPI openkache@${RELEASE_VERSION}"
check_registry \
  "https://registry.npmjs.org/openkache/${RELEASE_VERSION}" \
  "npm openkache@${RELEASE_VERSION}"
check_registry \
  "https://crates.io/api/v1/crates/openkache/${RELEASE_VERSION}" \
  "crates.io openkache@${RELEASE_VERSION}"
```

The check is per package. A `404` means that package still needs its tag and
workflow; a `200` means leave its existing tag alone and skip its workflow.
Stop for any other response. If all three checks return `200`, there is
nothing to release.

For a server release, check the server crate separately:

#### Server registry check

```bash
check_registry \
  "https://crates.io/api/v1/crates/openkache-server/${SERVER_RELEASE_VERSION}" \
  "crates.io openkache-server@${SERVER_RELEASE_VERSION}"
```

Before dispatching, confirm that `pypi-release`, `crates-io-release`,
`npm-release`, and (when releasing archives) `server-release` exist as
protected GitHub environments with required reviewers. Also confirm the
registry credential for the package you are about to publish: PyPI and npm use
Trusted Publishing; crates.io needs its bootstrap token for the first
publication and can use Trusted Publishing after that first version exists.
See [Review and credentials](#21-review-the-run-and-credentials).

If a check returns `200`, skip that package and continue only with packages
whose checks returned `404`. Choose a new shared patch version only when
starting a new release or when you would otherwise need to republish a
version already accepted by a registry.

### 1.2 Build and inspect the packages

Keep generated artifacts outside the repository or in ignored directories.
Run the client or server block that matches the release you are preparing.

#### Client packages

```bash
set -euo pipefail

# Rust client
cargo metadata \
  --manifest-path clients/rust/Cargo.toml \
  --locked \
  --no-deps \
  --format-version 1
cargo publish \
  --manifest-path clients/rust/Cargo.toml \
  --locked \
  --dry-run
cargo package --manifest-path clients/rust/Cargo.toml --locked
tar -tzf "target/package/openkache-${RELEASE_VERSION}.crate" \
  | grep -Fx "openkache-${RELEASE_VERSION}/Cargo.toml"
tar -tzf "target/package/openkache-${RELEASE_VERSION}.crate" \
  | grep -F "openkache-${RELEASE_VERSION}/src/lib.rs"

# Python
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

# TypeScript
(
  cd clients/typescript
  bun install --frozen-lockfile
  bun run build
  bun run typecheck
  bun run build:native
  bun pm pack --dry-run
)
```

#### Server crate

For a server release, inspect the crates.io package separately:

```bash
set -euo pipefail

cargo metadata \
  --manifest-path server/Cargo.toml \
  --locked \
  --no-deps \
  --format-version 1
cargo publish \
  --manifest-path server/Cargo.toml \
  --locked \
  --dry-run
cargo package --manifest-path server/Cargo.toml --locked
tar -tzf "target/package/openkache-server-${SERVER_RELEASE_VERSION}.crate" \
  | grep -Fx "openkache-server-${SERVER_RELEASE_VERSION}/Cargo.toml"
tar -tzf "target/package/openkache-server-${SERVER_RELEASE_VERSION}.crate" \
  | grep -F "openkache-server-${SERVER_RELEASE_VERSION}/src/main.rs"
```

The server binary workflow builds the complete archive matrix on clean GitHub
hosted runners. It produces Linux x86_64 and aarch64 static-musl binaries and
an Apple Silicon macOS arm64 binary, then verifies each archive and its
checksum before the protected publish job.

The release workflows build the complete platform matrix on clean GitHub
hosted runners:

- Python: one sdist plus Linux x86_64/aarch64, macOS x86_64/arm64, and Windows
  x86_64/arm64 wheels.
- Rust client or server: one crate archive for the selected package.
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

Only tag the commit that is on public `main` and passed the checks above. Create
tags only for packages whose registry check returned `404`. For example, if
PyPI is already published, leave `python-v${RELEASE_VERSION}` untouched and
create only the Rust and TypeScript tags:

```bash
set -euo pipefail

# Uncomment only the tags for packages that still need publication.
release_tags=(
  # "python-v${RELEASE_VERSION}"
  # "client-v${RELEASE_VERSION}"
  # "typescript-v${RELEASE_VERSION}"
  # "server-v${SERVER_RELEASE_VERSION}"
)

main_commit="$(git rev-parse HEAD)"
for tag in "${release_tags[@]}"; do
  remote_commit="$(git ls-remote --refs origin "refs/tags/${tag}" | cut -f1)"
  if [[ -n "${remote_commit}" ]]; then
    [[ "${remote_commit}" == "${main_commit}" ]] || {
      echo "${tag} already points to a different commit; stop and inspect it" >&2
      exit 1
    }
    echo "${tag} already points to this main commit"
  else
    git tag "${tag}"
    git push origin "${tag}"
  fi
done
```

For a server-only release, set `SERVER_RELEASE_VERSION` to the version from
`server/Cargo.toml` and enable only `"server-v${SERVER_RELEASE_VERSION}"` in
the array. Do not create client tags for a server-only release.

If a release workflow was fixed after a tag was created, first rerun the
registry check for the affected package. Only when that package's version is
still unused may you move its tag to the fixed `main` commit:

```bash
tag_prefix=python  # set to client or typescript for another package
fixed_main_commit="$(git rev-parse HEAD)"
git tag -f "${tag_prefix}-v${RELEASE_VERSION}" "${fixed_main_commit}"
git push --force-with-lease \
  "origin" \
  "${fixed_main_commit}:refs/tags/${tag_prefix}-v${RELEASE_VERSION}"
```

Never move a tag after its registry has accepted the version; create the next
shared patch release instead.

Dispatch one workflow for each package whose registry check returned `404`,
from its matching tag. The `RELEASE` value is case-sensitive:

```bash
# Run only the commands for packages that still need publication.
gh workflow run publish-pypi.yml \
  --repo openkache/openkache \
  --ref "python-v${RELEASE_VERSION}" \
  -f "version=${RELEASE_VERSION}" \
  -f confirm=RELEASE

gh workflow run publish-crates.yml \
  --repo openkache/openkache \
  --ref "client-v${RELEASE_VERSION}" \
  -f package=client \
  -f "version=${RELEASE_VERSION}" \
  -f confirm=RELEASE

# Server release (use the server-v<version> tag and its independent version).
gh workflow run publish-crates.yml \
  --repo openkache/openkache \
  --ref "server-v${SERVER_RELEASE_VERSION}" \
  -f package=server \
  -f "version=${SERVER_RELEASE_VERSION}" \
  -f confirm=RELEASE

gh workflow run publish-server-binaries.yml \
  --repo openkache/openkache \
  --ref "server-v${SERVER_RELEASE_VERSION}" \
  -f "version=${SERVER_RELEASE_VERSION}" \
  -f confirm=RELEASE

gh workflow run publish-npm.yml \
  --repo openkache/openkache \
  --ref "typescript-v${RELEASE_VERSION}" \
  -f "version=${RELEASE_VERSION}" \
  -f confirm=RELEASE
```

The web UI can dispatch the same workflows: choose the matching tag as the
branch, enter the version from the manifest, and enter `RELEASE` in the
confirmation field.

At this point, wait for `guard`, build, and artifact-collection jobs to pass.
Then stop. The `publish` job should show **Waiting for approval** for
`pypi-release`, `crates-io-release`, `npm-release`, or `server-release`.
Do not select **Review deployments** or **Approve and deploy** in this phase.

Record each run ID and cancel stale runs before retrying a failed preparation:

```bash
gh run list --repo openkache/openkache --limit 20 \
  --json databaseId,workflowName,status,conclusion,headBranch,createdAt
stale_run_id=123456789
current_run_id=123456790
gh run cancel "${stale_run_id}" --repo openkache/openkache
gh run watch "${current_run_id}" --repo openkache/openkache --exit-status
```

Wait until the cancelled run is actually `completed` before dispatching its
replacement. A queued or in-progress run for the same tag can make a retry
look stale or consume the protected-environment approval.

## 2. Publish the release

### 2.1 Review the run and credentials

Before approving a package or server archive, open its workflow run and verify:

- the run is on the expected package/server tag and source commit;
- the manifest version matches the workflow input;
- the registry or release availability guard passed;
- the complete artifact set was uploaded;
- `RELEASE-METADATA` names the expected source tag and commit; and
- `SHA256SUMS` validates every staged artifact. For a binary release, also
  verify that all three platform archives are present.

Configure the protected environments before the first release:

- **`pypi-release`** — configure a PyPI Trusted Publisher for owner
  `openkache`, repository `openkache`, workflow
  `.github/workflows/publish-pypi.yml`, and environment `pypi-release`.
- **`npm-release`** — configure an npm Trusted Publisher for package
  `openkache`, repository `openkache`, workflow `publish-npm.yml`, and
  environment `npm-release`.
- **`server-release`** — protect the environment used by
  `.github/workflows/publish-server-binaries.yml` and require a reviewer.
- **`crates-io-release`** — for each crate that has never been published,
  store a narrowly scoped `CARGO_REGISTRY_TOKEN` environment secret for its
  bootstrap release. crates.io Trusted Publishing cannot bootstrap an
  unpublished crate. After the first version exists, configure that crate's
  [Trusted Publisher](https://crates.io/docs/trusted-publishing) with owner
  `openkache`, repository `openkache`, workflow filename
  `publish-crates.yml`, and environment `crates-io-release`. Configure this
  separately for `openkache` and `openkache-server`.

PyPI and npm use GitHub OIDC short-lived credentials. The Rust workflow uses
the bootstrap secret when present and otherwise obtains a short-lived
crates.io credential through GitHub Actions OIDC. Never commit registry
credentials, print them in chat or shell history, or put them in release
artifacts.

Keep the bootstrap token until the current release has completed. Before a
later release, remove or disable the `CARGO_REGISTRY_TOKEN` environment secret
so the `Authenticate with crates.io Trusted Publishing` step can run. Confirm
that step succeeds rather than being skipped, and then leave the bootstrap
secret removed. If the step is skipped, the workflow is still using the
bootstrap token.

### 2.2 Approve and publish

For each package, select **Review deployments**, choose its protected
environment, and approve only after the artifact review above. The workflow
then performs the registry mutation on the tagged checkout:

| Registry | Workflow publish step |
| --- | --- |
| PyPI | `pypa/gh-action-pypi-publish` uploads the sdist and six wheels. |
| crates.io | `rust-lang/crates-io-auth-action` (after bootstrap) or the protected bootstrap secret supplies the registry token to `cargo publish --manifest-path <selected crate> --locked`. |
| npm | `bun run release:publish` publishes the assembled package with the `latest` dist-tag. |
| GitHub Releases | `publish-server-binaries.yml` uploads the three server archives, checksums, and release metadata. |

There is no separate local publish command in the normal release path. The
protected workflow rechecks provenance, checksums, package contents, and
registry availability immediately before its publish step.

### 2.3 Verify consumers and record the release

After each workflow succeeds, verify the exact version from a clean consumer
environment. For a client release:

```bash
curl --fail --silent --show-error \
  "https://pypi.org/pypi/openkache/${RELEASE_VERSION}/json" >/dev/null
npm view "openkache@${RELEASE_VERSION}" version dist.integrity
curl --fail --silent --show-error \
  "https://crates.io/api/v1/crates/openkache/${RELEASE_VERSION}" >/dev/null

# Python
python3 -m venv /tmp/openkache-consumer-python
/tmp/openkache-consumer-python/bin/python -m pip install \
  --no-cache-dir --index-url https://pypi.org/simple \
  "openkache==${RELEASE_VERSION}"

# Rust
cargo new /tmp/openkache-consumer-rust
(
  cd /tmp/openkache-consumer-rust
  cargo add "openkache@${RELEASE_VERSION}"
  cargo check
)

# TypeScript
mkdir -p /tmp/openkache-consumer-typescript
(
  cd /tmp/openkache-consumer-typescript
  npm init --yes
  npm install "openkache@${RELEASE_VERSION}"
)
```

For a server release:

```bash
curl --fail --silent --show-error \
  "https://crates.io/api/v1/crates/openkache-server/${SERVER_RELEASE_VERSION}" \
  >/dev/null
gh release view "server-v${SERVER_RELEASE_VERSION}" \
  --repo openkache/openkache \
  --json tagName, assets
cargo install --locked --version "${SERVER_RELEASE_VERSION}" openkache-server
```

Download the workflow artifacts before their retention period expires and keep
`RELEASE-METADATA` and `SHA256SUMS` in the release record:

For client workflow runs:

```bash
# Replace these with completed run IDs; run only the commands for workflows
# that actually ran.
pypi_run_id=123456791
crates_client_run_id=123456792
npm_run_id=123456793
mkdir -p "release-artifacts/${RELEASE_VERSION}"
gh run download "${pypi_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/pypi"
gh run download "${crates_client_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/crates"
gh run download "${npm_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/npm"
```

For server workflow runs:

```bash
# Replace these with completed run IDs; run only the commands for workflows
# that actually ran.
crates_server_run_id=123456794
server_binaries_run_id=123456795
mkdir -p "release-artifacts/${SERVER_RELEASE_VERSION}"
gh run download "${crates_server_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${SERVER_RELEASE_VERSION}/server-crate"
gh run download "${server_binaries_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${SERVER_RELEASE_VERSION}/server-binaries"
```

## 3. Recovery

Fix the cause in `main`, merge it, and create or move an unused release tag
only after the registry check returns `404`. A workflow run reads its YAML
from the tag, so rerunning an old tag does not pick up a later workflow fix.

| Symptom | Action |
| --- | --- |
| A macOS Python wheel is named `universal2` or fails the expected architecture check | Keep the wheel architecture-specific, verify the bundled dylib with `lipo -archs`, merge the workflow/setup fix, then move the still-unused `python-v<version>` tag. |
| A wheel build reports missing Bun or Smithy | Use the generated contract snapshots in the sdist and remove generator inputs before the native build; verify the fixed workflow is present on the release tag. |
| A macOS runner stays queued or is unavailable | Use a currently supported runner label in the workflow, merge it, and retag only while the version remains unused. |
| `cargo publish --dry-run` rejects a wildcard dependency requirement | Pin the dependency range in the affected `Cargo.toml`, merge the fix on `main`, confirm that only that version is still `404`, move only its tag, and rerun the matching workflow. |
| crates.io reports a missing token | For the first crate version, add `CARGO_REGISTRY_TOKEN` to `crates-io-release` without exposing it. For later versions, configure the crates.io Trusted Publisher and let the OIDC step mint the credential. |
| One registry returns `200` while another returns `404` | Skip the already-published package and continue only the `404` package workflows at the same shared version. Do not move the published package's tag. |
| Artifact collection fails or the count is wrong | Do not approve publication. Inspect the failed matrix job and artifact names, fix the build, and rerun preparation from a tag containing the fix. |
| A stale or duplicate run is queued | Cancel the stale run, wait for `completed`, then dispatch one replacement and watch that run ID. |
| A server binary fails its architecture or static-link check | Do not approve publication. Fix the target-specific build, merge it to `main`, and move `server-v<version>` only while the crates.io and GitHub Release versions remain unused. |

If a workflow fails before its registry command, fix the cause and rerun the
tagged workflow. If a registry has accepted a version, that version is
consumed even when a later workflow step fails: versions cannot be overwritten
or republished. Publish a corrected higher patch version and deprecate or yank
the broken release where the registry supports it.
