# TODO

## spore-moss integration

### Not yet implemented

**`moss.analyze.security(path)`**
- Calls external tools (bandit for Python, etc.) which may not be installed
- Would need graceful fallback or feature detection
- Consider: should agents even run security scans, or is that a human-initiated action?

**`moss.analyze.docs(path, opts)`**
- Requires moss file index to be set up for cross-file interface resolution
- Without index, falls back to on-demand parsing which is slower
- Need to decide: should spore-moss manage its own index, or expect one to exist?

### Future considerations

- Batch edit support (`moss.edit.batch()`) - moss has BatchEdit for atomic multi-file edits
- Call graph queries (`moss.analyze.callers()`, `moss.analyze.callees()`) - requires index
- Duplicate detection (`moss.analyze.duplicates.functions()`, `moss.analyze.duplicates.types()`)
