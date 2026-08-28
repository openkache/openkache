# OpenKache scripts

## Trilingual documentation enforcement

`check_doc_i18n.py` enforces that English, Korean, and Chinese versions of
registered documents stay in sync. It checks staged files before each commit
and rejects the commit if:

- **co-change violation:** only one language was edited (e.g., you fixed a
  typo in `README.md` but forgot `README.ko.md` and `README.zh.md`)
- **structural parity violation:** the three files have different numbers of
  `##` / `###` headings or fenced code blocks, or one is missing the
  language-switcher line

The checker runs from the `pre-commit` hook. Install it once per clone:

```bash
./scripts/install-hooks.sh
```

Emergency bypass (use sparingly):

```bash
git commit --no-verify
```

### Registered document sets

Currently enforced (see `DOC_SETS` in `check_doc_i18n.py`):

- `README.md` · `README.ko.md` · `README.zh.md`

To add a new document set (e.g., `ROADMAP`), append a tuple to `DOC_SETS`.
