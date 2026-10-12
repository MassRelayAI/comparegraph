# Gold call labels (Plan 5, A6)

One `calls.jsonl` per project: 150 call sites each, 750 in all. Project sources are not committed, only the labels with `path:line`
(paths are relative to the project root at the pinned head).

## Draw

150 sites per project, seed 11, drawn at random from the project's own source (not from either engine's output): a per-language
regex finds call-shaped text, the first call per line is kept, then the 150 are sampled. Row shape:
`{id, path, line, callee, reader1: {is_call, target}, reader2: {is_call, target}, final: {is_call, target} | null}`.
`final` is the label both readers agree on (is_call equal and, for calls, targets equal after normalising path separators and dropping
`#n` suffixes); it is `null` where they disagree, and those rows are left out of the final scores.

## Rubric

- `is_call`: `yes` if the text at the line is an executed call or instantiation of `callee`; `no` for text inside a string or
  docstring, embedded code of another language, a pattern in match or if-let, a definition, or the keyword `let` caught by the regex.
- `target` (calls only): the qualified name `repo/relative/path::name` of the definition the call reaches; `external` for the standard
  library or a third-party package; `unknown` when it cannot be decided statically (dynamic assignment, a value returned by a fixture).
  `target` is `null` when `is_call` is `no`.
- The two readers worked independently and did not see each other's labels.

## Upstream projects and heads

| project | upstream | head |
|---|---|---|
| graphmind | https://github.com/aouicher/graphmind | 4a90a7e |
| srclight | https://github.com/srclight/srclight | 23924ff |
| code-review-graph | https://github.com/tirth8205/code-review-graph | 6b12d11 |
| hono | https://github.com/honojs/hono | 552ea02 |
| caddy | https://github.com/caddyserver/caddy | d293dcf |
