# Rhipper Agent

**The endgame, and the road to it.** The heading above is the project's own title and nothing more, because `Store::import` gives the project the outline's title. This file starts where `docs/fable/agent-roadmap.md` stops. That roadmap's nine milestones make today's agent cheap, honest and durable; this file says what the agent is for once that is true, the architecture that gets it there, and thirteen phases past the roadmap. The first three phases are written as cards; the ten after them are milestones and sections with one-line objectives, which the walk plans itself when it reaches them.

The file imports whole with `rc import`: everything outside the outline is prose at the margin with no list marks, so the parser in `crates/ra-roadmap/src/outline.rs` skips it. The outline is the last third of the file.

Three kinds of claim are made here and they are worth telling apart. A claim with a function or a line number was read in the code at commit 787be7f. A claim with a percentage of today's runs is one of the five reviews' and was not measured again. Every number in the future tense is a target I am setting, not a measurement; each phase names the measure that will show whether it held.

#### The endgame in one paragraph

The person says what they want and reads what was proven. Between those two acts nothing needs them. The agent turns a sentence into a plan whose every card a machine can check, predicts what each card will cost and whether it will pass before it spends a run on it, writes the failing test before the code, sends the work to the cheapest model on the nearest machine that its own record says can do it, proves the result by its tree, lands it, and keeps the whole run as a case it can replay. It changes itself every day, on the person's own fork, and no change reaches a walk until it has beaten the installed build on those cases and survived a canary on the agent's own roadmap.

The design under all of it is one sentence: **judgment moves to before the run, proof moves to after it, and models only write.** Today a model decides whether a card is runnable (by failing at it), whether the work is right (by reading a diff) and what to do next (by a look-back every ten landings). In the endgame a machine decides each of those from a record, and a model is called where only a model will do: to write a plan, a test, a change, or an answer to a question nobody has proven.

#### A normal day

**07:40.** The person opens the window. It shows one screen, the same on the desktop and the laptop, because both read the same hub.

```
rhipper · Friday 07:40 · since 22:10 yesterday
landed     31   agent 9 · runelite 14 · asm 8          29 passed first time
proven by  24 machine verdicts · 7 reviewer verdicts · 0 escapes traced this week
spent      4.1M tokens, 63% local · 11% of the week's room · 2 machines
changed    rc 0.41.3 promoted at 03:12 (bench 212 better, 3 same, 0 worse; canary 12 landings)
           rc 0.41.2 rolled back at 01:40 (canary: turns a run up 18% over 6 landings), now card #2291
needs you  2 questions, 1 approval                      about 4 minutes
```

Nothing on it is a diff. A landed card opens to its verdict: the numbered criteria of its contract, and beside each the proof: the test that was red on the base and is green on the landing, the changed lines that test executes, the mutants it killed. Seven cards had a criterion no test could carry ("the error message reads well"), so a reviewer model ruled on that criterion alone and its sentence is there with the line it cites.

**07:42.** The two questions. Each is a typed thing with options, a default, and the hour the default is taken if nobody answers, so a question never stops a walk for longer than the person chose.

```
Q1 · runelite #1873 · asked 02:15 · default at 12:00 is A
The overlay cache can key on the widget id or on the widget's hash. The id is 40 lines and
goes stale when a plugin reloads; the hash is 110 lines and never does. Like cards: 6, all
landed with the hash, 4 of them after a rerun on the id.
A  key on the hash (default)      B  key on the id      C  send a scout and ask me again
```

The approval is a card on the agent's own roadmap that touches a protected path: it changes what the pre-tool hook lets a worker run. The agent may write that change and prove it; it may not land it alone. The person reads one sentence of what widens, the bench's count, and types `rc approve 2288`.

**08:00.** The person wants something new and says it.

```
> rc intend runelite "reload a plugin without restarting the client"
plan P-88 · 14 cards in 3 sections · 9 can start now
predicted   1.6M tokens (71% local) · 2 h 50 min on two machines · 12 of 14 pass first time
risky       #5 "the class loader lets go of the old plugin"  0.46, so a scout runs first (12 turns)
            #11 "a reload keeps the plugin's config"          0.58, so it is split in two
asks        does a reload keep unsaved config, or drop it?    cards 11 to 13 wait on the answer
approve with `rc approve P-88`, or edit plans/P-88.md and approve
```

The plan is markdown in the roadmap's own format. The person reads fourteen titles, answers the question, moves one card up, approves. They did not write an acceptance, pick a model, or set a wait; the plan arrived with all three, and the gate had already run every check on the base to see it red.

**10:30, on a train.** The laptop's window shows the walk on the desktop and the GPU tower at home. Fourteen seats are busy: five on the Max plan's best model writing specs and the two hard cards, nine on the tower's local coder turning red tests green. The person asks why a card is waiting.

```
> rc why 1904
#1904 waits · 6 min
because  its check needs the build slot #1899 holds (rank 2 of 3, about 2 min left)
because  #1899 and #1904 both change `Overlay::paint`; the planner put the smaller one first
then     a seat on `tower` (local coder): its spec is written and red; like cards pass there 0.91
events   e88213 Decided(schedule) · e88240 Leased(slot 3) · e88102 Predicted(route)
```

The lid closes. Nothing is lost, because the laptop held nothing but a view.

**14:10.** The remote provider has an outage. The walk says so once. Cards whose spec is written and whose predicted pass on a local model is over the bar move to the tower; the two that need the best model wait with that reason; nothing is retried into a wall. At 14:50 the provider is back and the two cards run.

**16:20.** The person pushes a commit of their own to runelite's main and it breaks a test. Four minutes later a question arrives: the failure, the commit it bisected to (theirs), a revert already proven green and a fix card already written, and a default of "fix card at 17:00". No worker was sent to fail against the red main in the meantime; twelve cards whose checks never touch the broken crate kept landing.

**22:00.** The day's walks are done and 30% of the week's plan-window room is unspent, with two days to go. The agent spends some of it on itself: a live replay of forty old cards under a candidate build whose map ranks ranges differently, because the scripted bench said "better" with less confidence than a promotion needs. In the morning there is one line about it.

The person spent eleven minutes, answered four questions, and read no diff. That is the product.

#### The eight capabilities

##### 1. It knows the code as a graph, not as files

Today the worker greps. After the roadmap it is handed a map: ranked line ranges found by words. In the endgame the agent holds an **atlas** of every project: every item (function, type, test, config key), every reference between them, keyed by the hash of the blob it was parsed from, so a commit re-parses only what it changed. Four overlays sit on the graph, all written from the record: which landed cards changed an item, which tests execute it, what it has cost (reruns, conflicts, escapes that passed through it), and the notes and invariants runs have left on it.

Four questions are asked of it, and each feeds one decider. `where(card)` is the map a worker starts with. `impact(diff)` is the set of tests worth running. `overlap(a, b)` says whether two cards can run side by side, by item rather than by file. `like(card)` finds the landed cards that most resemble a new one, which is what every prediction rests on. `rhipper.context.md` stops being written by hand and becomes a rendering of the atlas.

##### 2. It plans from intent, and a plan is a checked object

An intent is a sentence, a gambit, an issue, a failing test, or evidence the agent found in its own record. The planner turns it into a card graph: each card with a contract whose checks the gate has already run on the base and seen red, waits derived from the atlas, a size and a first-pass probability from like cards, and the questions that have to be answered before work starts rather than after it fails. A card predicted to fail is not dispatched to find out: it is split, or a scout is sent to read first, or the question is asked.

##### 3. It schedules seats, slots, models and machines as one problem

The hub becomes a small daemon on the person's main machine. Every machine they own joins as a node offering what it has: seats for workers, build slots, a GPU with a local model server, a plan window with a provider. The scheduler is the pure `plan` of the roadmap's ninth milestone grown one dimension: the same function, with a node and a route in its answer and a reason for every wait. A route is chosen from a fitted table, not a rule: for this kind of card, on this project, the cheapest model whose measured first-pass rate holds the bar. Local comes first because the table prices it that way, and the bar is why local-first never means worse.

##### 4. It proves work, and a proof is kept

A landing carries a proof, keyed by the hash of the tree and a stamp of the toolchain, and anyone can ask for it again with `rc verify <commit>`. The proof is a ladder, and each rung removes a reason to read the diff. The check ran (roadmap). The check was red on the base and is green on the tree, so it tests something. The new tests execute the changed lines, so they test this change. A sample of mutants in those lines died, so the tests would notice a wrong change. Where the project declares them: a property holds, an old and a new implementation agree, a benchmark did not regress. A reviewer model is the last rung and is asked only about criteria no rung above could carry.

The proof system has its own measure: the **escape rate**, bugs later traced to a card that passed. Every other measure of quality can be gamed by proving less; this one cannot.

##### 5. It learns from every run, and the lesson is measured

Every landed card is a **case**: the base commit, the card as it was sent, the landing, the proofs, the calls the worker made. The cases are the agent's benchmark, made of the person's own work, growing by one with every landing. A change to how the agent maps, selects, routes, schedules or prompts is replayed over them before it is installed, in three tiers: scripted (no model call, seconds), recorded (a stored run replayed against new hooks and prompts, no model call), and live (a sample re-run for real in a throwaway worktree, paid from the room the week would otherwise leave unspent).

On that base everything else that is called learning becomes arithmetic the person can read. An estimate is the median of like cases, with its error. A route is a table of pass rates by kind of card and model, with its counts. A note from a retro is a hypothesis: it is given to four runs in five that it fits and withheld from the fifth, and it is kept while the four do better. A local model is trained on the person's own landed work, on their own GPU, for the roles where the bench says a small model is enough, and it takes a role only by beating the incumbent on cases it never saw.

Nothing is learned from a run by asking a model what it thought of it. Every run adds one row to each of these, and each table has one reader.

| What the run showed | The table it is a row of | What reads the table |
|---|---|---|
| Predicted and actual turns and tokens | The estimate's error, by kind and size of card | The estimator; the channel's guard |
| Passed first time or not, on which route | A cell of the route table | The scheduler |
| The map's pointers, and the ranges then edited | The map's hit rate; the weights of `where` | The atlas |
| The checks selected, and the checks that failed | The selection's misses | The prover |
| Each note and hook rule given or withheld, and the outcome | That note's or rule's effect | The learner, which retires it |
| The items changed, the minutes, the conflicts | The cost overlay of the atlas | The planner and the scheduler |
| The rungs of proof passed; later, an escape or none | The escape rate by rung | The prover; the guard |
| Each question, its answer and the time it took | The attention ledger; proposed standing rules | The inbox |
| The whole run | A case | The bench; the dataset of phase 18 |

##### 6. It explains itself from the record

Every state in the window has a cause chain that ends in events, and `rc why` prints it. `rc ask` answers a question in prose with the events it read. The person's attention is treated as the scarcest budget the agent has: every question it asks and every minute it takes is in a ledger, with a weekly figure of minutes for each hundred landed cards, and a question the person answers the same way three times becomes a rule the agent proposes to stop asking.

##### 7. It recovers from anything, and has practiced

The record is append-only and every stage of a run is idempotent, so a killed walk, a lost machine and a full disk are the same event: a lease expires, another node reads the record and takes up the stage. A red main is bisected to its landing and a proven revert is ready before the person asks. A bad build of the agent is rolled back by the channel. A note that made runs worse retires itself. `rc undo` takes back a landing with a proof that the tree after it is green. And none of this is trusted on argument: a drill kills a walk, a node or a provider on a schedule and the measure is the seconds until work resumes.

##### 8. It improves itself safely

The loop exists today in one sentence: the person runs a local fork, `rc install; rc restart`. The endgame keeps the fork and removes the sentence. Friction the agent meets in any project becomes a card on its own roadmap, with the evidence and a predicted saving. The card lands like any other. The build then earns its install: the bench, then a shadow run in which the new deciders read the live record and their differences are logged, then a canary walk on the agent's own project, then every project, with guard measures that roll it back without anyone asking.

Above the loop sits a short **charter**: the things the agent may never change alone. No landing without a proof. Never a forced push. A worker cannot widen what its own hook allows. The holdout cases are never read by anything that tunes. The guard thresholds, and the charter itself. A card that touches one of these is written and proven like any other and then waits for the person.

#### What no tool does today that this one would

A claim about other tools is a claim about what I know to be public as I write this in November 2026. I did not survey the field for this file, and a row that turns out to be wrong is a row someone else has shown to be possible.

| What | Why nobody has it | What it rests on here |
|---|---|---|
| Its own history is its benchmark. Every card it ever landed is a case, and a change to the agent is scored on the person's code before it touches the person's code. | Agents are scored on public benchmarks that are not your code, by their vendor, before release. | The record, the bench (phase 10). |
| It changes itself on the person's fork and each change earns its place: bench, shadow, canary, rollback, under a charter it cannot edit alone. | Self-improving agents either cannot change their own harness or change it with nothing but the next run to judge by. | The channel (phase 11). |
| A landing carries a proof that its tests would catch a wrong change, and the proof system is graded by escapes. | Tools stop at "the tests pass" and a model's opinion of the diff. | The proof ladder (phase 12), escapes. |
| It says what a card will cost and whether it will pass before it runs, and declines the run when the answer is no. | That needs a record of like work with outcomes; a tool without a roadmap and a ledger has neither. | The estimator (phase 13). |
| Work migrates to local models by measurement, role by role, including models trained on the person's own landed work on their own GPU. | Local routes exist as a setting; nothing measures where they are good enough, and nothing closes the loop with training. | Fitted routes (13), local models (18). |
| One scheduler for seats, build slots, plan windows, GPUs and machines the person owns, with no hosted control plane. | Hosted agents schedule in their cloud; local ones schedule within one machine and one provider. | The hub daemon (phase 17). |
| The person's attention is a measured budget, and questions carry defaults and deadlines. | Tools ask for approval per action or not at all. | The inbox (phase 19). |
| An improvement made on one fork travels to another with its evidence and none of its owner's code. | There is no unit of "a proven change to an agent" to trade. | The exchange (phase 21). |

#### The measures of the endgame

| Measure | Today (the reviews) | After the roadmap (its targets) | Endgame (my targets) |
|---|---|---|---|
| Runs that were reruns | agent 69%, runelite 85%, asm 97% | 50%, 60%, 50% | 10% or less in every project |
| First verdicts passed | 71%, 14%, 8% | 85%, 50%, 50% | 95% or more |
| Landings whose diff no reviewer read | none | none | 70% or more |
| Escapes for each 100 landings | not measured | not measured | 1 or fewer, and measured |
| Median tokens a turn | 55k | 35k | 15k or less |
| Turns of a c3 first run | 47 to 65 | 25 or fewer | 12 or fewer, after a spec pass of 8 |
| Tokens run on the person's machines | about none | cheap roles (live card #200) | half of all tokens, every role but the hardest writing |
| Run's end to landed | about 4 min (2043 s over 8 landings) | 60 s or less | 20 s or less |
| A killed walk resumes | by hand | at its stage | in 60 s on the same machine, 5 min on another |
| Builds of the agent installed unproven | all | all | none |
| The person's minutes for each 100 landed cards | not measured | not measured | 30 or fewer, and measured |
| Diffs the person had to read | most | few | none |

##### What a landed card costs, by the reviews' own arithmetic

The efficiency review's model is that a run's input is about its turns times the context of a turn, and a landed card costs that times the runs it took, which is one over one minus the rerun rate. The table applies it to a c3 card of the agent's own project. It is arithmetic on the medians and the targets above, not a measurement, and a rerun is usually shorter than a first run, so the first column overstates. The ratios between the columns are the point.

| | Today | After the roadmap | Endgame |
|---|---|---|---|
| Turns of a run | about 55 | 25 | 8 for the spec, 12 for the code |
| Tokens a turn | 55k | 35k | 15k |
| Tokens a run | 3.0M | 0.9M | 0.3M |
| Runs for each landed card | 3.2 (69% reruns) | 2.0 (50%) | 1.1 (10%) |
| Tokens for each landed card | about 9.8M | about 1.8M | about 0.33M |
| Of them sent to a remote model | all | nearly all | about 0.14M: the spec, and one code pass in ten |
| Reviews of the diff for each landed card | 3.2 | 2.0, on the delta | 0.3, and one short ruling on the spec |

The roadmap is worth about five times. The endgame is worth about five times again in tokens and about twelve times in tokens that leave the person's machines, and almost all of its share comes from three things the roadmap cannot do: a context that shrinks (phase 15), a card that is not run when it would fail (phases 12 and 13), and a writer that is local because its task was made small (phases 12 and 18).

#### The person's five values, and where each ends up

| Value | Where it lives in the endgame |
|---|---|
| Efficiency and local execution everywhere | The route table prices tokens, wall time and the week's room together and picks the cheapest route that holds the bar; the hub, the record, the atlas, the bench and the training all run on the person's machines and need no service. |
| A card that runs once and passes | A card is dispatched only with a red check and a predicted pass over the bar; the rest are split, scouted or asked about first. The measure is the rerun rate. |
| Reading verdicts, not diffs | The proof ladder leaves the reviewer, and then the person, only what no machine proved. The measure is landings whose diff no reviewer read, beside the escape rate. |
| Work that helps one project is done where every project gets it | Every learned table is pooled across projects at the hub; friction found anywhere is a card on the agent's roadmap; the bench holds every project's cases. |
| A self-improvement loop on the person's own fork | The channel: the fork's own cards, benched on the fork's own cases, canaried on the fork's own roadmap. The exchange lets forks trade what they proved. |

#### The architecture

##### Intent, fact, and pure deciders

There are two stores. The **roadmap** is intent: what should be. It stays the person's markdown file, `rhipper.md`, in the form it has today. The **record** is fact: what was. It is the typed event log of the roadmap's seventh milestone, grown until nothing the agent knows about its past lives anywhere else. Everything between them is one of three things: a **fold** (a pure function from the record to a view: the board, the usage table, the cases, the policy tables), a **decider** (a pure function from the roadmap, the atlas and folds to a decision, which is itself written to the record with its inputs), or an **effect** (a worker run, a build, a git operation: the only parts that touch the world, each a stage that can be run twice safely).

That split is what makes every capability above cheap. Replay is calling a decider on old inputs. Shadow is calling two builds' deciders on the same inputs. Recovery is re-reading the record. Explanation is printing the inputs of a decision. Many machines is many writers of the record.

```
              the person
      intent   |      ^   verdicts, questions
               v      |
   +------------------------+          +------------------------+
   |  ROADMAP   rhipper.md  |          |  WINDOW   rc, rc-gui   |
   |  what should be        |          |  folds and nothing else|
   +-----------+------------+          +-----------^------------+
               |                                   |
               v                                   |
   planner > gate > estimator > scheduler > runner > prover > lander
   (deciders are pure; each decision goes to the record with its inputs;
    only the runner, the prover and the lander touch the world)
               |                                   ^
               v                                   |
   +-----------------------------------------------+------------+
   |  RECORD   append-only typed events, one log for each       |
   |  project and machine; cases; proofs by tree; policy tables |
   +---------^-------------------------------------^------------+
             |                                     |
        the ATLAS                           the BENCH and the LEARNER
   code graph by blob hash             replay, experiments, fitted tables,
   with overlays from the record       and the channel for the agent's own builds
```

##### The laws

Ten sentences hold the design together. A card that breaks one is wrong whatever it measures, and the first, fifth, ninth and tenth are in the charter.

1. The roadmap is written by the person and by the planner through the gate, and by nothing else.
2. Every fact is an event. A view that cannot be rebuilt from the record is a bug.
3. A decider is pure, and its decision is recorded with its inputs.
4. An effect is a stage that can be run twice.
5. Nothing lands without a proof of its tree, and main moves only forward.
6. A model is never asked what a machine can decide.
7. A change to the agent is judged on cases before walks, and on the agent's own project before any other.
8. Every learned number is shown with its count and has a rule to fall back on.
9. Nothing leaves the person's machines but the prompts of remote routes, and those are counted.
10. The charter changes only by the person.

##### The budgets

A design this size stays fast only if each part has a bound it is tested against. These are bounds I am setting; each becomes an acceptance of the phase that builds the part.

| Part | Bound |
|---|---|
| The scheduler's `plan` | 50 ms for 500 open cards, since it runs on every event |
| The window's snapshot | 100 ms to fold, and never computed in the window |
| The first prompt of a code pass | 6k tokens: the contract, the map, the failing test |
| `rc bench`, scripted | 300 s for every case of every project |
| The atlas after a commit | 2 s for a commit of 20 files; nothing re-read whose blob did not change |
| The record | 20 kB a run without its stream; a year of three projects under 1 GB |
| Seeing a card's base | one run of one filter, from a warm target, shared by every card with that check |
| A run's end to landed | 20 s |
| A lease | renewed every 30 s, taken over after three missed |
| The spec pass | 12 turns at most, 8 at the median; skipped where `wants_spec` says it would not pay |

##### The components

| Component | Crate | What it owns | Grown from |
|---|---|---|---|
| The roadmap | `ra-roadmap` | Nodes, states, waits, gambits, the outline form, the gate (`admit`), the contract (`contract_of`) | Stays. Gains waits in the outline and plans as files. |
| The record | `ra-record` (new) | Typed events, one append-only log for each project and machine, the folds (`Board`, usage, cases), the run ledger | `events.rs`, `events/board.rs`, `walk/ledger.rs` of the roadmap; `ra-usage` becomes a fold of it. |
| The atlas | `ra-atlas` (new) | Items and references by blob hash; the history, coverage, cost and note overlays; `where`, `impact`, `overlap`, `like` | `ra-context/src/map.rs` and `ra_check::select` become two of its queries. |
| The prover | `ra-check` | Checks, the ledger by tree hash, blame, selection, red-on-base, changed-line execution, mutants, the proof bundle and `rc verify` | Stays and grows; the roadmap creates it. |
| The planner | `ra-plan` (new) | Intent to card graph, questions, splits, scouts, derived waits | `ra-console/src/plan/` and `lookback.rs`. |
| The policy | `ra-policy` (new) | Estimates, first-pass probability, the route table, each a fitted table with counts, with the person's rules as overrides | `router.rs`, `route.rs`, `Estimate` on the node. |
| The scheduler | `ra-run` (new) | The pure `plan` over cards, seats, slots, nodes and routes, with a reason for every wait | `walk/schedule.rs` of the roadmap. |
| The runner | `ra-run` (new) | A run as named stages on disk; worktrees; the Claude Code seat; the native tool loop with a context the orchestrator owns | `run_task` and `tool_loop` in agent.rs, `ra-tools`, `walk/landing.rs`, `walk/merge.rs`. |
| The gate on tools | `ra-gotchas` | Pre and post tool hooks, the catalogue, a card's capabilities, the charter's protected paths | Stays; rules gain a measured precision from replay. |
| The hub | `ra-hub` (new) | The daemon, nodes, leases with fencing tokens, the pool, the priority order, the shared proof store | `hub.rs`: lock files become leases. |
| The bench | `ra-bench` (new) | Cases, the replay of each decider, scores, the comparison of two builds, the holdout | `rc map --eval` and `rc review replay` of the roadmap are its first two replays. |
| The learner | `ra-learn` (new) | Experiments with holdouts, the fitting of the policy tables, the dataset and the local adapters | `lookback.rs`, `retro.rs`, `plan/evidence.rs`. |
| The channel | `ra-console/src/channel.rs`, later `ra-self` | Builds kept by version, candidate and stable, canary, guards, rollback, the charter | `install.rs`, `hub::moved_to`. |
| The providers | `ra-providers` | `ModelProvider` and its adapters; typed stops; limits and plan windows | Stays. Gains a local training and serving adapter. |
| The window | `rc`, `rc-gui`, the MCP server, the pair feed | Views of folds: the board, why, the inbox, the morning report | Stays; the roadmap's seventh milestone already makes it a reader. |

##### The data that flows between them

| Record | What it is | Written by | Read by | Keyed by |
|---|---|---|---|---|
| Card | Title, objective, acceptance, files, waits, size | The person, the planner | Everything | Node id |
| Contract | Checks and numbered criteria, each with who proves it | `contract_of` | Worker, prover, reviewer, verdict | Card and its text's hash |
| Prediction | Tokens, turns, pass probability, route, each with the like cases it came from | The policy | Gate, scheduler, `rc why` | Card and policy version |
| Decision | A decider's name, the hash of its inputs, its output | Every decider | Shadow, replay, `rc why` | Event id |
| Lease | A seat, slot or stage held by a node until a time, with a fencing token | The hub | Scheduler, runner | Resource and token |
| Stage | Where a run stands: mapped, spec written, coded, proven, staged, landed | The runner | Recovery, the board | Run id |
| Proof | A check's command, exit, parsed result, the rung it satisfies, the environment stamp | The prover | Verdict, landing, `rc verify`, other nodes | Tree hash, command, stamp |
| Verdict | Each criterion with its proof or a reviewer's sentence and the line it cites | The prover, the reviewer | The person, the lander, the bench | Card and tree |
| Case | Base, card as sent, landing, proofs, calls, outcome | A fold of the record | The bench, the estimator, training | Project and node |
| Escape | A later fix tied to the landing that let the bug through | Blame on a fix's lines, bisect of a red main | The proof measures, the bench | Both cards |
| Question | Text, options, default, deadline, what waits on it | Planner, runner, channel | The inbox | Id |
| Policy table | A fitted table with its counts and the cases it was fitted on | The learner | The policy, `rc learn --explain` | Name and version |
| Build | A version of the agent with its bench score, canary state and guards | The channel | The hub, every walk | Version and commit |

##### One card's path through the endgame

1. The person approves a plan. Its cards enter the roadmap through the gate; each already has a contract and a check seen red on the base.
2. The policy writes a prediction for each ready card from its like cases. A card under the bar goes back to the planner to be split, scouted or asked about.
3. The scheduler places the card: a node, a seat, a route, and a reason for anything it waits on. Each placement is a decision in the record.
4. The runner leases a worktree with a warm target and writes the first stage. The atlas gives the map; the contract and the map are the prompt, and almost nothing else is.
5. Where the check names a test that does not exist, a spec pass on a strong model writes the tests and nothing else; the prover sees them compile and fail; a reviewer rules on whether each criterion has an assertion that would fail without it.
6. The code pass runs on the route the table chose, often a local model, in a loop whose context the orchestrator owns: read ranges leave the context once they are edited or stale, and a question of the form "where is X" is answered by the atlas, not by a search.
7. The worker calls `rc prove`. The prover runs the selected checks once, by the tree; climbs the ladder; and writes the proof.
8. Criteria the ladder proved are ruled by the machine. The rest go to the reviewer with the facts. The verdict is written.
9. The lander stages the tree in the landing worktree, the batch is proven, main moves by fast-forward. The proof goes to the shared store.
10. The record folds the run into a case. The estimator's error, the route's outcome, the map's hits and each note's effect are each one more row in a table.
11. If the card was the agent's own, the channel builds a candidate and the loop of capability 8 begins.
12. If a later fix changes these lines, the escape is traced here and the rungs that let it pass are scored down.

##### What stays and what is replaced

| Today (at 787be7f) | In the endgame | Why |
|---|---|---|
| `rhipper.md`, cards, sections, milestones, gambits | Stays | The roadmap is the one thing the person writes; it must stay a file they can read and edit. |
| A git worktree for each card, squash landing, never a forced push | Stays, behind the landing worktree of the roadmap | Git is the transport between machines too; fast-forward is the compare-and-swap. |
| `ModelProvider` (`ra-providers/src/lib.rs`) and the Claude Code seat | Stays | The best remote model remains a seat type; the trait is where local and trained models plug in. |
| `rc hook pre` and `rc hook post`, the gotchas catalogue | Stays; rules gain measured precision, cards gain capabilities | The hook is the only place every worker of every kind passes through. |
| Friction routed to the agent's own roadmap (`plan/home.rs`, `Owner::Agent`) | Stays; gains evidence, a predicted saving and the channel | It is the seed of the whole self-improvement loop. |
| The usage ledger `var/usage.db` | Becomes a fold of the record | Two stores of what happened drift apart; the reviews found the window disagreeing with the logs. |
| `agent.rs`, 8,600 lines in which `run_task` decides and acts in one function | Staged runner and pure deciders in `ra-run`, `ra-plan`, `ra-policy` | A function that both decides and acts cannot be replayed, shadowed or resumed. |
| Lock files and digests in the hub directory (`hub.rs`) | Leases from a daemon, with fencing tokens | A file lock cannot expire on a machine that is gone. |
| `[[route]]` rules by complexity, kind, files, words (`router.rs`) | A fitted table; the rules remain as the person's overrides and as the fallback under ten cases | A rule is a guess that never learns it was wrong. |
| The lexical map (`map_of` of the roadmap) | `where` on the atlas | Words find where a thing is named; a graph finds what it touches. |
| The reviewer reading the whole diff (`judge`, `review_brief`) | The last rung of the ladder, on unproven criteria only | An opinion is the most expensive and least repeatable proof. |
| The look-back every ten landings (`lookback.rs`, `EVERY = 10`) and `notes.log` | The learner: every run updates the tables; notes are experiments | Ten landings is an arbitrary clock, and a note's effect was never measured. |
| The investigator (`investigate`, agent.rs:4928) | Scouts: short read-only runs that return pointers into a throwaway context | Reading in is the largest avoidable cost of a turn. |
| `rc install; rc restart` by hand (`install.rs`) | The channel | The step that carries the most risk has the least proof. |
| `rhipper.context.md` written by hand | Rendered from the atlas | A hand-kept map is stale the day after it is written. |

##### The hard problems, and how each is solved

**1. Tests that pass and prove nothing.** A model asked to make a check green will write a check that is green. The answer is the ladder's middle rungs, each mechanical: the check is run on the base and must fail there; the tests are written in a separate pass, before the code, by a model that has not seen the code, and judged against the criteria alone; the code pass may add to them but a removed assertion is a fact the reviewer sees; the new tests must execute the changed lines; a handful of mutants in those lines must die. None of these needs a model's opinion. What shows it worked is the escape rate, by rung: if cards that passed with dead mutants escape as often as cards without, the rung is theatre and is removed.

**2. A system that tunes its own measures will cheat them.** Every measure the agent can move is paired with one it cannot move the same way: first-pass rate with escape rate, machine verdicts with the disagreement of a sampled blind review, tokens with landings, questions asked with defaults the person overrode. A guard fails when a measure improves while its pair worsens. One case in five is a holdout that no fitting and no prompt ever reads, and the holdout rule is in the charter. And one landing in fifty is put in front of the person as an audit, picked at random, so the top of the chain is a human sample, not a model.

**3. Changing the agent with the agent.** A bad build can break the thing that would have caught it. So the channel is small, changes rarely, and is on the charter's protected list; the stable build is always kept and a walk that dies twice on a candidate moves back without consulting the candidate; the canary is the agent's own project, where a failure costs the agent and not runelite; and a charter test that a candidate deletes or renames refuses the build before the bench runs.

**4. Many machines without a cloud.** Three things are shared and each has one simple rule. Code moves by git, and main moves only by fast-forward, which is a compare-and-swap every git server already implements. The record is one append-only log for each project and machine, so there is one writer a file and merging is concatenation ordered by a hybrid clock. Resources are leased by the hub daemon for a time, with a fencing token that the runner must present to stage or land, so a node that comes back from the dead cannot land stale work. If the hub's machine is gone the nodes finish their stages and stop; another node can be made the hub from the record. Nothing here needs consensus, because there is one person and one hub at a time.

**5. Local models that are not good enough.** They do not have to be good; they have to be measured. A role moves to a local model when that model, on holdout cases, holds the incumbent's bar, and moves back when its live rate falls under it. Two things raise what a small model can do. The spec pass turns "implement this card" into "make this red test green in these ranges", which is the easiest form a coding task has. And training on the person's own landed work, on their own GPU, teaches the idiom of their codebases, which no general model was trained on. Where neither is enough the table simply keeps the role remote; local-first is a price, not a rule.

**6. The context of a turn is the cost of a run.** The reviews' model stands: input is about turns times floor plus growth times turns squared over two. The roadmap cuts the floor and the turns. The endgame cuts the growth, which needs the loop to be the orchestrator's own: a range read is dropped from the context when it is edited or superseded, a build's output is kept as its parsed failures, a search is a scout's job in a context that is thrown away, and cache breakpoints are placed where the prefix is known stable. The Claude Code seat stays for the work where its harness earns its floor; the native loop takes every role where the table says it is cheaper.

**7. Many languages, one atlas.** The breadth layer is tree-sitter: items and name references for any language with a grammar, in milliseconds, keyed by blob. The depth layer is the language's own server where one exists (rust-analyzer, a Java or Kotlin server for runelite, SCIP indexes where a project can produce them), asked lazily and cached by the same key. Every query degrades to the breadth layer and says so; `where` on an assembler source with no grammar falls back to the lexical map.

**8. A proof that does not hold on another day or another machine.** A proof's key is the tree, the command and a stamp of the toolchain, the lockfile and the platform. A proof is reused only under the same stamp. A test seen both green and red on one tree is flaky by definition and is quarantined with a card to fix it, not retried into a pass. Checks run with the network denied unless the card's capabilities say otherwise.

**9. Conflicts at width.** Twelve writers on one repository collide on files; the roadmap handles that by holding back on overlap. The atlas makes the claim an item, not a file, so two cards in one large file run together when they touch different functions. Where they do touch, the planner orders them and the later one starts stacked on the earlier one's branch. A merge that still conflicts is retried structurally (by item) before a model is asked. And the landing batch bisects itself, so one bad card costs one extra proof, not the batch.

**10. A bench that lies.** A scripted replay shows what a decider would have decided, not what a model would then have done. So each tier claims only what it can show: the scripted tier scores decisions against outcomes already known (did the map name the ranges the landing changed; did the selection include the test that failed); the live tier answers the counterfactual on a sample; and after every walk the bench's predicted direction is set beside the live measure. A decider whose bench score and live measure disagree twice loses the right to be promoted on the bench alone.

**11. Small numbers.** One person's three projects make hundreds of cases, not millions. So the tables are simple: medians, rates and counts, pooled across projects and shrunk toward the pool where a project has few cases; a cell under ten cases falls back to the person's rule; and every prediction is shown with its count, so "0.91 from 34 like cards" and "0.9 from 3" are never mistaken for each other. Nothing here is a neural network until phase 18, and there the model learns to write code, not to make decisions.

**12. The model changes underneath.** A provider updates a model and every fitted table is quietly wrong. Tables are keyed by model id; a control chart on first-pass rate by route flags a shift; a flagged route gets a live replay of a sample that night, and its table is refitted from cases after the change.

**13. A worker with a shell.** More autonomy on more machines needs less trust in any one run. A card carries capabilities: the paths it may write (its `files:` and their tests), whether its checks may reach the network, whether it may add a dependency. The hook enforces them; a run that needs more asks, and the ask is a question with a default of no. A new dependency is always a criterion owned by the person. On nodes other than the person's desk, a run is in a sandbox with a copy of nothing but the worktree and the toolchain.

**14. The person as the bottleneck.** If the agent is good, the queue of questions is the only queue. So a question is designed like a card: the fewest words, the options, the evidence, the default, the deadline, and what waits on it; work that does not depend on the answer goes on; and the ledger of the person's minutes is a guard measure that a build of the agent can fail.

#### The road

The roadmap's nine milestones are phases 1 to 9. Each phase below is one milestone of the outline.

| Phase | Milestone | The capability it delivers | The measure that proves it | Depends on |
|---|---|---|---|---|
| 1 to 9 | `docs/fable/agent-roadmap.md` | A cheap, honest, durable walk: admitted cards, warm builds, a map, one contract, one proof for each tree, a walk that resumes, a typed record, a hermetic suite, one planner. | Its own targets: reruns 50/60/50%, first reviews 85/50/50%, a 35k median turn, 60 s to landed. | The agent at 787be7f. |
| 10 | Every landed card is a case the agent can replay | The agent measures a change to itself on the person's past work, in minutes, with no model call. | `rc bench` replays 100 or more cases from three projects in under 300 s; its direction agrees with the next two walks on three deciders. | 2 (check ledger), 3 (`rc map --eval`), 4 (`rc review replay`), 6 (run ledger), 7 (events). |
| 11 | A new build of the agent earns its install | The fork changes itself without the person installing anything, and a bad build undoes itself. | Over ten of the agent's own landings no other project runs an unproven build; a seeded bad build is rolled back with no one asked. | 10; 6 (stages on disk); 9 (the move to a newer install). |
| 12 | The contract is red before the run and green after it | A landing carries proof that its tests test the change; proven criteria need no reviewer. | First verdicts 90/75/75%; 40% of landings whose diff no reviewer read; reruns 30% or less; escapes measured, 2 or fewer for each 100 landings. | 4 (contract, proof lines); 2 and 5 (ledger, one proof set); 10 (to score the rungs). |
| 13 | A card's cost and its chance are known before it runs | Predictions with their error; a card under the bar is split or scouted, not run; routes fitted, local-first by price. | Median error of predicted turns 30% or less; cards predicted over 0.8 pass first 80% of the time or more; local share of tokens 25%. | 10 (cases), 12 (specs make cheap routes viable), live cards #196 to #200. |
| 14 | The agent knows the code as a graph | The atlas, its four queries, a generated context file. | Map hit rate 90% on the bench; select misses under 1%; two cards in one file run together; read-in is two calls. | 3 (the map it replaces), 5 (`select`), 10. |
| 15 | The run's context is the orchestrator's | A native loop for every route, with eviction, scouts and the atlas as a tool. | Median turn 15k tokens; a c3 run 12 turns or fewer; context growth a turn halved against the Claude Code seat on live replays. | 13 (routes to compare), 14 (atlas), 10 (live tier). |
| 16 | A sentence in, a proven plan out | `rc intend`: intent to a checked card graph with predictions and questions asked first. | 80% of a plan's cards land as written; the person edits fewer than one card in ten; no card is written by hand for a month. | 12 (red checks at plan time), 13, 14. |
| 17 | One hub, every machine the person owns | The hub daemon, nodes, leases, a shared proof store; a walk outlives any one machine. | A walk on three machines; a node killed mid-run costs no landed work and resumes in 5 min; the laptop's lid closes on nothing. | 6, 9, 11 (builds move by the channel on every node). |
| 18 | Local models earn their roles | A dataset of the person's own work, adapters trained at home, promotion by holdout. | Half of all tokens local; every cheap role local; no measured fall in first-pass rate or rise in escapes. | 10 (holdout), 12 (the code-on-spec role), 13, 15, 17 (the GPU as a node). |
| 19 | The person answers questions and reads verdicts, and nothing else | The inbox, the morning report, `rc ask`, the attention ledger. | 30 minutes or fewer of the person for each 100 landed cards; no walk stopped on a person for longer than a default's deadline. | 7 (why), 12 (verdicts), 16 (questions at plan time). |
| 20 | Nothing is lost and anything is undone | `rc undo`, bisect and a proven revert of a red main, walk replay, drills. | Every drill in the suite recovers inside its bound, weekly, with no one asked. | 6, 11, 17. |
| 21 | Forks trade proven changes | A change to the agent packaged with its evidence; imported through bench and canary. | A change proven on one fork is installed on a second by its own channel with no hand edit, and carries none of the first fork's project code. | 10, 11. |
| 22 | A month of walks nobody watched | The endgame's own audit. | Every row of the endgame column of the measures table, held for thirty days. | All. |

##### Why this order

The bench comes first because every phase after it is a change to a decider, and without the bench each would be judged the way the five reviews judged today's agent: by reading logs once. The channel comes second because every phase after it changes the build that runelite and asm are walked with, on a fork the person works from. The proof ladder comes third because it is the largest single move toward both of the person's hardest values at once: machine verdicts are what end the reading of diffs, and a red test is what lets a cheap or local model do the writing. After those three the order is by dependency, and phases 13 to 16 could each be reordered against a neighbour if the record says a different cost dominates by then; the bench exists to say so.

##### What the person stops doing, phase by phase

A capability is easiest to check by the chore it ends.

| After phase | The person no longer |
|---|---|
| 9 (the roadmap) | watches a walk burn runs on a card that cannot pass, or restarts one that died |
| 10 | reads logs, or commissions a review, to learn whether a change to the agent helped |
| 11 | runs `rc install; rc restart`, or finds out from runelite that the agent's last change was a bad one |
| 12 | reads the diff of a card whose criteria were proven, or wonders whether a green check tested anything |
| 13 | sizes a card, picks its model, or learns from a third rerun that it was too big |
| 14 | keeps `rhipper.context.md`, or holds a card back because another names the same file |
| 15 | pays for a context that only grows |
| 16 | writes a card, an acceptance or a wait |
| 17 | keeps one machine on and sits at it |
| 18 | sends routine writing off their machines |
| 19 | watches the window |
| 20 | repairs anything by hand: a red main, a bad landing, a lost machine |
| 21 | carries an improvement from one fork to another by hand |
| 22 | takes any of this on trust |

##### What I am least sure of, and what would change the plan

An endgame written with no doubt in it would be the first thing here that was not measured. These are the bets, in the order I would watch them.

| The bet | What would show it lost | What changes then |
|---|---|---|
| A scripted replay predicts a live walk. | Most replays marked `unproven` by the calibration of phase 10. | The live tier's share of the week's room grows, the channel leans on the canary and the shadow, and promotion on the bench alone ends. The channel still works; it is slower. |
| The spec pass pays for its second run. | After 50 landings, cards with a spec cost more tokens for each landed card than like cards without. | `wants_spec` narrows to c4 and above, and the red-base rule and the changed-line rule carry the ladder alone. |
| Mutants from a text table say something. | The escape rate of cards that killed their mutants equals that of cards that had none. | The rung is removed. A rung that does not move escapes is cost. |
| The person's hardware can hold the bar for the code pass. | No local route passes the exam of phase 18 for any kind of card above c2. | Half of all tokens local is not reached. Nothing else in the design depends on it: the table keeps those roles remote, and the cheap roles and scouts stay local. |
| Three projects are enough data. | Cells of the route table stay under ten cases for most kinds of card. | The tables stay coarse (by size alone), and the exchange of phase 21 moves earlier, since pooled counts from other forks are the only other source. |
| The Claude Code seat stays what it is. | Its floor, its tools or its plan limits change under the agent. | That is the reason for phase 15: the native loop is the seat the agent owns. If the seat worsens, phase 15 moves ahead of 14. |
| A canary on the agent's own project is fast enough. | Eight landings take longer than the agent's cards arrive, and candidates queue. | The canary's minimum falls for changes the shadow shows to be small, and the live bench takes more of the proof. |
| Proofs travel between machines. | The desk is Windows and another node is not, so few stamps match. | Each platform proves for itself and the store is shared only within a platform. Phase 17 loses a saving and keeps its point. |

Each of phases 13 to 22 is written below as a milestone and its sections, with no cards. That is on purpose and it is the first use of the endgame: live gambit g3 carries `plan-next: true`, so when a walk has nothing ready it asks the architect model to plan the first milestone that has no cards (`next_unplanned`, `crates/ra-console/src/commands/run.rs`). It takes them in the order of the file, which is the order of the table above. Today it would also take one whose dependencies had not landed; the first section of phase 10 closes that, and until it lands the last section of this file says what to set by hand.

#### The outline

Phases 10, 11 and 12 follow as cards: 65 of them, 32 at c2, 28 at c3, 5 at c4. Paths under `walk/`, `agent/`, `events`, `ra-check`, `contract.rs`, `map.rs` and the split test files are the ones the roadmap's own cards create; a card here that names one waits on the roadmap card that makes it, as the last section says.

## Every landed card is a case the agent can replay
objective: The five reviews measured the agent once, by reading its logs: 68 of 151 calls before the first edit, a suite run 3 to 5 times for one clean pass, 14% of runelite's first reviews passed. Each number took a session to find, and none can be found again after the change it argued for. After this milestone the measuring is a command. Every landed card is kept as a case (its base, the card as sent, its landing, its proofs, the calls its worker made), and each decider the roadmap made pure (the gate, the map, the selection of checks, the blame, the route, the prompt's assembly) is replayed over the cases with no model call and scored against what really happened. Every later phase changes a decider and shows it here first.
acceptance: Every child accepted, and `rc bench` replays at least 100 cases from the three projects in under 300 s with no model call; over the two walks after it lands, the direction it gave for the map's hit rate, the checks selected and the prompt's weight agrees with the walk's own measures.

### The roadmap carries its own order
- The outline reads a `waits:` line, and an import sets the waits {c2}
  objective: The outline format carries no waits, so `agent-roadmap.md` ends in a table of 30 rows to be typed into `rc depend`, and this file ends in another. `Item` (outline.rs) gains `waits`, where `above` names the item before it in its section, `#N` a node already in the roadmap, and anything else the start of a title in the same outline. `Store::import` (store.rs:265) resolves them once the nodes have ids. A name that matches nothing, or two items, is a fault the gate prints, and the card is written without that wait.
  acceptance: `cargo test -p ra-roadmap -q outline_waits` passes, importing three cards where the third waits on the first by its title and the second on `above`.
  files: crates/ra-roadmap/src/outline.rs crates/ra-roadmap/src/outline/tests.rs crates/ra-roadmap/src/store.rs crates/ra-roadmap/src/store/tests.rs
- A wait on a node above a card holds the card {c2}
  objective: `effective_state` (tree.rs:86) reads only a card's own waits; its comment says any node above a task keeps its stored state. So a milestone that waits on another milestone holds nothing, and phases cannot be put in order without a wait on every card. A card is ready when its own waits and those of every node above it are accepted, and a new `holder_of` gives the node that holds a card, its own wait or one above, for `rc next --why` today and `rc why` once it exists.
  acceptance: `cargo test -p ra-roadmap -q waits_above_hold` passes, with a card under a section whose milestone waits on an open node staying defined, and ready once that node is accepted.
  files: crates/ra-roadmap/src/tree.rs crates/ra-roadmap/src/tree/tests.rs
- Plan-next passes over a milestone whose waits are open {c2}
  objective: `next_unplanned` (commands/run.rs) takes the first milestone with no card under it, in the order of the file, whatever it waits on. Phases 13 to 22 of the endgame are imported as milestones without cards, each resting on earlier ones; planned early, a milestone's cards cite code that does not exist yet and the gate faults them. A milestone with an open wait is passed over, and the feed says once which milestone waits on which.
  acceptance: `cargo test -p ra-console -q plan_next_passes_over` passes, with two unplanned milestones of which the first waits on an open node and the second is the one planned.
  files: crates/ra-console/src/commands/run.rs crates/ra-console/src/commands/tests.rs
- `rc import --chain` makes a card wait on the card above it where their files meet {c2}
  objective: "Inside a section each card builds on the ones above it" is the convention of every outline written so far, and the import ignores it: a walk with free seats starts a section's cards together and they collide. With `--chain`, a card with no `waits:` line waits on the nearest card above it in its section that names one of its files. The dry run prints the waits it would set.
  acceptance: `cargo test -p rc -q import_chains` passes, printing `#3 waits on #1 (src/a.rs)` for an outline of three cards where the second names other files.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-roadmap/src/store.rs
- The planner's cards arrive with their waits {c2}
  objective: `plan` writes cards as defined and unordered, so a milestone the walk planned has the same hole as an imported one. The planning brief asks for a `waits:` line in the outline form the gate admits, the planner's outline goes through the same resolver as an import, and the cards it leaves unordered are chained by their files.
  acceptance: `cargo test -p ra-console -q planned_cards_wait` passes, with a scripted planner whose third card names the first by its title.
  files: crates/ra-console/src/plan/propose.rs crates/ra-console/src/plan/prompt.rs crates/ra-console/src/plan/tests.rs

### A landed card leaves a case
- The record is its own crate, `ra-record` {c3}
  objective: The event types, the log and the `Board` live in `ra-console` (events.rs, events/board.rs), so anything that reads the record links the whole console. The bench, the window and later the hub read the record and need nothing else of the console. The files move to `crates/ra-record` with their tests, and `ra-console` re-exports the names so no other card's citations break.
  acceptance: `cargo test -p ra-record -q board_` passes with the tests that moved.
  files: crates/ra-record/Cargo.toml crates/ra-record/src/lib.rs crates/ra-record/src/board.rs crates/ra-record/src/tests.rs crates/ra-console/src/events.rs crates/ra-console/src/lib.rs crates/ra-console/Cargo.toml
- A dispatch keeps the card as it was sent and the prompt by its pieces {c2}
  objective: A card is edited after it runs (by a reassessment, by the person, by a split), so the roadmap cannot say what a past run was asked. The run ledger (walk/ledger.rs) keeps `card.md`, the card's text at dispatch, and `prompt.tsv`, one row for each piece of the prompt with its name, its bytes and its hash. A case can be replayed only from these.
  acceptance: `cargo test -p ra-console -q dispatch_keeps_the_card` passes, with the card edited after dispatch and the ledger still holding the first text.
  files: crates/ra-console/src/walk/ledger.rs crates/ra-console/src/agent/prompt.rs crates/ra-console/src/commands/tests/run.rs
- A run's calls are kept as rows {c3}
  objective: The worker's stream is parsed for counts (`reads`, `read_in`, `whole_reads` on `Completion`) and then dropped. What the worker read, ran and edited, in order, is the evidence every review had to dig out of logs. Each tool call becomes a row of `calls.tsv` in the run's ledger: the turn, the tool, the target (a path and range, or a command), bytes in and out, milliseconds, and the hook's decision. A run is a few kilobytes. The whole stream is kept only for the newest runs, `[bench] streams = 200`.
  acceptance: `cargo test -p ra-providers -q calls_are_rows` passes, with a recorded stream of a read, an edit and a denied command giving three rows.
  files: crates/ra-providers/src/claude_code.rs crates/ra-providers/src/claude_code/tests.rs crates/ra-providers/src/lib.rs crates/ra-console/src/walk/ledger.rs
- `ra-bench` holds a case and its file form {c3}
  objective: A case is the unit every later measure is taken over. `Case` (case.rs, in a new crate) holds the project, the node, the base and landing commits, the card as sent, the contract, each run with its route, turns, tokens and outcome, the checks with their blame, and each verdict. It holds commits, not code: the repository is the store, so a case is about 2 kB of TOML at `var/bench/<node>.toml`. `Case::load` refuses a case whose commits are not in the repository. `ra-bench` depends on `ra-roadmap` and on no other crate of the agent; a crate that owns a decider implements the replay beside it.
  acceptance: `cargo test -p ra-bench -q case::` passes, with a case written, read back equal, and refused once its base commit cannot be reached.
  files: crates/ra-bench/Cargo.toml crates/ra-bench/src/lib.rs crates/ra-bench/src/case.rs crates/ra-bench/src/case/tests.rs
- A landing writes its case, and `rc bench build` writes the ones before it {c3}
  objective: Cases have to exist for the hundreds of cards already landed, not only for those to come. A landing folds its run ledger, its events and its check rows into a case. `rc bench build` does the same for every accepted card whose usage rows carry a base and a landing commit (the roadmap's `rc map --eval` put them there) and prints `<n> cases, <m> skipped`, with the skips counted by reason.
  acceptance: `cargo test -p rc -q bench_build` passes, printing `2 cases, 1 skipped (no base commit)` for a scripted record of three landed cards.
  files: apps/rc/src/bench.rs apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-bench/src/build.rs crates/ra-bench/src/build/tests.rs crates/ra-console/src/walk/landing.rs
- A case knows whether its check was red on its base {c3}
  objective: Nobody knows how many of the checks that passed would have passed before the card ran, and phase 12 is built on the answer. `rc bench verify` runs each case's checks at its base in a leased worktree, through the check ledger so a tree is run once, and marks the case `red`, `green` or `no test`. It runs at the lowest slot rank and stops when a walk wants the slots.
  acceptance: `cargo test -p ra-bench -q red_on_base` passes, with three scripted cases marked `red`, `green` and `no test`.
  files: crates/ra-bench/src/verify.rs crates/ra-bench/src/verify/tests.rs apps/rc/src/bench.rs

### The deciders replay with no model call
- A replay is a decider, a case and a score {c2}
  objective: Each decider is scored its own way and the bench should not know how. `replay.rs` holds a trait `Replay` with a name and `score(&Case, repo) -> Score`, where `Score { hit, of, note }` is a count out of a count with the case's own line of explanation. `run` maps the replays over the cases on as many threads as there are build slots and gives one row for each replay.
  acceptance: `cargo test -p ra-bench -q replay::` passes, with a scripted replay over four cases summing to `3 of 4`.
  files: crates/ra-bench/src/replay.rs crates/ra-bench/src/replay/tests.rs crates/ra-bench/src/lib.rs
- The map is replayed: of the hunks a landing changed, how many the map named {c2}
  objective: `rc map --eval` (roadmap, milestone 3, test `map_eval`) already replays landed cards for the map's hit rate. It is the first replay and moves under the bench with its measure unchanged: for a case, the changed hunks its map's pointers cover out of all changed hunks, with the map built from the card as sent and the files at the case's base commit. `rc map --eval` stays as a name for `rc bench --only map`.
  acceptance: `cargo test -p ra-console -q map_replay` passes, with a case whose landing changed two hunks and whose map named one scoring `1 of 2`.
  files: crates/ra-console/src/bench.rs crates/ra-console/src/bench/tests.rs crates/ra-console/src/lib.rs apps/rc/src/bench.rs
- The selection is replayed: whether the checks chosen held every test that failed {c3}
  objective: `ra_check::select` (roadmap, milestone 5) picks the crates to test from the crate graph. A selection that leaves out a test that would have failed lands a broken tree; one that is too wide spends build slots. For each case the replay asks `select` for the landing's diff and scores the failed checks of the case's runs that fall inside the selection, and beside it the crates selected out of the workspace's.
  acceptance: `cargo test -p ra-check -q select_replay` passes, with a case whose run failed a test in a crate the selection left out scoring `0 of 1`.
  files: crates/ra-check/src/select.rs crates/ra-check/src/select/tests.rs crates/ra-check/Cargo.toml
- The gate is replayed against what the runs then cost {c2}
  objective: The gate is right when the cards it faults are the ones that would have burned runs. For each case the replay runs `admit::faults` on the card as sent and scores it against the record: a card that took four runs or more and was not faulted is a miss, and a faulted card that landed in one run is a false alarm, counted beside it.
  acceptance: `cargo test -p ra-bench -q gate_replay` passes, with a scripted case of a card that ran five times on a command the tool rejected scoring a miss for a gate that admits it.
  files: crates/ra-bench/src/gate.rs crates/ra-bench/src/gate/tests.rs crates/ra-bench/src/lib.rs
- The blame is replayed against what then fixed the check {c2}
  objective: `classify` (roadmap, milestone 1) is right when the one it blames is the one whose change made the check pass. For each failed check of a case the replay classifies the kept evidence and scores it against the record: a failure blamed on the card that went away with no change to the card's files is a miss, and so is one blamed on main or the machine that only the card's next run fixed.
  acceptance: `cargo test -p ra-check -q cause_replay` passes, with a scripted failure blamed on the card that passed on a retry with no edit scoring a miss.
  files: crates/ra-check/src/cause.rs crates/ra-check/src/cause/tests.rs
- The prompt is replayed: its weight by piece, for every case {c2}
  objective: The floor under every turn was found by hand (17 to 21k tokens, efficiency review). The replay assembles each case's first prompt with the build under test and gives its bytes by piece beside the case's `prompt.tsv`. A change to the prompt shows its weight on every project's cards before a worker reads it.
  acceptance: `cargo test -p ra-console -q prompt_replay` passes, printing each piece's bytes for a scripted case and the change from the recorded prompt.
  files: crates/ra-console/src/bench.rs crates/ra-console/src/bench/tests.rs crates/ra-console/src/agent/prompt.rs
- The route is replayed: the rung chosen beside the lowest rung that passed {c2}
  objective: The router picks a rung by rule (router.rs, live cards #196 and #197) and nothing says how often a lower rung would have done. For each case the replay asks the router for the card as sent and scores it against the record: the lowest rung on which a card of the same kind and size passed first time in that project. It is a coarse score, and it is the baseline the fitted table of phase 13 has to beat.
  acceptance: `cargo test -p ra-console -q route_replay` passes, with a scripted record in which the rule picks the top rung for a kind that passed on the second.
  files: crates/ra-console/src/bench.rs crates/ra-console/src/bench/tests.rs crates/ra-console/src/router.rs
- The hooks are replayed over the kept calls {c3}
  objective: A gotcha rule is added when something goes wrong once and is never checked again, and a rule that fires on passing runs costs every run a turn. The replay runs the catalogue over each case's `calls.tsv` and gives, for each rule, the runs it would fire in that passed first time and that did not. A rule that fires more on passing runs than on failing ones is printed as a candidate to retire.
  acceptance: `cargo test -p ra-gotchas -q rules_replay` passes, with a scripted rule firing in two passing runs and no failing one named as a candidate.
  files: crates/ra-gotchas/src/replay.rs crates/ra-gotchas/src/replay/tests.rs crates/ra-gotchas/src/lib.rs crates/ra-gotchas/Cargo.toml
- `rc bench` prints one table and keeps it {c2}
  objective: One command, one table: a row for each replay with its cases, its score and the change from the baseline; `--json` for the window; `--project` to narrow. The run is timed and the time printed, because a bench that takes an hour is not run. The table is saved under the hub as `bench/<version>.tsv`, the baseline the next build is compared with.
  acceptance: `cargo test -p rc -q bench_prints` passes, printing a row `map  2 cases  3 of 4  +1` against a saved baseline of `2 of 4`.
  files: apps/rc/src/bench.rs apps/rc/src/tests.rs crates/ra-bench/src/report.rs crates/ra-bench/src/report/tests.rs

### Two builds are compared before one is trusted
- The bench reads every project's cases from the hub {c2}
  objective: A change to the map that helps the agent's Rust and hurts runelite's Java is not an improvement, and a walk of one project cannot see it. The hub already knows the projects (`hub::recent` and the priority file, hub.rs:833). `rc bench` with no `--project` replays all of them and gives each replay a row for each project and one for the pool.
  acceptance: `cargo test -p rc -q bench_reads_the_hub` passes, with two scripted projects giving two rows and a pooled one.
  files: apps/rc/src/bench.rs apps/rc/src/tests.rs crates/ra-console/src/hub.rs
- One case in five is a holdout that nothing tunes on {c2}
  objective: Anything fitted or tuned on the cases will score well on them. `Case::holdout` is true for one case in five, by a hash of its project and node, so it never moves. `rc bench` scores the holdout in its own column, and `ra_bench::cases(Split::Fit)` is the only door a fitter or a planner's evidence may use. A test fails the build when `Split::All` is named outside the bench's own report.
  acceptance: `cargo test -p ra-bench -q holdout` passes, with 100 scripted cases splitting 80 to 20 the same way on two runs.
  files: crates/ra-bench/src/case.rs crates/ra-bench/src/case/tests.rs crates/ra-bench/src/lib.rs
- Two tables are compared case by case, with a verdict {c3}
  objective: "3 of 4 against 2 of 4" does not say whether a change is better or lucky. `compare.rs` pairs two builds' scores by case and gives each replay a verdict, `better`, `worse` or `same`, by a sign test over the cases that differ, with the cases that got worse named. `rc bench --against <version>` prints it and exits 1 on any `worse`. The person reads the verdicts and, under a `worse`, the three worst cases.
  acceptance: `cargo test -p ra-bench -q compare::` passes, with two scripted tables that differ on nine cases, eight of them better, giving `better (8 of 9, p 0.02)`, and two that differ on two cases giving `same`.
  files: crates/ra-bench/src/compare.rs crates/ra-bench/src/compare/tests.rs apps/rc/src/bench.rs

### The bench is checked against the walks
- After a walk, the bench's direction is set beside the walk's own measure {c3}
  objective: A scripted replay says what a decider would have decided, not what a worker then did. For each build the record keeps what the bench said of each replay and, once the build has landed twenty cards, the live measure that replay stands for: the map's hit rate from the calls before the first edit, the selection's misses from red landings, the prompt's weight from the first turn's tokens. `rc bench --calibrate` prints agree or disagree for each, and a replay that disagreed on its last two builds is marked `unproven` in every table after.
  acceptance: `cargo test -p ra-bench -q calibrate` passes, with a scripted record where the map's replay said better twice and the live hit rate fell twice, marking it `unproven`.
  files: crates/ra-bench/src/calibrate.rs crates/ra-bench/src/calibrate/tests.rs apps/rc/src/bench.rs
- A case can be run again for real, and never lands {c4}
  objective: Some questions only a model can answer: would this card have passed first time under the new prompt. `rc bench --live <n>` takes n cases by a seeded sample, makes a worktree at each case's base on a branch `bench/<node>` that no walk lands, runs the card as sent through the stages of a run up to the verdict, and writes the outcome (passed first time, turns, tokens) as a live row of the case. It takes seats only when the walk leaves them idle and stops at `[bench] live`, a share of the week's room read from the same marks as live card #199.
  acceptance: `cargo test -p ra-console -q live_case` passes, with a scripted provider running one case to a verdict, a live row written and no commit on main.
  files: crates/ra-console/src/bench/live.rs crates/ra-console/src/bench/tests.rs crates/ra-console/src/walk/pool.rs apps/rc/src/bench.rs
- The kept reviews are the reviewer's live replay, and a row of the bench {c2}
  objective: `rc review replay` (roadmap, milestone 4, test `replay_agreement`) sends kept briefs to the review role again and prints how many verdicts agree. It is a live replay under another name, with its own files and no baseline. `rc bench --live --reviews <n>` runs it under the same budget, joins each kept review to its case, and saves the agreement as a row a build is compared on, split by whether the card later escaped once phase 12 records that.
  acceptance: `cargo test -p ra-console -q review_replay_row` passes, with three kept reviews and a scripted reviewer that changes one giving the row `review  3 cases  2 of 3`.
  files: crates/ra-console/src/bench/live.rs crates/ra-console/src/bench/tests.rs crates/ra-console/src/agent/review.rs apps/rc/src/bench.rs
- The window shows the bench beside the walks {c2}
  objective: A table nobody opens changes nothing. The metrics view gains one panel from `rc bench --json`: each replay's score over the last ten builds, its verdict against the build before, and the calibration mark. The window computes none of it.
  acceptance: `cargo test -p rc-gui -q bench_panel` passes, rendering a saved table of two builds as two points a replay.
  files: apps/rc-gui/src/metrics.rs apps/rc-gui/src/snapshot.rs apps/rc-gui/src/tests.rs

## A new build of the agent earns its install
objective: The self-improvement loop today is a sentence: the person runs their fork, `rc install; rc restart`. `install` (install.rs) sets the old binary aside as `.old` and writes `rc.version`, and a walk moves to a newer install between cards (`hub::moved_to`, hub.rs:810). Nothing between a landing on the agent's roadmap and every project's next run asks whether the build is better, and the only build kept is the last. After this milestone a build is benched before it is installed, walks the agent's own project before any other, is watched by guard measures that put the old build back without anyone asking, and cannot change the short list of things the person reserves.
acceptance: Every child accepted; over the next ten landings on the agent's own roadmap no walk of runelite or asm runs a build that has not passed the bench and its canary; and `rc channel drill` ends with the stable build walking every project and a card on the agent's roadmap, with no person asked.

### A build is kept, named and benched
- An install keeps the last five builds by version {c2}
  objective: `install` keeps one build back, as `.old` beside the binary, and a second install overwrites it. Builds go under the hub as `builds/<version>/` with the commit they were built from; the binaries on the path are copies of the one `rc.version` names; the oldest beyond five is removed. `rc install --back` puts the build before the current one back and says which.
  acceptance: `cargo test -p ra-console -q install_keeps_builds` passes, with three scripted installs leaving three directories and `--back` naming the second.
  files: crates/ra-console/src/install.rs crates/ra-console/src/install/tests.rs apps/rc/src/main.rs
- Every run and every usage row names the build that made it {c2}
  objective: A measure cannot be laid against a build while the record does not say which build ran. The run ledger's first row and each usage row carry the version and the commit of the `rc` that wrote them, and the `Board` gives every measure by build.
  acceptance: `cargo test -p ra-record -q measures_by_build` passes, with a scripted log of two builds giving two rows of the rerun rate.
  files: crates/ra-record/src/board.rs crates/ra-record/src/tests.rs crates/ra-usage/src/lib.rs crates/ra-console/src/walk/ledger.rs
- `rc install` benches the build against the installed one first {c3}
  objective: The cheapest moment to refuse a worse build is before it is on the path. `rc install` builds, runs the new binary's `rc bench --against <installed>`, and refuses on a `worse` with the three worst cases printed. `--anyway` installs and records that it was forced. A replay the calibration marked `unproven` cannot refuse.
  acceptance: `cargo test -p ra-console -q install_asks_the_bench` passes, with a scripted comparison that is worse on the selection refusing the install and naming a case.
  files: crates/ra-console/src/install.rs crates/ra-console/src/install/tests.rs crates/ra-console/src/channel.rs apps/rc/src/main.rs
- A build is installed as the candidate, beside the stable one {c3}
  objective: There is one build on the path, so trying a build means giving it every walk. `channel.rs` (new) holds `Channel { stable, candidate, pins, bad }`, kept in the hub as `channel.toml`. `rc install` makes the new build the candidate, and `hub::moved_to` gives a walk the build its project is pinned to, the stable one unless said otherwise. `rc install --stable` is today's behaviour.
  acceptance: `cargo test -p ra-console -q channel::` passes, with a candidate installed and a walk of an unpinned project staying on the stable build.
  files: crates/ra-console/src/channel.rs crates/ra-console/src/channel/tests.rs crates/ra-console/src/hub.rs crates/ra-console/src/install.rs

### A candidate walks the agent's own project first
- The agent's own project is pinned to the candidate {c2}
  objective: A build that fails should fail where it costs the agent a card, not runelite a day. While a candidate stands, the project `agent_home` names (plan/home.rs) walks with it and every other project with the stable build. The feed says so once a walk.
  acceptance: `cargo test -p ra-console -q canary_is_the_agent` passes, with two scripted projects moving to different builds at the same stage boundary.
  files: crates/ra-console/src/channel.rs crates/ra-console/src/channel/tests.rs crates/ra-console/src/plan/home.rs crates/ra-console/src/walk.rs
- `guard` rules on a candidate from the measures of its runs {c3}
  objective: Someone has to say whether the canary went well, and it should not be a model. `guard.rs` holds a pure `guard(stable, candidate, limits) -> Ruling` over the board's measures by build: the rerun rate, first verdicts passed, turns a run, tokens a turn, seconds from a run's end to landed, holds, and walks that stopped. `Ruling` is `Hold` (too few landings), `Pass`, or `Fail` with the measure and both numbers. The limits are `[guard]` in `rhipper.toml`; by default no measure may be more than 15% worse over at least eight landings.
  acceptance: `cargo test -p ra-console -q guard::` passes with one test for each ruling, one of them a candidate whose turns a run rose 18% failing on that measure.
  files: crates/ra-console/src/channel/guard.rs crates/ra-console/src/channel/guard/tests.rs crates/ra-record/src/board.rs
- A measure may not improve while its pair worsens {c2}
  objective: A build can pass more first verdicts by proving less, and spend fewer tokens by landing fewer cards. `guard` reads pairs: first verdicts with red landings, tokens with landings an hour, holds with reruns. A candidate better on one of a pair and worse on the other by more than half the limit is a `Fail` that names both.
  acceptance: `cargo test -p ra-console -q guard_pairs` passes, with a candidate whose first verdicts rose and whose red landings doubled failing on the pair.
  files: crates/ra-console/src/channel/guard.rs crates/ra-console/src/channel/guard/tests.rs
- A candidate that passes its guards becomes the stable build {c3}
  objective: Promotion is the step the person does by hand today, with less to go on. After each landing on the canary the walk asks `guard`. On `Pass` the channel makes the candidate stable, every walk moves at its next stage boundary (the roadmap's ninth milestone), and the feed and the record carry one line with the bench's verdicts and the canary's measures. `[channel] promote = "ask"` makes it a question for the person instead.
  acceptance: `cargo test -p ra-console -q promotes_on_pass` passes, with a scripted canary of eight landings ending with both projects on the new build.
  files: crates/ra-console/src/channel.rs crates/ra-console/src/channel/tests.rs crates/ra-console/src/walk.rs crates/ra-record/src/lib.rs
- A candidate that fails, or dies twice, is rolled back and becomes a card {c3}
  objective: The build that broke the walk cannot be the one that decides to leave. A `Fail` from `guard`, or a second walk on the candidate ending in a panic or a stop with no reason, marks the candidate's commit bad in `channel.toml`. The ruling is made by the stable build: the wake of the roadmap's sixth milestone starts the stable `rc`, which reads the run ledger and moves the canary back at its stage. A card goes to the agent's roadmap through the gate with the measure, both numbers and the runs, and a build of a commit marked bad is refused.
  acceptance: `cargo test -p ra-console -q rolls_back_on_fail` passes, with a scripted candidate that fails its guard ending with the stable build pinned everywhere and one defined card naming the measure.
  files: crates/ra-console/src/channel.rs crates/ra-console/src/channel/tests.rs crates/ra-console/src/plan/home.rs crates/ra-console/src/walk/wake.rs
- `rc channel` prints where every build stands, and `rc channel drill` proves the rollback {c2}
  objective: The person should see the loop on one screen and be able to test it. `rc channel` prints the stable build, the candidate with its bench verdicts and its landings so far against the guard's minimum, each project's pin, and the last rollback with its reason. `rc channel drill` installs the stable build again as a candidate carrying a drill mark, which the board reads as doubled turns for that build, and exits 0 once the rollback has happened and its card is written; the card is cancelled by the drill.
  acceptance: `cargo test -p rc -q channel_prints` passes, printing `candidate 0.41.3 · bench 2 better 0 worse · canary 3 of 8`.
  files: apps/rc/src/channel.rs apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-console/src/channel.rs

### The agent's own landings become candidates
- A landing on the agent's roadmap builds a candidate when the builders are idle {c3}
  objective: The person runs `rc install` after the agent lands its own cards, when they remember to. After a landing in `agent_home`, when no candidate stands and a build slot is free at the lowest rank, the walk builds main in the landing worktree's warm target and installs it as the candidate through the bench. One candidate stands at a time; landings made meanwhile ride in the next. `[channel] build = "off"` keeps today's behaviour.
  acceptance: `cargo test -p ra-console -q landing_builds_a_candidate` passes, with two scripted landings giving one candidate and the second landing waiting for the next.
  files: crates/ra-console/src/channel.rs crates/ra-console/src/channel/tests.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/install.rs
- A decision is an event that carries its inputs {c3}
  objective: To ask what a new build would have decided, the old build's decisions and what they were made from have to be on record. The schedule's `plan`, the route, the selection and the gate each write `Decided { decider, inputs, output }`, the inputs as a hash with the value kept once under `var/decisions/`. Inputs older than 30 days are pruned.
  acceptance: `cargo test -p ra-record -q decisions_carry_inputs` passes, with two equal inputs stored once and read back from either event.
  files: crates/ra-record/src/lib.rs crates/ra-record/src/decisions.rs crates/ra-record/src/tests.rs crates/ra-console/src/walk/schedule.rs crates/ra-console/src/router.rs
- A candidate shadows the live walk, and `rc shadow` prints where it would have decided otherwise {c4}
  objective: The bench replays the past; the shadow replays the present, on the cards being walked now, before the candidate walks anything. `rc shadow --since <event>`, run with the candidate's binary, reads the stable walk's `Decided` events, calls its own deciders on the same inputs and writes each difference to `var/shadow.tsv`: the decider, the card and both outputs. A candidate that differs on more than `[channel] shadow = "20%"` of the schedule's decisions is held for the person with the ten largest differences, because a change that large was not meant to be quiet.
  acceptance: `cargo test -p ra-console -q shadow::` passes, with a scripted log of three decisions and a decider that differs on one writing one row.
  files: crates/ra-console/src/channel/shadow.rs crates/ra-console/src/channel/shadow/tests.rs apps/rc/src/channel.rs crates/ra-record/src/decisions.rs

### What the agent may not change alone
- The charter names the protected paths and the tests that guard them {c3}
  objective: An agent that can change everything can change what stops it. `charter.toml` at the root of the agent's workspace lists paths (the hooks and their allowlists in `ra-gotchas`, `channel.rs` and `guard.rs`, the holdout rule in `ra-bench`, the push in the landing code, the charter itself) and, for each rule the person holds, the name of a test that proves it. `charter.rs` reads it and gives `protected(path)` and `guards()`. The first rules: main moves only by fast-forward and no command of the agent passes `--force`; nothing lands without a proven tree; a worker's allowlist is never widened by a worker; the holdout is read only by the bench's report.
  acceptance: `cargo test -p ra-console -q charter::` passes, reading a charter of two paths and two tests, with `protected` true for a file under one of the paths.
  files: charter.toml crates/ra-console/src/charter.rs crates/ra-console/src/charter/tests.rs crates/ra-console/src/lib.rs
- A card that touches a protected path lands only when the person approves it {c3}
  objective: The agent should be able to write and prove a change to its own guards; it should not be able to land one. At staging, a diff that names a protected path parks the card in review with the reason `charter: <path>` and writes a question: one sentence from the reviewer on what the change permits that was not permitted before, the bench's verdicts, and the options approve and reject. `rc approve <id>` lands it, and the approval is an event with its time.
  acceptance: `cargo test -p ra-console -q charter_parks` passes, with a scripted card editing a protected file parked with its reason and landed after the approval.
  files: crates/ra-console/src/walk/landing.rs crates/ra-console/src/charter.rs crates/ra-console/src/commands/tests/land.rs apps/rc/src/main.rs
- A build that lost a charter test is refused {c2}
  objective: The quiet way past a rule is to delete its test. Before the bench, `rc install` lists the candidate's tests (`cargo test -- --list`) and refuses a build in which a test the charter names is gone, renamed or ignored, naming it. Changing the name in the charter is itself a protected change.
  acceptance: `cargo test -p ra-console -q charter_test_missing` passes, with a scripted list lacking one named test refusing the install.
  files: crates/ra-console/src/charter.rs crates/ra-console/src/charter/tests.rs crates/ra-console/src/install.rs
- The pre hook denies a worker's edit to a protected path of the agent {c2}
  objective: Parking at the landing is late: the run is already spent. In a worktree of the agent's own project the pre hook answers an edit to a protected path with the charter's line and what to do instead (finish the rest of the card and name the protected change as a question), unless the card already carries the person's approval.
  acceptance: `cargo test -p ra-gotchas -q protected_path_denied` passes, with an edit to `charter.toml` denied and the same edit allowed for an approved card.
  files: crates/ra-gotchas/src/lib.rs crates/ra-gotchas/src/tests.rs crates/ra-console/src/charter.rs
- `rc approvals` lists what waits for the person, each with one sentence and its evidence {c2}
  objective: The person rules on these without reading a diff, or the charter becomes the place where diffs are read again. The list gives each parked card's protected paths, the reviewer's sentence on what it permits, the bench's verdicts and the size of the change. The window shows the count.
  acceptance: `cargo test -p rc -q approvals_list` passes, printing one parked card with its path and its sentence.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs apps/rc-gui/src/snapshot.rs

## The contract is red before the run and green after it
objective: After the roadmap a card has one contract and a reviewer that reads what the check proved, and the targets are 85% of first verdicts passed on the agent and 50% on runelite and asm. What remains is the reason a reviewer is needed at all: a passing check says the worker made it pass, not that it tests the card. Here the check is seen red on the base before any run; where its test does not exist it is written first, alone, and judged against the criteria; after the run the new tests must execute the changed lines and kill a sample of mutants in them. A criterion proven this way is ruled by the machine, the reviewer is asked only about the rest, and every later fix is traced back to the card that let its bug through, so the whole ladder has a measure that proving less cannot improve.
acceptance: Every child accepted, and over the next 100 landings: first verdicts passed 90% on the agent and 75% on runelite and asm, 40% of landings whose diff no reviewer read, reruns 30% or less in each project, and `rc escapes` printing 2 or fewer for each 100 landings.

### A check is seen red on the base
- The gate runs a card's checks on the base before its first run {c3}
  objective: A check that passes before the work is done proves nothing after it, and a card whose work is already done should not be run (the roadmap left `already_passes` out). When a card becomes ready the gate runs its checks at the base through the check ledger, in the base worktree with a leased target and at the lowest slot rank, and records `Base { Red, Green, NoTest, Broken }` on the card: it failed as a test fails, it passed, it matched no test, or it did not build. The proof of a red base is kept for the verdict.
  acceptance: `cargo test -p ra-console -q base_is_seen` passes, with four scripted checks giving the four answers and the base's tree run once for two cards that share a check.
  files: crates/ra-check/src/base.rs crates/ra-check/src/base/tests.rs crates/ra-console/src/walk/schedule.rs crates/ra-console/src/commands/tests/checks.rs
- A card green on its base is held with a question, not dispatched {c2}
  objective: Either the work is done or the check does not test it, and a worker cannot tell which. The card is held with `base: green` and the person is asked once: accept the card as done, or say the check is wrong, which returns the card to the planner for one repair turn with the passing output. A look-back's or a planner's own card that is green on its base is repaired without asking.
  acceptance: `cargo test -p ra-console -q green_base_is_held` passes, with a scripted card whose check passes at the base held and never given a seat.
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/plan/propose.rs crates/ra-console/src/commands/tests/checks.rs
- A base that does not build is main's, and only the cards that need it wait {c2}
  objective: `Base::Broken` on one card is `Broken` on every card of that crate. The walk asks `classify` once, treats the answer as main's failure the way live card #235 treats a failing test, and holds only the cards whose checks need the broken crate by `select`. The rest go on.
  acceptance: `cargo test -p ra-console -q broken_base_holds_its_crate` passes, with two scripted cards of which only the one in the broken crate waits.
  files: crates/ra-console/src/walk/schedule.rs crates/ra-console/src/commands/tests/red_main.rs

### The test is written first and judged alone
- A spec pass writes the card's tests and nothing else {c4}
  objective: When the worker writes the test and the code in one run, the test is shaped by the code it wrote. For a card whose base is `NoTest`, a first run of at most 12 turns gets the contract and the map and may write only test files and the smallest stubs that let them compile. It ends when the card's own check builds and its named tests fail: for cargo, `cargo test --no-run` and then the filter; for any other tool, the check run once and read by `ra_check::output` as failed tests and not as a build error. Its commit is the spec, the first commit of the card's branch, and the stage `SpecWritten` is on disk like any other. The pass is routed as its own kind, `spec`, so the router can give it the strongest rung.
  acceptance: `cargo test -p ra-console -q spec_pass` passes, with a scripted worker that edits a source file beyond a stub sent back once, and one that writes a failing test moved on to the code stage.
  files: crates/ra-console/src/walk/spec.rs crates/ra-console/src/walk/spec/tests.rs crates/ra-console/src/agent/prompt.rs crates/ra-console/src/router.rs crates/ra-gotchas/src/lib.rs
- The spec is judged against the criteria before any code is written {c3}
  objective: A wrong test caught now costs a 12-turn pass; caught at the review it costs the card. The reviewer gets the contract's criteria and the spec's diff and nothing else, and answers for each criterion with the assertion that would fail if it were not met, as a path and line, or `none`. A criterion with an assertion is bound to its tests in the contract; one with `none` stays for the final review; a spec with no criterion bound is sent back once.
  acceptance: `cargo test -p ra-console -q spec_is_judged` passes, with a scripted verdict binding C1 to a test and leaving C2 unbound.
  files: crates/ra-console/src/agent/review.rs crates/ra-console/src/walk/spec.rs crates/ra-roadmap/src/contract.rs crates/ra-console/src/commands/tests/review.rs
- The code pass starts from the red spec, and its prompt is the failing test {c3}
  objective: "Make these tests pass in these ranges" is the smallest form a coding task has, and the form in which a cheaper model does best. The code run's prompt is the contract, the map and the spec's failing output from the ledger, with the card's objective kept to one paragraph. It is routed as kind `code`, which the ladder may start one rung lower than a card of the same size with no spec.
  acceptance: `cargo test -p ra-console -q code_pass_reads_the_spec` passes, with the prompt of a scripted card holding the failing test's name and its assertion line.
  files: crates/ra-console/src/agent/prompt.rs crates/ra-console/src/walk/spec.rs crates/ra-console/src/router.rs crates/ra-console/src/commands/tests/run.rs
- A weakened spec is a fact the verdict carries {c3}
  objective: The easy way to green is to change the test. `spec_delta` (new, in `ra-check`) compares the spec commit's test files with the tree's: tests removed, assertions removed or changed, a test marked ignored. Additions are free. Each loss is a proof line `spec weakened: <test> at <path:line>` that the worker is shown once in the run, with the chance to restore it or to say why in one sentence, and that the reviewer then rules on as its own point.
  acceptance: `cargo test -p ra-check -q spec_delta` passes, with an assertion removed, a test ignored and a test added giving two losses.
  files: crates/ra-check/src/spec.rs crates/ra-check/src/spec/tests.rs crates/ra-check/src/lib.rs
- The spec pass is spent only where it pays {c2}
  objective: A second run on every card would give back the floor the roadmap took out. The pass runs when the base is `NoTest` and the card is c3 or above, or its project's first verdicts are under 70%; a card of c1 or c2 elsewhere keeps the single run with the red-base rule alone. The rule is one function, `wants_spec`, so phase 13 can put a prediction in its place.
  acceptance: `cargo test -p ra-console -q wants_spec` passes with one test for each row of the rule.
  files: crates/ra-console/src/walk/spec.rs crates/ra-console/src/walk/spec/tests.rs

### The tests test the change
- `rc prove` reports which changed lines the card's tests ran {c4}
  objective: A test can be red on the base and green on the tree and never reach half of what the card changed. A new table in `rhipper.toml`, `[check.coverage]`, names a command that writes an lcov file for a test filter (`cargo llvm-cov` for the agent, the JaCoCo task for runelite). `coverage.rs` parses lcov, meets it with the diff's changed lines, and gives a proof line `the card's tests run 41 of 48 changed lines; not run: <path:lines>`. A project with no such table gets no line and loses nothing.
  acceptance: `cargo test -p ra-check -q coverage::` passes, with a scripted lcov file and a diff of five changed lines of which three are run.
  files: crates/ra-check/src/coverage.rs crates/ra-check/src/coverage/tests.rs crates/ra-check/src/report.rs apps/rc/src/prove.rs
- Unrun changed lines go to the worker once, inside the run {c2}
  objective: The cheapest time to add the missing test is while the session that wrote the code is warm. When the lines run are over `[check.coverage] floor = "80%"` nothing is said. Under it the worker gets the unrun ranges once, with the rule that a line is either tested or named in one sentence as untestable. What remains goes to the reviewer as a fact, never as a rejection by itself.
  acceptance: `cargo test -p ra-console -q unrun_lines_once` passes, with a scripted run told once and its second proof going on to the review.
  files: crates/ra-console/src/agent/builds.rs crates/ra-console/src/agent/review.rs crates/ra-console/src/commands/tests/checks.rs
- A sample of mutants in the changed lines must die {c4}
  objective: Running a line is not checking it. `mutate.rs` makes mutants from a table that needs no parser: a comparison flipped, a boolean swapped, a number moved by one, `+` for `-`, a statement's line removed. Up to five are drawn, seeded by the tree's hash, from the changed lines the tests run. Each is applied in a leased scratch worktree and the card's tests alone are run at the lowest slot rank while the review goes on. A mutant that does not build is drawn again. A survivor is a proof line `a mutant lived at <path:line>: <before> became <after>`.
  acceptance: `cargo test -p ra-check -q mutate::` passes, with one test for each row of the table and a scripted run in which one of three mutants lives.
  files: crates/ra-check/src/mutate.rs crates/ra-check/src/mutate/tests.rs crates/ra-check/src/lib.rs crates/ra-console/src/agent/builds.rs
- Survivors are settled like lint: one turn in the session, then a fact {c2}
  objective: A survivor found after the session is gone costs a cold rerun, ten times a warm turn (efficiency review). While the session is worth resuming (`worth_resuming`) the worker gets the survivors once. After that, or when it answers that the mutant is equivalent, they are facts in the reviewer's brief.
  acceptance: `cargo test -p ra-console -q survivors_once` passes, with a scripted survivor sent once and a second one, found after the session's cache life, left for the review.
  files: crates/ra-console/src/agent/builds.rs crates/ra-console/src/commands/tests/review.rs

### A proven criterion needs no opinion
- The contract says, for each criterion, what proved it {c3}
  objective: The roadmap's proof lines say what the checks did, and nothing says which criterion each line answers. `Contract` gains `proof_of(criterion)`, giving the bound tests, whether they were red on the base and green on the tree, the lines they run and the mutants they killed, from the binding the spec's judge made and the ledger's rows. `rc verdict <id>` prints the criteria one a line, each with its proof or `unproven`.
  acceptance: `cargo test -p ra-roadmap -q proof_of` passes, with C1 proven by a bound test seen red then green and C2 unproven.
  files: crates/ra-roadmap/src/contract.rs crates/ra-roadmap/src/contract/tests.rs apps/rc/src/main.rs apps/rc/src/tests.rs
- The reviewer is asked only about what is unproven {c3}
  objective: The reviewer reads the whole diff to rule on criteria a test has already settled, and its rejections on those are the nits the roadmap had to forbid. The brief lists the proven criteria in one line each as settled, and asks for a ruling on the unproven ones and on the facts: a weakened spec, unrun lines, survivors, files outside the card's. The diff it is given is the hunks those criteria and facts name, with the rest as a stat.
  acceptance: `cargo test -p ra-console -q asks_only_unproven` passes, with a scripted card of three criteria, two proven, giving a brief that asks for one ruling.
  files: crates/ra-console/src/agent/review.rs crates/ra-console/src/commands/tests/review.rs
- A card with every criterion proven and no fact against it passes without a reviewer {c3}
  objective: An opinion on a proof adds a chance of a wrong rejection and nothing else. When every criterion is proven red then green, the spec is not weakened, the changed lines run are over the floor, no mutant lived, lint is clean and the diff stays inside the card's files, the verdict is passed by the machine and cites the proofs. `[verdict] machine = "off"` sends everything to the reviewer as before.
  acceptance: `cargo test -p ra-console -q machine_verdict` passes, with a scripted card meeting every rule landed with no call to the reviewer's provider, and one with a survivor reviewed.
  files: crates/ra-console/src/agent/review.rs crates/ra-console/src/commands/tests/review.rs crates/ra-console/src/commands/tests/land.rs
- One machine verdict in ten is reviewed blind, and the rate follows the disagreement {c3}
  objective: A rule that stops calling the reviewer also stops hearing it. One machine verdict in ten, by the card's hash, also goes to the reviewer with no word that the machine passed it; the landing does not wait. A rejection is an event `Disagreed` with the reviewer's point, a follow-up card through the gate, and a rise of the sample to one in five for that project until twenty in a row agree.
  acceptance: `cargo test -p ra-console -q blind_sample` passes, with a scripted disagreement writing the event, the card and the new rate.
  files: crates/ra-console/src/agent/review.rs crates/ra-console/src/walk/landing.rs crates/ra-console/src/commands/tests/review.rs crates/ra-record/src/lib.rs

### A bug is traced to the card that let it through
- A fix is tied to the landing whose lines it changes {c3}
  objective: The measure of every rung above is whether what it passed stayed right, and nothing records that. A card is a fix when it was written for a red main, a revert or a reviewer's disagreement, or carries `fixes #N`. At its landing `escapes.rs` blames the lines it removed or changed, finds the landing commits they came from within 60 days, reads the card ids from the squash subjects, and writes `Escaped { fix, origin, lines }`.
  acceptance: `cargo test -p ra-console -q escape_is_traced` passes, in a scripted repository of two landings where a fix card changes a line of the first.
  files: crates/ra-console/src/escapes.rs crates/ra-console/src/escapes/tests.rs crates/ra-console/src/walk/landing.rs crates/ra-record/src/lib.rs
- A red main is bisected to its landing over proven trees {c3}
  objective: `main_failures` knows that main is red and not who made it so. The ledger holds a proof for each landed tree, so the search is over the landings since the last tree where the failing check is proven green, running only that check, a logarithm of them. The landing found is an escape of its card, and the card written for the failure carries `fixes #N` and the failing output.
  acceptance: `cargo test -p ra-console -q bisects_a_red_main` passes, with eight scripted landings and a failure from the sixth found in three runs of the check.
  files: crates/ra-console/src/escapes.rs crates/ra-console/src/escapes/tests.rs crates/ra-check/src/ledger.rs crates/ra-console/src/commands/tests/red_main.rs
- `rc escapes` prints the rate for each project, by how the origin was passed {c2}
  objective: The question is which rung leaks. The table gives escapes for each 100 landings by project and by the origin's verdict: machine or reviewer, with a spec pass or without, with mutants or without. It becomes the pair of first verdicts passed in the channel's guard.
  acceptance: `cargo test -p rc -q escapes_prints` passes, printing `agent  2 of 140  machine 0 of 60  reviewer 2 of 80`.
  files: apps/rc/src/main.rs apps/rc/src/tests.rs crates/ra-console/src/escapes.rs crates/ra-record/src/board.rs crates/ra-console/src/channel/guard.rs
- An escaped case scores the proof rules on the bench {c2}
  objective: A change to a proof rule should be judged by the bugs it would have caught. An escape marks its origin's case. A replay named `proof` runs the machine verdict's rules over each marked case from its kept proofs and scores the escapes the build would not have passed, beside the clean cases it would have sent to a reviewer for nothing.
  acceptance: `cargo test -p ra-console -q proof_replay` passes, with two scripted escaped cases, one of which a stricter floor would have stopped.
  files: crates/ra-console/src/bench.rs crates/ra-console/src/bench/tests.rs crates/ra-bench/src/case.rs

#### Phases 13 to 22: milestones and sections

No cards are written under these. Each is planned by the walk when its waits are accepted, by the planner of that day, from the record of that day, through the gate. A section's line is its objective and the brief the planner starts from.

## A card's cost and its chance are known before it runs
objective: Today a card's size is a guess on a scale of five and its route a rule; the run is the first thing that finds out whether the card can pass. With cases to learn from and specs that make cheap routes viable, every ready card gets a prediction (turns, tokens, the chance it passes first time, each from like cases and each with its count), a card under the bar is split, scouted or asked about and not run, and the route is the cheapest one a fitted table says holds the bar, with local priced first.
acceptance: Every child accepted, and over the next 100 landings: the median error of predicted turns is 30% or less, cards predicted at 0.8 or over pass first time at least 80% of the time, no card predicted under 0.5 was dispatched whole, and a quarter of all tokens ran on the person's machines.

### An estimate comes from like cases, and says how many
objective: `like(card)` ranks the fit split's cases by kind, size, files and words; the estimate is the median turns and tokens of the nearest, with the spread and the count, replacing `Estimate` as a guess on the node.
### Every ready card carries the chance it passes first time
objective: The share of like cases that passed first on the route in question, shrunk toward the project's and then the pool's rate where the count is small, shown as `0.91 from 34`.
### A card under the bar is split, scouted or asked about, not run
objective: Under `[predict] bar` the card returns to the planner with its like cases that failed and why; the planner splits it, sends a 12-turn read-only scout whose pointer lines join the card, or writes a question.
### The route is a fitted table, and the person's rules sit over it
objective: For each kind of card, size and project, each route's first-pass rate and cost from the record; the walk takes the cheapest route over the bar, a `[[route]]` rule still wins, and a cell under ten cases falls back to the rule.
### A price puts tokens, minutes and the week's room on one scale
objective: A local token costs seat-time only, a remote one costs room that grows dearer as the plan window empties; the same card routes differently on Monday and on the last evening of the window.
### Every prediction is scored when its card lands
objective: Predicted against actual for turns, tokens and the first verdict, by policy version; the error is a row of the bench and a guard of the channel, so a build that predicts worse does not promote.
### Room the week would leave unspent goes to the live bench
objective: When the marks of live card #199 say the window will end with room, the walk spends it at night on live replays of the cells the tables know least.

## The agent knows the code as a graph
objective: The roadmap's map finds a card's ranges by words, the selection works by crate, and the overlap rule by file. Each is a coarse answer to a question about items and what refers to them. The atlas holds every item and reference of every project, keyed by blob so a commit re-reads only what it changed, with the record laid over it; the map, the selection, the overlap and the likeness of cards become four queries of one graph.
acceptance: Every child accepted, and on the bench the map names 90% of the hunks a landing changed, the selection misses fewer than 1 failed test in 100, and over the next walk two cards naming one file ran side by side and a worker's first edit came within two calls of its start at the median.

### Items and references for every file, keyed by blob
objective: A new crate `ra-atlas` parses each blob once with tree-sitter into items (functions, types, tests, keys) and name references, stored by blob hash under the hub so worktrees and projects share it.
### Depth from the language's own server, where there is one
objective: rust-analyzer for the agent and asm's tools, a Java server or a SCIP index for runelite, asked lazily for resolved references and cached by the same key; every query says which layer answered.
### The record is laid over the graph
objective: Four overlays from events: the cards that changed an item, the tests that run it (from the coverage of phase 12), what it has cost in reruns, conflicts and escapes, and the notes runs left on it.
### `where` takes the map's place
objective: The card's words find items, the graph widens them to callers, implementers and tests, the cost overlay ranks them; scored against the lexical map on the bench and installed only as `better`.
### `impact` takes the crate graph's place in the selection
objective: The tests that reach the changed items, from references and the coverage overlay, falling back to the crate graph for an item no test is known to reach.
### `overlap` lets two cards share a file
objective: The scheduler's claim becomes the items a card is predicted to change; cards that meet are ordered, cards that do not run together, and a conflict that happens anyway corrects the prediction.
### Notes live on items, and reach only the cards that touch them
objective: A retro's note is attached to the items its run changed and rides in the prompt of a card whose map names them, in place of one shared `notes.log` every worker reads.
### The context file is rendered, not written
objective: `rhipper.context.md` is generated from the atlas (crates, entry points, the most-changed and most-costly items) and a test fails when the checked-in file is stale.

## The run's context is the orchestrator's
objective: The cost of a run is its turns times its context, and the context of a Claude Code session grows with every read and never shrinks. The agent already has a tool loop for bare models (`tool_loop`, agent.rs:7450, and `ra-tools`). Grown into a loop where the orchestrator decides what stays in context, it serves every route that is not Claude Code, and takes any role from the Claude Code seat where live replays say it is cheaper for the same verdicts.
acceptance: Every child accepted, and over the next 100 landings the median turn is 15k tokens or less, a c3 first run is 12 turns or fewer at the median, and no route's first-pass rate fell by more than the guard's limit.

### One loop for every model that brings no tools of its own
objective: A new crate `ra-run` holds the loop: tools from `ra-tools`, the hooks of `ra-gotchas` in-process, stages on disk, the same typed stops as the Claude Code adapter.
### A range leaves the context when it is edited or stale
objective: The loop owns the message list; a read is replaced by its pointer once the range is edited, superseded or untouched for a set number of turns, and can be read again at no more than its first cost.
### A build's output stays as its failures
objective: Tool results are kept as `ra-check` parses them (the failing tests, the first error with its lines), never as the raw log.
### A search is a scout's, in a context that is thrown away
objective: A question of the form "where is" or "what calls" spawns a short read-only run on the cheapest route, and only its pointer lines return to the worker's context.
### The atlas is a tool
objective: `where`, `callers`, `tests_of` and `item` (a function's body by name) are tools of the loop, so most reads are one call with no search before them.
### Edits are by item
objective: A tool replaces a named item's body or inserts after one, resolved by the atlas, with the string edit kept as the fallback; the count of failed edits is its measure.
### The prefix is cached where it is stable
objective: System prompt, contract and map are ordered and marked so a provider's cache or a local server's saved state serves them on every turn; cache hits by turn are in the usage rows.
### The seat is chosen by measure
objective: For each role the live bench runs the same cases on the Claude Code seat and on the loop; the route table takes whichever is cheaper at equal verdicts, and says so.

## A sentence in, a proven plan out
objective: The person writes cards today, or approves a planner's cards one at a time, and a look-back every ten landings proposes more. With red checks at plan time, predictions and the atlas, a plan can be a checked object: the person gives an intent and gets a card graph that has already passed the gate, with what it will cost, where it is unsure, and the questions that have to be answered first.
acceptance: Every child accepted, and over a month 80% of planned cards land as written, the person edits fewer than one planned card in ten, and no card was written by hand.

### `rc intend` turns a sentence into a plan file
objective: The intent, the atlas's `where` for it and the like plans from the record go to the planner; the answer is `plans/P-<n>.md` in the outline form, with waits, not yet in the roadmap.
### A plan's checks are red before it is shown
objective: Every card of a plan passes the gate and has its base seen; a card that is green or cannot be checked is repaired by the planner before the person sees the plan.
### A plan carries its predictions and its questions
objective: The header of a plan is its cost, its width, its wall time on the machines at hand, its riskiest cards and the questions whose answers change it, each with what waits on it.
### A criterion names the kind of proof it needs
objective: The planner writes each criterion as a test, a property over generated inputs, an agreement of an old and a new implementation, or a bound on a benchmark, and the prover gains a rung for each; the `[[check.contract]]` tables the roadmap left out return here as declared proofs.
### What is unknown is scouted before it is carded
objective: A scout card reads and reports and changes nothing; its report is kept on the items it read, and the cards that follow are planned from it.
### A design is a card whose landing is a decision
objective: A choice between approaches is a card that lands a short decision record with the options weighed; the cards after it cite it, and a later plan finds it through the atlas.
### A plan is revised as it runs
objective: A landing that changes what later cards assume, a failed prediction or an answered question reopens only the cards it touches; this takes the place of the look-back's clock (`EVERY = 10`).
### A dependent starts stacked on the card it waits for
objective: When a card's wait is proven and queued to land, the dependent starts on its branch and is rebased by the landing; the wait costs minutes, not a landing.
### The agent files its own intents
objective: Friction the record shows (a cell with a poor rate, a costly item, a rule that fires on passing runs) becomes an intent on the agent's own roadmap with its predicted saving, and after it lands the measured saving is set beside the prediction.

## One hub, every machine the person owns
objective: The hub is a directory of lock files on one machine, and a walk is a process there. The person has more machines than that and wants all of them used and none of them needed. The hub becomes a daemon that leases seats, slots and stages; every machine joins as a node; code moves by git, proofs by their hash, the record by one log for each writer; and a walk outlives any one of them.
acceptance: Every child accepted, and a walk runs on three machines for a week in which a node is killed mid-run, the laptop is closed mid-walk and the hub is moved, with no landed work lost, no stale work landed, and work resumed within 5 minutes each time.

### The hub is a daemon, and its lock files are leases
objective: A new crate `ra-hub`: one process owns the pool, the slots and the priority order, and grants each for a time with a fencing token that staging and landing must present.
### A machine joins as a node and offers what it has
objective: `rc node join <hub>` over the person's own network (SSH or a tailnet), declaring seats, build slots, toolchains, GPUs and plan windows; the scheduler's `plan` gains a node in its answer and a reason for it.
### The record is one log for each project and machine
objective: One writer a file, stamped with a hybrid clock; a node ships its log to the hub and reading is a merge by stamp, so no write ever waits on another machine.
### Code moves by git, and main by fast-forward alone
objective: Each node works in worktrees of its own clone and pushes card branches to the hub's bare repository; only the lander moves main, and a fast-forward that fails is a retry, never a force.
### A proof made on one node holds on every node with its stamp
objective: The check ledger becomes a store keyed by tree, command and environment stamp, shared through the hub; a node proves only what no node with its stamp has proven, and `rc verify <commit>` prints a landing's proofs and runs them again on request.
### A node that dies loses its leases and nothing else
objective: Its stages are on the hub's copy of the record and its branches are pushed at each stage; when the lease runs out another node takes up the run from its last stage.
### A model served on the person's GPU is a seat like any other
objective: A node declares the models its server holds; the route table sees them as routes with a seat count, and the scheduler queues for them as it queues for a build slot.
### The window is a view from anywhere
objective: `rc` and `rc-gui` on any node read the hub's snapshot and send it the person's commands; a machine that only watches holds nothing a walk needs.

## Local models earn their roles
objective: A local model today is a setting for the cheap roles. The person wants local execution everywhere it holds up, and the way to know where it holds up is the holdout. The cases become a dataset that never leaves the person's machines, each role gets an exam, small models are trained at home on the idiom of the person's own code, and a model takes a role by passing the exam and keeps it by its live rate.
acceptance: Every child accepted, and over a month half of all tokens ran on the person's machines, every cheap role and the code pass of at least half the cards with a spec ran local, and neither the first-pass rate nor the escape rate moved past the guard's limit.

### The dataset is the person's own landed work
objective: The fit split's cases render to rows for each role (a red spec and its green diff, a failed check and its fix, a card and its map, a brief and its verdict), kept under the hub and never sent anywhere.
### Every role has an exam from the holdout
objective: For each role a fixed set of holdout cases and a machine score (the spec goes green, the fix passes, the verdict agrees with the record); `rc exam <role> <route>` prints it.
### An adapter is trained at home, when the GPU is idle
objective: `rc train <role>` fits a small adapter to an open-weights coder on the dataset, on the GPU node, as a job the scheduler places like any other, with its dataset's hash in its name.
### A model takes a role by its exam and keeps it by its live rate
objective: A route enters the table for a role when it holds the incumbent's exam score, and leaves when its live rate falls under the bar; both moves are events with the numbers.
### Forms a small model cannot get wrong
objective: Edits, verdicts and reports are decoded under a grammar on local routes, so an off-form answer is impossible, not retried.
### The code pass on a red spec goes local first
objective: The smallest task form meets the cheapest route; the remote rung is the fallback after the progress rule's two stalled rounds, starting from the local run's tree.
### What leaves the machine is counted, and can be zero
objective: Tokens sent off the person's machines by project and role, in the window; a project marked local-only is routed only to local seats and waits when none can hold its bar.

## The person answers questions and reads verdicts, and nothing else
objective: The roadmap makes the window true. The endgame makes it short. Everything the agent needs from the person becomes a question with options, a default and a deadline; everything it has to tell them becomes one screen in the morning; anything else they want to know they ask, and are answered from the record. The person's minutes become a measure a build of the agent can fail.
acceptance: Every child accepted, and over a month the person spent 30 minutes or fewer for each 100 landed cards by the attention ledger, no walk waited on the person past a default's deadline, and the monthly audit sample found no landing whose verdict misdescribed it.

### A question is a typed thing with options, a default and a deadline
objective: Every hold for the person, approval and planner's ask becomes one `Question` in the record: the fewest words, the evidence, the options, what waits on it, and what happens at the deadline.
### Work that does not need the answer goes on
objective: A question holds only the cards that wait on it, by the same waits as any card, and the scheduler's reason for them is the question.
### The morning report is one screen
objective: Since the person last looked: landed by project and how proven, spent and where, what changed in the agent and why, and what needs them with the minutes it will take.
### `rc ask` answers from the record, with the events it read
objective: A question in prose becomes queries over the folds; the answer cites event ids, and an answer the record cannot support says so.
### The person's minutes are a ledger and a guard
objective: Each question's time to answer and each approval, summed by week for each 100 landings; a candidate build that asks more for the same landings fails its guard.
### An answer given three times becomes a rule the agent proposes
objective: Like questions answered alike are offered back as a standing rule in `rhipper.toml`, itself one question, and the rule's later exceptions are counted.
### One landing in fifty is put before the person
objective: A random landing with its verdict and its diff, to read or to skip; what the person finds that the verdict did not say is an escape of the proof system and is recorded as one.

## Nothing is lost and anything is undone
objective: The roadmap makes a walk survive its own death. The endgame asks the same of everything else: a landing can be taken back with proof, a red main mends itself by a default, any past moment can be looked at again, a provider's outage moves work and does not stop it, a lesson that made things worse retires itself, and every one of these is drilled weekly and timed.
acceptance: Every child accepted, and for a month `rc drill --all` runs weekly with every drill recovered inside its bound and no person asked.

### `rc undo` takes back a landing with a proven revert
objective: The revert is built in the landing worktree, proven like any tree, and lands by fast-forward; cards stacked on the landing are reopened with the reason.
### A red main is mended by a default
objective: The bisect of phase 12 ends in a question with a proven revert and a fix card both ready, and a default that is taken at its deadline; cards whose checks do not reach the failure never stop.
### Any moment of a walk can be looked at again
objective: `rc replay <walk> --at <event>` folds the record up to that event and shows the board, each decision and its inputs, as the window showed them then.
### A provider's outage moves the work
objective: A typed stop from a provider reprices its routes to unavailable; cards another route can hold at the bar move, the rest wait with that reason, and the first success moves them back.
### A lesson that made runs worse retires itself
objective: Every note, hook rule and prompt piece is withheld from one run in five that it fits; one whose runs do no better, or worse, is retired with its numbers, and the retirement is an event.
### The hub is rebuilt from the record
objective: The roadmap, the repositories and the record are everything; `rc restore <dir>` on a fresh machine rebuilds the hub, the ledgers, the atlas and the tables, and a test does it.
### Every recovery is drilled and timed
objective: `rc drill` kills a worker, a walk, a node, the hub and a provider, fills a disk and corrupts a worktree, each in a sandbox project, and prints the seconds until work resumed against its bound.

## Forks trade proven changes
objective: Every person who runs the agent runs their own fork, improved by their own projects. What one fork proves, another should be able to take without trusting it: as a package that carries the change and its evidence, enters through the taker's own bench and canary, and holds nothing of the giver's projects.
acceptance: Every child accepted, and a change proven on one fork is installed on a second by the second's own channel with no hand edit, the package having passed a check that it holds no path, name or line of the first fork's projects.

### A change to the agent is a package
objective: `rc pack <cards>` writes the commits, the cards as landed, and the bench's verdicts as counts by replay, with the agent's version it applies to.
### A fork takes a package through its own bench and canary
objective: `rc take <package>` applies it on a branch, runs the taker's cases, and installs it as a candidate like any landing; the giver's numbers are shown and decide nothing.
### Rules and notes travel with their measured effect
objective: A gotcha rule, a prompt piece or a route cell can be packed alone, with the counts of its withheld-run experiment, and is put through the same experiment on the taker's runs.
### Nothing of a person's projects is in a package
objective: A packer that reads only the agent's repository and the counts, a scan of the package for the hub's project names and paths, and a charter rule over both.
### A fork can follow another
objective: `rc follow <remote>` takes each build the other fork promoted as a package, in order, so a fork with one project gains from a fork with ten.

## A month of walks nobody watched
objective: The endgame is a claim about a month of the person's life, and it should be checked like a card. For thirty days the agent walks every project from intents alone, on the person's machines, changing itself as it goes, while each measure of the endgame is kept on one page, and at the end the person reads that page and a sample, and what fell short becomes the next roadmap.
acceptance: Every child accepted, and `rc endgame` prints every row of the endgame's measures at or past its target for thirty days running.

### Every measure of the endgame is on one page, by week
objective: `rc endgame` prints the table of this file's measures from the record, each with its target, its four weekly values and the events behind the worst week.
### The proof system is audited by a person and a second model
objective: Fifty landings of the month, at random, are read by the person or a model that has not seen their verdicts; every fault found that no escape recorded is the audit's measure.
### What ran where is audited
objective: Tokens by machine, by route and by role for the month, what left the person's machines and for which cards, set against the local share's target.
### The drills ran for the month
objective: Four weekly runs of the whole drill suite, their bounds and their times, and the one that came closest to its bound.
### What the month showed becomes the next roadmap
objective: Each measure short of its target becomes an intent on the agent's roadmap with the month's evidence, filed by the agent, approved by the person.

#### After the import

Check the outline with `rc import docs/fable/agent-endgame.md --under 1 --dry`, then import it without `--dry`. I checked this file against the parser by reading `parse` in outline.rs, not by running it; the dry run is the proof. It should report 13 milestones, 85 sections and 65 cards.

This file carries no `waits:` lines, because until the first card of phase 10 lands the parser would add such a line to the card's objective. So the waits are set by hand once more, with `rc depend <id> <id>...`. A card is named below by the test filter of its acceptance. A name in **bold** is a card of `agent-roadmap.md`, named as that file's last table names it.

"Chain" means each card of the section waits on the card above it. "Any order" means the section's cards wait on nothing inside it.

| Phase | Section | Inside | Waits from outside the section |
|---|---|---|---|
| 10 | The roadmap carries its own order | any order, but `planned_cards_wait` on `outline_waits` | none: this section can be walked beside the roadmap's first milestone. If the roadmap's carve of the test file has landed first, the test of `plan_next_passes_over` goes in `commands/tests/plan.rs`. |
| 10 | A landed card leaves a case | chain | `board_` on **`board_`**; `dispatch_keeps_the_card` on **`walk::ledger`**; `bench_build` on **`map_eval`** and **`landing_stage`**; `red_on_base` on **`check_rows`** |
| 10 | The deciders replay with no model call | each on `replay::`, `bench_prints` last | `map_replay` on **`map_eval`**; `select_replay` on **`select`**; `gate_replay` on **`admit_faults`**; `cause_replay` on **`cause`**; `prompt_replay` on **`prompt_carries_the_map`**; `rules_replay` on `calls_are_rows` |
| 10 | Two builds are compared before one is trusted | any order | each on `bench_prints` |
| 10 | The bench is checked against the walks | any order | `calibrate` on `compare::` and on `measures_by_build` of phase 11; `live_case` on **`stage_is_on_disk`**; `review_replay_row` on **`replay_agreement`** and `live_case`; `bench_panel` on **`refresher_`** |
| 11 | A build is kept, named and benched | chain | `measures_by_build` on `board_`; `install_asks_the_bench` on `compare::` |
| 11 | A candidate walks the agent's own project first | chain | `canary_is_the_agent` on **`newer_install`**; `rolls_back_on_fail` on **`walk::wake`** |
| 11 | The agent's own landings become candidates | `shadow::` on `decisions_carry_inputs` | `landing_builds_a_candidate` on **`landing_stage`** and `promotes_on_pass`; `decisions_carry_inputs` on **`walk::schedule`** |
| 11 | What the agent may not change alone | chain | `charter_parks` on **`landing_stage`**; `charter_test_missing` on `install_asks_the_bench`; `approvals_list` on **`snapshot::`** |
| 12 | A check is seen red on the base | chain | `base_is_seen` on **`proven_tree`**, **`walk::schedule`** and `red_on_base` |
| 12 | The test is written first and judged alone | chain | `spec_pass` on **`stage_is_on_disk`**, **`prompt_carries_the_map`** and `base_is_seen`; `spec_is_judged` on **`contract_of`** and **`points_cite_criteria`** |
| 12 | The tests test the change | chain | `coverage::` on **`rc_prove`**; `unrun_lines_once` on **`continued_in_session`** |
| 12 | A proven criterion needs no opinion | chain | `proof_of` on `spec_is_judged` and **`proof_lines`**; `asks_only_unproven` on **`delta_review`**; `machine_verdict` on `mutate::` and `spec_delta` |
| 12 | A bug is traced to the card that let it through | chain | `escape_is_traced` on **`landing_stage`**; `bisects_a_red_main` on **`proven_tree`**; `escapes_prints` on `guard::`; `proof_replay` on `machine_verdict` and `replay::` |

The milestones of phases 13 to 22 take their waits on other milestones. A wait on a milestone does nothing until `waits_above_hold` and `plan_next_passes_over` have landed, which is why those two cards come first in this file and wait on nothing; set these when you import, and they take effect when those cards do.

| Milestone | Waits on the milestones of phases |
|---|---|
| 13, a card's cost and its chance | 10, 12 |
| 14, the code as a graph | 10 |
| 15, the run's context | 13, 14 |
| 16, a sentence in | 12, 13, 14 |
| 17, one hub | 11, and the roadmap's sixth and ninth |
| 18, local models | 13, 15, 17 |
| 19, questions and verdicts | 12, 16 |
| 20, nothing is lost | 11, 17 |
| 21, forks | 11 |
| 22, a month nobody watched | 13 to 21 |

One caution until then. Gambit g3's plan-next plans the first milestone without cards whenever a walk has nothing ready. With the roadmap's 151 cards and these 65 ahead of it that moment is far off, but a walk in which everything left is blocked would reach it. If that worries you, import phases 13 to 22 only after `plan_next_passes_over` is accepted: cut this file at the heading "Phases 13 to 22" for the first import and import the rest later with the same command.

