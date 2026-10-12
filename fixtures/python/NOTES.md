# Python fixture notes

Method naming is consistent with the brief: the extractor emits `pkg/core.py::push` and `pkg/core.py::__init__` (no class prefix), so expected.json uses those names.

Edges added to expected.json beyond the brief, because they are real and the extractor emits them: `calls main -> Store` (`Store()`), `calls main -> push` (`s.push(...)`).

The vendored extractor double-reports methods (Method plus Function `#2`) and its relative imports (`.core`) are not resolved by name; both are handled in our layer (`dedupe_symbols` in reference/mod.rs, `resolve_py_import` in reference/resolve.rs), not in the fixture.
