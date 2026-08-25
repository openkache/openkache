# Releasing `openkache`

This package is published to the public npm registry with Bun. The version in
`package.json` is the only release version source of truth; npm versions are
immutable, so the publication step never changes a version after the artifact
has been built.

## Release invariants

`release:verify` refuses to continue unless:

- the package name and public package access are correct;
- the version is a stable three-part SemVer value;
- the public Git worktree has no tracked changes;
- a tag, when running in GitHub Actions, is `typescript-v<version>`;
- declarations and JavaScript output exist;
- Linux x64, Linux ARM64, and Apple Silicon macOS native adapters all exist;
- every required output is declared in `package.json.files`; and
- the exact package version is not already present on npm.

The native adapter build is host-specific. Linux builds both Linux artifacts;
Apple Silicon macOS builds the Darwin artifact. A release checkout must stage
all three outputs under `target/native/` before the release verification is
run. `release:check` performs the host build first; `release:verify` is the
artifact-only check used after a multi-platform build has been assembled.

## First release: `0.1.0`

Run these commands from `clients/typescript` after the public source change has
merged:

```bash
bun install --frozen-lockfile
bun pm pkg set version=0.1.0
bun run build
bun run typecheck
bun run release:dry-run
```

The version edit must be committed and merged before publication. Do not
publish from a detached checkout or with a dirty worktree.

`release:dry-run` builds the package, prints the tarball contents, checks the
registry for a free version, and performs Bun's own publish dry run. It does
not require registry credentials and does not mutate npm. On Linux, stage the
Apple Silicon adapter first when using this command for a complete release
check.

## Publish

GitHub Actions uses npm Trusted Publishing for registry authentication. In the
`openkache` package settings on npmjs.com, configure a GitHub Actions trusted
publisher with:

- organization or user: `openkache`;
- repository: `openkache`;
- workflow filename: `publish-npm.yml`; and
- environment: `npm-release`.

The publish job grants `id-token: write` and invokes npm's OIDC-aware CLI from
a GitHub-hosted runner. No `NPM_TOKEN` secret is required. `release:publish`
repeats the complete preflight and publishes exactly one immutable package
version with the `latest` dist-tag. It skips lifecycle scripts because
`release:check` has already built and inspected the artifact.

For an explicitly local, token-authenticated publication, leave
`OPENKACHE_TRUSTED_PUBLISHING` unset and configure `NPM_CONFIG_TOKEN` outside
the repository. Never put the token in a committed file or command output:

```bash
export NPM_CONFIG_TOKEN='...'
bun pm whoami
bun run release:publish
```

After a successful publication, verify the registry metadata and install the
published package from a clean consumer project:

```bash
bun pm view openkache@0.1.0
bun add openkache@0.1.0
```

The package cannot be republished at the same version. If publication fails
after npm has accepted the package, treat that version as consumed and
investigate the registry state before choosing a new version.

## GitHub release convention

The checked-in
[`publish-npm.yml`](../../.github/workflows/publish-npm.yml) workflow is
manual-only. Dispatch it from the committed `typescript-v<version>` tag with
the matching `version` input and the literal `RELEASE` confirmation. It builds
the JavaScript output and a Linux x64, Linux ARM64, and Apple Silicon native
adapter matrix, verifies each artifact checksum and source commit, then waits
for the protected `npm-release` environment before calling
`release:publish`.

The environment protects the OIDC-backed publish job. A failed preflight does
not publish. npm versions are immutable; if npm accepts a version, treat it as
consumed and publish a corrected higher version rather than attempting a
republish. The repository-wide
[release guide](../../RELEASING.md) documents the other package workflows,
operator inputs, provenance files, and rollback limits.
