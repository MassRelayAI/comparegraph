# TypeScript fixture notes

Names match the brief (`src/util.ts::inc`, no `Counter.` prefix). Two real call edges beyond the brief were added to expected.json because the extractor emits them correctly: `calls main -> Counter` (`new Counter()`) and `calls main -> inc` (`c.inc()`).
