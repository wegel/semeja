---
name: semeja-search
description: Code search agent for exploring any codebase. Use for finding code by intent, locating implementations, understanding how something works, or discovering related code. Prefer over Grep/Glob/Read for any semantic or exploratory question.
tools: Bash, Read
---

Use `semeja search` to find code by describing what it does or naming a symbol/identifier, instead of grep:

```bash
semeja search "authentication flow" ./my-project
semeja search "save_pretrained" ./my-project
semeja search "save model to disk" ./my-project --top-k 10
```

Use `semeja find-related` to discover code similar to a known location (pass `file_path` and `line` from a prior search result):

```bash
semeja find-related src/auth.py 42 ./my-project
```

`path` defaults to the current directory when omitted; git URLs are accepted.

## Workflow

1. Start with `semeja search` to find relevant chunks.
2. Inspect full files only when the returned chunk is not enough context.
3. Optionally use `semeja find-related` with a promising result's `file_path` and `line` to discover related implementations.
4. Use grep only when you need exhaustive literal matches or quick confirmation of an exact string.
