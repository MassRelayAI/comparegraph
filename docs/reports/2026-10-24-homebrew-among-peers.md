---
title: Homebrew among the code-graph tools we compare with
date: 2026-10-24
valid_until: 2027-01-24
lane: distribution
verdict: "Homebrew is common but not universal: 6 of 11 code-intelligence peers offer it (4 through their own tap, 1 through a formula in the main repo, 1 in homebrew-core); the two largest, colbymchenry/codegraph (75,167 stars) and tirth8205/code-review-graph (32,363), do not, and codegraph created an empty tap repository and ships an install script instead. Where a project offers brew, a separate tap is the norm (5 of 6); a formula inside the main repository appears once. Recommendation: keep Homebrew as one shared tap for all our tools (rename massrelay-ai/homebrew-comparegraph to massrelay-ai/homebrew-tap), not a formula in the main repository."
status: open
---

# Homebrew among the code-graph tools we compare with

## The question
The maintainer asked whether `massrelay-ai/homebrew-comparegraph` can be removed, why the formula does not live in the main
repository, and whether other code-graph tools ship Homebrew at all.

## Method
For each of the 14 repositories cloned under `comparegraph-study/` (their `origin` remotes), `gh api` on 2026-10-24 checked:
a tap repository (`homebrew-<name>`, `homebrew-tap`, `homebrew-<owner>`, `homebrew-formulae`), a `Formula/` or
`HomebrewFormula/` directory in the repository, a formula at `Homebrew/homebrew-core/Formula/<letter>/<name>.rb`, and the
first `brew install|tap` line of the current README. Star counts are from the same call.

## Result
| repository | stars | Homebrew | README line |
|---|---|---|---|
| colbymchenry/codegraph | 75,167 | none in use: `homebrew-codegraph` exists but is **empty** (last push 2026-06-22) | install script (`curl … \| sh`, `install.ps1`) |
| tirth8205/code-review-graph | 32,363 | none | — |
| bartolli/codanna | 756 | own tap `homebrew-codanna` **and homebrew-core** | `brew install codanna` |
| CodeBendKit/codeseek | 762 | formula **in the main repository** (`Formula/`) | `brew tap CodeBendKit/codeseek` (two steps) |
| aovestdipaperino/tokensave | 677 | shared tap `homebrew-tap` | `brew install aovestdipaperino/tap/tokensave` |
| Muvon/octocode | 513 | shared tap `homebrew-tap` | `brew install muvon/tap/octocode` |
| aouicher/graphmind | 216 | own tap `homebrew-graphmind` | `brew install aouicher/graphmind/graphmind` |
| codegraph-ai/CodeGraph | 132 | none | — |
| srclight/srclight | 58 | none (Python) | — |
| Graphify-Labs/graphify | 134,136 | none (Python) | — |
| redhat-et/ripwire | 2,472 | none | — |
| caddyserver/caddy (reference, not a peer) | 78,341 | homebrew-core | — |

Counts over the 11 code-intelligence peers (caddy, hono and rea left out as non-peers): Homebrew offered by 6 (4 own or
shared tap, 1 in-repo formula, 1 tap plus homebrew-core); not offered by 5, including the two largest.

## Reading
- A separate tap is how 5 of the 6 brew-offering peers do it; the in-repo formula (codeseek) costs users a two-line install
  and puts release-bot commits on the main branch.
- Two peers already use the shared `homebrew-tap` shape for one owner's tools — the shape that fits CompareGraph and
  PersonaJury together.
- codanna shows the end state: its own tap first, homebrew-core later, after which no tap is needed.
- The largest peer never filled its tap; the install script carries it. Homebrew is a convenience, not a requirement.

## Options
| option | cost | gives | forecloses |
|---|---|---|---|
| A. keep `homebrew-comparegraph` as is | none | brew for CompareGraph | a second tap repository per tool |
| B. formula in the main repository | move the formula, two-line install, a bot commit per release on protected `main` | one fewer repository | the one-line install |
| C. drop Homebrew | delete the repository, remove the README/docs lines | one fewer surface | brew users |
| **D. one shared tap** (`homebrew-tap`) | a rename (GitHub redirects the old name), the README/docs lines, later a PersonaJury formula on request | one repository for every tool | nothing |

## Recommendation
D: rename to `massrelay-ai/homebrew-tap` and change the install line to `brew install massrelay-ai/tap/comparegraph`; add a
PersonaJury formula only when someone asks for brew (ship then learn). Revisit homebrew-core when CompareGraph is notable
enough for it. The rename is the maintainer's.

## Detector
After the rename, `brew install massrelay-ai/tap/comparegraph` on a clean macOS runner, and the old
`brew install massrelay-ai/comparegraph/comparegraph` still resolving through the redirect; the release workflow's tap update
step must target the new name or the next release leaves the formula stale.

## Regenerate when
A peer changes its install route, or the maintainer decides; rerun the method above.
