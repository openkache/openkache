---
name: commit
description: xxxxxxxxxxxxxxxxxx Standards for git commit messages, atomic commit principles, and the /commit workflow. Consult before committing code to ensure format compliance and proper change granularity.
---

# Git Commit Standards & Workflow

## Overview & Triggers

### Commit Trigger [LOAD-BEARING]
The `/commit` button (or slash command) is the **ONLY** trigger for `git commit` and `git push`.

**NEVER** commit or push through:
- Direct `git commit` commands without `/commit` trigger
- IDE commit shortcuts
- Other background automation

### Approval Protocol [LOAD-BEARING]
**NEVER** auto-run a commit command (`git commit`, etc.), even if a workflow annotation (like `// turbo`) or separate workflow instructions imply it is safe.

- **Mandatory User Approval**: Present the exact `git commit` command to the user and wait for explicit approval before execution (via the tool approval UI).
- **Workflow Priority**: This rule takes precedence over ALL other workflows, including `/commit` and `/update-memory`.
- **Zero-Tolerance for Redundant Confirmation [LOAD-BEARING]**: Asking for natural language confirmation ("Shall I proceed?", "Is it okay to commit?") for a `/commit` command or an approved plan is a **PROTOCOL FAILURE**. Present the command directly and let the tool UI handle the final human-in-the-loop approval. **If the user has ordered a commit via a slash command, do not ask for permission in text; present the tool call directly.**
- **Rationale**: Reduces confirmation fatigue while keeping human control over VCS state.

---

## Execution Workflow: `/commit`

This workflow covers **all git repositories** in the workspace, including nested sub-repos.

1. **Discover All Repos [LOAD-BEARING]**:
   Search for all independent nested git repositories:
   ```bash
   find . -name ".git" -not -path "./.git" -maxdepth 4 -type d 2>/dev/null
   ```
   Run `git status` in **each discovered repo** AND the root. Process ALL repos that have changes.

2. **Check Status & Diff**:
   In each modified repo, inspect `git status` and `git diff` to review all changed and untracked files.

3. **Atomic Splitting**:
   Separate changes covering multiple logical features, bug fixes, or refactors into distinct commit boundaries.

4. **Stage & Commit Cycles**:
   For each logical unit in each repo:
   - Stage files: `git add <relevant files>`
   - Formulate a Conventional Commit message (with emoji and description if non-trivial).
   - Present the `git commit` command directly for tool UI approval.

5. **Push All Repos**:
   Once all logical units are committed across all repos, run `git push` in each repo.
   *Note: If SSH path/config errors occur, use `GIT_SSH_COMMAND="/nix/store/71zy9jmcszcfmmn4zf68sm8vywryybhp-openssh-10.3p1/bin/ssh -F /dev/null -i /home/kkimdev/.ssh/id_ed25519"`.*

6. **Verify**:
   Ensure all pushes succeeded and all repos are in a clean working state (excluding `.local.` files).

---

## Core Commit Principles

### Proactive Committing [LOAD-BEARING]
Commit logically complete units of work **as they are finished**, rather than waiting until the end of a long session.
- **Logical Units**: A single bug fix, a new feature component, a refactored module, or documentation updates.
- **Micro-Commits**: Split tasks with backend/frontend or multi-step work into separate commits.
- **VCS Safety**: Keep the working directory clean and create frequent save points.

### Atomic Commits [LOAD-BEARING]
Each commit should represent **ONE logical change**. Keep separate commits for:
- Feature implementations
- Bug fixes
- Refactoring
- Documentation updates
- Configuration changes

*Anti-pattern*: Mixing feature code, refactoring, and documentation in a single commit.

### Multi-Repo Commits [LOAD-BEARING]
When changes span multiple repositories:
1. **Maintain Isolation**: Commit to each repo separately.
2. **Context Synchronization**: Use consistent commit messages across repositories for related tasks.
3. **Dependency Order**: Commit to dependencies or sub-projects first, then consuming projects.

### Untracked Files and Folders [LOAD-BEARING]
1. **Identify & Classify**: Inspect all untracked files from `git status`.
2. **Transient/Scratch Files**: Keep files matching `.local.` or `_local/` untracked.
3. **Project Files**: Automatically stage, commit, and push permanent project files (docs, configs, new source files) as part of the commit workflow without prompting for text confirmation.

---

## Commit Message Format

### Message Structure
```
<emoji> <type>: <short summary>

<detailed description>
```

### Conventional Commit Types & Emojis
Use direct **Unicode emojis**, not shortcodes.

| Type | Emoji | Meaning |
| :--- | :---: | :--- |
| `feat` | ✨ | New features or capabilities |
| `fix` | 🐛 | Bug fixes |
| `refactor` | ♻️ | Code restructuring without behavior change |
| `docs` | 📝 | Documentation only changes |
| `chore` | 🔧 | Build, dependencies, tooling |
| `test` | ✅ | Test additions or modifications |
| `style` | 🎨 | Code style/formatting (not CSS) |
| `perf` | ⚡ | Performance improvements |
| `ci` | 💚 | CI/CD changes |

### Summary & Description Rules
- **Summary**: Use imperative mood ("add feature", not "added feature"). Keep under 72 chars, no trailing period.
- **Description**: **REQUIRED** for non-trivial changes (explain the "why", breaking changes, and context). Optional for simple typos or minor docs updates.

### Examples

#### Good Commits
```
✨ feat: add user authentication with JWT tokens

Implements token-based auth using jsonwebtoken library.
Tokens expire after 24 hours and are stored in httpOnly cookies.
Adds middleware to protect routes requiring authentication.
```

```
🐛 fix: prevent memory leak in WebSocket connections

WebSocket connections were not being properly closed on
component unmount, causing memory to accumulate over time.
Added cleanup function in useEffect return.
```

#### Bad Commits
```
update stuff
```

```
feat: add auth and refactor database and update docs
```
