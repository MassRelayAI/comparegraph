---
title: The README and docs, judged by a persona jury, fixed, and judged again
date: 2026-10-24
valid_until: 2027-04-24
lane: readme front door
verdict: "Seven composite personas judged README.md, usage, FAQ, troubleshooting and the charts under three lenses (critical, docs, compare against two sibling projects' READMEs). Round 1 raised 22 findings: 16 were fixed in the docs, 4 became issues and 2 were dismissed. Round 2 judged the edited pages. All three deltas landed. Critical blockers went from 15 to 6 (primary 8 to 3, secondary 6 to 3, bystander 1 to 0), and the compare blockers from 6 to 0. Docs medians rose to 4 on all four dimensions for primary, except run-it, which stayed at 3. The forced pick went from 4 of 7 for this README to 7 of 7. The 6 round-2 findings are code or process issues, not docs."
status: decided
---

# The README and docs, judged by a persona jury

**What was judged.** The pages a newcomer reads:
- `README.md`, `docs/usage.md`, `docs/faq.md` and `docs/troubleshooting.md`;
- PNG renders of the five README charts;
- a screenshot of the README's first screen as GitHub renders it.

The jurors judged files, never a live page.

**Who judged.** Seven composite personas, invented roles and not real people, in three strata:
- **primary (would use it):** an OSS library maintainer, a platform engineer doing vendor due diligence, an author of agent skills, and a first-month newcomer with one evening;
- **secondary (approves it):** an engineering manager and a security reviewer;
- **bystander (passes by):** a newsletter curator who reads the first screen only.

**How.** The run used [PersonaJury](https://github.com/massrelay-ai/personajury) v0.2:
- one rule: the public surface, the panel above, three lenses, compact briefs;
- 21 briefs per round, about 315,000 tokens by the tool's own estimate of 15,000 per brief;
- one Sonnet subagent per persona, each in a private folder;
- an Opus foreman, who proposed one outcome per finding and made the edits.

The tool calls no model; the jurors and the foreman are the agent's subagents. Every verdict line that claims to have
seen something cites a file and line. A verdict that cites nothing is refused and the persona answers again. Scores are
reported per stratum and are never averaged across strata.

**Comparators.** The READMEs of code-review-graph and CodeGraph, held as snapshots: their markdown and a render of their
first screen.

## Round 1: what the jury said
Medians per stratum (primary / secondary / bystander), on a scale of 1 to 5:

| lens: dimension | round 1 | round 2 |
|---|---|---|
| critical: works | 3 / 3 / 2 | 3 / 4 / 4 |
| critical: blocks (5 = nothing stops me) | 2 / 2 / 2 | 3 / 2 / 4 |
| critical: missing | 3 / 2 / 2 | 3 / 3 / 3 |
| docs: find-it | 3 / 3 / 2 | 4 / 4 / 4 |
| docs: run-it | 3 / 3 / 2 | 3 / 4 / 3 |
| docs: trust-it | 4 / 3 / 3 | 4 / 4 / 4 |
| docs: next | 3 / 3 / 3 | 4 / 4 / 4 |
| compare: clarity (this README vs. the two) | 2 / 3 / 1 | 4 / 3 / 4 |
| compare: price (what it costs me to start) | 3 / 3 / 3 | 4 / 4 / 4 |
| compare: trust | 4 / 3 / 2 | 4 / 3 / 2 |

The forced pick (which project would you try first):

| | primary | secondary | bystander |
|---|---|---|---|
| round 1 | this README 3, code-review-graph 1 | this README 1, code-review-graph 1 | code-review-graph 1 |
| round 2 | this README 4 | this README 2 | this README 1 |

The round-1 findings, grouped:
- **The first screen** was 27 badges and an agent list before any sentence saying what the tool does. A passer-by left
  before the first sentence. The competitor opens with a picture of the problem.
- **The first run:**
  - the sample walk assumed a clone of this repository without saying so;
  - nothing said how long a first result takes;
  - nothing said how to compare your own repositories rather than the shipped sample family.
- **Trust:**
  - the install line piped a script from `main` into a shell;
  - the docs did not say what the tool does with a repository you do not trust;
  - nothing said whether the "no network" claim is enforced;
  - nothing said whether the attestation is checked.

## What changed
Sixteen findings were decided `fixed`:
- **The first screen:** one plain sentence, three badges, and a new hero picture. It shows the table the tool produces,
  each cell pointing at the code. It is drawn from the cells both readers agreed on in `fixtures/gold-map/`.
- **Install:** a read-before-you-run route pinned to `v0.1.1`, with what the script does in words.
- **The sample walk:** it starts at `git clone`.
- **Your own repositories:** a section on comparing them.
- **A measured time to a first result:** [first-result-time](2026-10-24-first-result-time.md).
- **FAQ answers:**
  - free, no paid tier;
  - where support is;
  - how to check the "no network" claim yourself;
  - what happens with an untrusted repository (symlinks not followed, size and file caps);
  - that the install scripts verify `SHA256SUMS` and not the attestation.

Four were decided `row`. They are true and not docs problems, so they are now issues:
- held-out mapping recall (already disclosed);
- no CI test that enforces "no network";
- no hash pin for plugins;
- no SBOM or reproducible-build statement.

Two were `dismissed`: both were about a comparator's README, not this one.

## Round 2
The same seven personas, drawn by the same seed, judged the edited pages.
- **`delta` landed on all three lenses.** Every cell of round 1 was judged again, no stratum's blockers rose, and no
  score fell by more than one anchor. Two jurors first left some comparator cells unscored. The tool refused to call
  the compare delta landed until they scored them.
- **`check` passes.** All 16 fixes were judged again by the personas who raised them, and none was raised again.
- **Six findings remain, all decided:**
  - four `row`: drafting a family pack from plain words, map ranking quality, the CI no-network test and the SBOM. The
    last two repeat round-1 issues;
  - two `dismissed` with the line that answers them:
    - `none` carries its search by design (faq.md);
    - `setup` without `--apply` prints the plan, and `--client` names one agent.

## What this shows, and what it does not
- **The front door was the biggest problem, and the cheapest to fix.** Clarity against the two comparators went from
  2 to 4 for the primary stratum and from 1 to 4 for the bystander. The engine did not change.
- **The secondary stratum still says `blocks: 2`.** The security reviewer's points are real gaps: CI enforcement, SBOM,
  and what `setup` writes. Better words cannot close them, only code.
- **Scores are relative to the options in the room.** CompareGraph is compared here with two specific READMEs.
- **Seven composites are not a survey.** They show where to look; they do not replace real users.
