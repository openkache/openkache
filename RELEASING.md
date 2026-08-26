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
- **`crates-io-release`** — store a narrowly scoped
  `CARGO_REGISTRY_TOKEN` repository secret for the `openkache` crate.

PyPI and npm use GitHub OIDC short-lived credentials. The Rust workflow passes
the scoped token to `cargo publish`. Never commit registry credentials or put
them in shell history or release artifacts.

### 2.2 Approve and publish

For each package, select **Review deployments**, choose its protected
environment, and approve only after the artifact review above. The workflow
then performs the registry mutation on the tagged checkout:

| Registry | Workflow publish step |
| --- | --- |
| PyPI | `pypa/gh-action-pypi-publish` uploads the sdist and six wheels. |
| crates.io | `cargo publish --manifest-path clients/rust/Cargo.toml --locked` uploads the crate. |
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

If a workflow fails before its registry command, fix the cause and rerun the
tagged workflow. If a registry has accepted a version, that version is
consumed even when a later workflow step fails: versions cannot be overwritten
or republished. Publish a corrected higher patch version and deprecate or yank
the broken release where the registry supports it.
