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

Configure an npm automation token outside the repository. Bun reads
`NPM_CONFIG_TOKEN`; never put the token in a committed file or command output:

```bash
export NPM_CONFIG_TOKEN='...'
bun pm whoami
bun run release:publish
```

`release:publish` repeats the complete preflight, verifies the authenticated
npm identity, and publishes exactly one immutable package version with the
`latest` dist-tag. It skips lifecycle scripts because `release:check` has
already built and inspected the artifact.

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
intentionally disabled. It remains as the complete release definition, but it
has no automatic event trigger and every job requires the
`OPENKACHE_NPM_RELEASE_ENABLED=true` repository variable. Repository Actions is
currently disabled, so no event consumes a runner or publishes to npm.

Before enabling the workflow, assign it to an approved customer runner and
configure the `NPM_TOKEN` repository secret. Then restore the
`typescript-v<version>` tag trigger, set the release variable to `true`, and
remove the disabled job guard only if the runner and credential policy allows
it. The enabled workflow must publish only from a committed public `main`
snapshot; a manually selected branch must not be used for a production
publication.
