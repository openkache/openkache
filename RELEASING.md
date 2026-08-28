# Release OpenKache packages and server binaries

This guide covers the Python package on PyPI, the Rust crate on crates.io,
the TypeScript package on npm, and the server archives published as GitHub
Release assets. Every registry version and server release is immutable.

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

## Server binary releases

Server binaries use an independent `server-v<version>` tag. The version must
match `server/Cargo.toml`; it does not need to match the client package
versions. The public release workflow builds and checks these immutable assets:

- Linux x86_64: static `x86_64-unknown-linux-musl`
- Linux aarch64: static `aarch64-unknown-linux-musl`
- Apple Silicon macOS: native `aarch64-apple-darwin`

Before tagging, merge the workflow and server changes to public `main`, verify
the public checkout is clean, and confirm that the protected `server-release`
environment requires a reviewer. Then create and push the tag:

```bash
export RELEASE_VERSION=0.1.0  # replace with the chosen server version
git fetch origin main --no-tags
git switch main
git pull --ff-only
test "$(sed -n 's/^version = "\(.*\)"/\1/p' server/Cargo.toml | head -n 1)" = "${RELEASE_VERSION}"
git tag "server-v${RELEASE_VERSION}"
git push origin "server-v${RELEASE_VERSION}"
```

Dispatch the workflow from that exact tag and type `RELEASE` for the
confirmation input:

```bash
gh workflow run publish-server-binaries.yml \
  --repo openkache/openkache \
  --ref "server-v${RELEASE_VERSION}" \
  -f "version=${RELEASE_VERSION}" \
  -f confirm=RELEASE
```

The workflow pins every build and publish checkout to the dispatch commit,
refreshes the remote tag immediately before publication, and aborts if the tag
moved during the run. It refuses an existing GitHub Release with the same tag
and fails closed when the GitHub API cannot prove that the release is absent.
The workflow verifies ELF/Mach-O architecture and Linux static-linking
properties, then publishes per-archive `.sha256` files plus the aggregate
`SHA256SUMS`. SHA-256 detects transfer corruption; it is not a signature or
provenance attestation. These archives remain preview artifacts and must not be
described as production support until a separate signing and provenance policy
is in place.

## Choose the release version

The three client manifests must contain the same version:

- Python: `clients/python/pyproject.toml`
- Rust: `clients/rust/Cargo.toml`
- TypeScript: `clients/typescript/package.json`

For a normal release, increment the shared version by `0.0.1`. If a previous
release was partial, keep its version and release only the packages whose
registry checks still return `404`. If the change is breaking, stop and ask
the user to choose the version before updating any manifest or tag. Never
republish a package version that a registry has accepted, even if it was later
yanked.

Set the version once and use it for every command below:

```bash
export RELEASE_VERSION=0.1.4  # replace with the chosen unused version
```

## 1. Prepare the release (stop before publication)

All commands in this section run from a clean checkout of the public
`openkache` repository, not from the private monorepo.

### 1.1 Check the source and versions

Start from the public `main` branch after the client changes have been merged:

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
versions are still unused:

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

Before dispatching, confirm that `pypi-release`, `crates-io-release`, and
`npm-release` exist as protected GitHub environments with required reviewers.
Also confirm the registry credential for the package you are about to publish:
PyPI and npm use Trusted Publishing; crates.io needs its bootstrap token for
the first publication and can use Trusted Publishing after that first version
exists. See [Review and credentials](#21-review-the-run-and-credentials).

If a check returns `200`, skip that package and continue only with packages
whose checks returned `404`. Choose a new shared patch version only when
starting a new release or when you would otherwise need to republish a
version already accepted by a registry.

### 1.2 Build and inspect the packages

Keep generated artifacts outside the repository or in ignored directories.
The commands below validate the artifacts that can be built on the current
host:

```bash
set -euo pipefail

# Generate the dependency notice used by the Rust crate. The other package
# commands below invoke their own packaging hooks, but all use this same
# locked generator and should be run after these preparation steps.
cargo fetch --locked
cargo fetch --locked --manifest-path clients/python/native/Cargo.toml
bun install --cwd scripts --frozen-lockfile --production
./scripts/generate-third-party-notices.ts \
  --artifact rust \
  --output clients/rust/THIRD-PARTY-NOTICES.txt

# Rust: verify metadata and inspect the crate archive.
cargo metadata \
  --manifest-path clients/rust/Cargo.toml \
  --locked \
  --no-deps \
  --format-version 1
cargo publish \
  --manifest-path clients/rust/Cargo.toml \
  --locked \
  --dry-run \
  --allow-dirty
cargo package --manifest-path clients/rust/Cargo.toml --locked --allow-dirty
tar -tzf "target/package/openkache-${RELEASE_VERSION}.crate" \
  | grep -Fx "openkache-${RELEASE_VERSION}/Cargo.toml"
tar -tzf "target/package/openkache-${RELEASE_VERSION}.crate" \
  | grep -F "openkache-${RELEASE_VERSION}/src/lib.rs"
tar -tzf "target/package/openkache-${RELEASE_VERSION}.crate" \
  | grep -Fx "openkache-${RELEASE_VERSION}/THIRD-PARTY-NOTICES.txt"

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
  bun run build:package
  bun run typecheck
  bun pm pack --dry-run
)
```

The generated notices may contain a `LEGAL REVIEW REQUIRED` section when a
registry archive does not include a separate upstream license file. Inspect
the package-specific notice files before dispatching a release; do not approve
the protected publication environment until every such entry has been checked
against its upstream source. The notice bundle does not replace notices for a
container base image or system libraries supplied by the host.

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

Only tag the commit that is on public `main` and passed the checks above. Create
tags only for packages whose registry check returned `404`. For example, if
PyPI is already published, leave `python-v${RELEASE_VERSION}` untouched and
create only the Rust and TypeScript tags:

```bash
set -euo pipefail

# Uncomment only the tags for packages that still need publication.
release_tags=(
  # "python-v${RELEASE_VERSION}"
  "client-v${RELEASE_VERSION}"
  "typescript-v${RELEASE_VERSION}"
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
`pypi-release`, `crates-io-release`, or `npm-release`. Do not select **Review
deployments** or **Approve and deploy** in this phase.

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

Before approving a package, open its workflow run and verify:

- the run is on the expected package tag and source commit;
- the manifest version matches the workflow input;
- the registry availability guard passed;
- the complete artifact set was uploaded;
- `RELEASE-METADATA` names the expected source tag and commit;
- `SHA256SUMS` validates every staged artifact; and
- each `THIRD-PARTY-NOTICES.txt` has been inspected, with no unresolved
  `LEGAL REVIEW REQUIRED` entries.

Configure the protected environments before the first release:

- **`pypi-release`** — configure a PyPI Trusted Publisher for owner
  `openkache`, repository `openkache`, workflow
  `.github/workflows/publish-pypi.yml`, and environment `pypi-release`.
- **`npm-release`** — configure an npm Trusted Publisher for package
  `openkache`, repository `openkache`, workflow `publish-npm.yml`, and
  environment `npm-release`.
- **`crates-io-release`** — if `openkache` has never been published, store a
  narrowly scoped `CARGO_REGISTRY_TOKEN` environment secret for the bootstrap
  release. crates.io Trusted Publishing cannot bootstrap a crate that has
  never been published. After the first `openkache` version exists, configure
  its [Trusted Publisher](https://crates.io/docs/trusted-publishing) with
  owner `openkache`, repository `openkache`, workflow filename
  `publish-crates.yml`, and environment `crates-io-release`.

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
| crates.io | `rust-lang/crates-io-auth-action` (after bootstrap) or the protected bootstrap secret supplies the registry token to `cargo publish --manifest-path clients/rust/Cargo.toml --locked`. |
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

Download the workflow artifacts before their retention period expires and keep
`RELEASE-METADATA` and `SHA256SUMS` in the release record:

```bash
# Replace these with the completed run IDs. Run only for workflows that ran.
pypi_run_id=123456791
crates_run_id=123456792
npm_run_id=123456793
mkdir -p "release-artifacts/${RELEASE_VERSION}"
gh run download "${pypi_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/pypi"
gh run download "${crates_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/crates"
gh run download "${npm_run_id}" \
  --repo openkache/openkache \
  --dir "release-artifacts/${RELEASE_VERSION}/npm"
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
| `cargo publish --dry-run` rejects a wildcard dependency requirement | Pin the dependency range in `clients/rust/Cargo.toml`, merge the fix on `main`, confirm that only the Rust version is still `404`, move only `client-v<version>`, and rerun the Rust workflow. |
| crates.io reports a missing token | For the first crate version, add `CARGO_REGISTRY_TOKEN` to `crates-io-release` without exposing it. For later versions, configure the crates.io Trusted Publisher and let the OIDC step mint the credential. |
| One registry returns `200` while another returns `404` | Skip the already-published package and continue only the `404` package workflows at the same shared version. Do not move the published package's tag. |
| Artifact collection fails or the count is wrong | Do not approve publication. Inspect the failed matrix job and artifact names, fix the build, and rerun preparation from a tag containing the fix. |
| A stale or duplicate run is queued | Cancel the stale run, wait for `completed`, then dispatch one replacement and watch that run ID. |
| A registry check returns `200` | Skip that package. If you intended to republish the same package version, stop and choose the next shared patch version; continue other packages only when their checks return `404`. |

If a workflow fails before its registry command, fix the cause and rerun the
tagged workflow. If a registry has accepted a version, that version is
consumed even when a later workflow step fails: versions cannot be overwritten
or republished. Publish a corrected higher patch version and deprecate or yank
the broken release where the registry supports it.
