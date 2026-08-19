# Warlock

**Org:** Genetic Pottery
**Product:** Warlock
**Language:** Rust
**GitHub description:** See your codebase the way your AI does. A TUI where documentation is the interface.

---

## 1. What it is

A terminal UI for operating a codebase through its documentation tree. You direct an AI rather than write most code by hand, and the interface is the project rendered as the AI sees it: a tree of module READMEs, colored by whether they are fresh or stale.

## 2. Thesis

Most teams already work this way and will not admit it. Engineers direct AI quietly. They strip the em dashes, they do not let the AI commit, they hide how much of it was the model. A whole room of people using AI while performing restraint, which means they are using it poorly, with no structure, no shared context, no record. The pretending is the waste.

Warlock makes the real workflow legible and structured instead of hidden, so it survives handoff, onboarding, and the agent not being you.

Core bets:
- Engineers are AI drivers, not engineers + AI.
- The work is human-gated. No automagic.
- Process artifacts are produced as a byproduct of doing the work, not as a tax paid afterward. Most companies LARP process: the ticket exists, the doc exists, and both are one-sentence husks. Warlock's artifacts are real by construction because they are load-bearing.

The payoff is leverage, not just honesty. Maximum project context on tap, and the ability to draft a well-formed intent before a single ticket is cut. Spend eight minutes acting as PM with the AI, pressure-testing the desire, and you get a clean project brief and clean tickets from it. Eight minutes up front against eight months of fumbling.

Important caveat to hold onto: the quality of that ticket comes from real thinking, captured and structured. The pipeline does not manufacture thought. The failure mode in a world where everyone uses this is a lazy one-liner inflated into a beautifully formatted page with nothing behind it. Long and well-structured is not thought-through. **The eight minutes is the product. Warlock makes sure the eight minutes is not lost.**

## 3. Guiding principle

**Warlock makes the right thing visible and easy. It never makes the wrong thing impossible.**

This shows up in three places: manual file edits are allowed, pact expansion is a proposal rather than a block, and pushing stale docs is discouraged rather than prevented. Clear path, no walls. If you want to wander, that is your choice.

## 4. The three sources of truth

Each fact has exactly one home. The others reference it, never copy it.

| Store | Owns | Notes |
|---|---|---|
| **Engineer's journal** | Why decisions were made | Private, isolated, not audited. Lives at `~/.warlock/<project>/<date>` |
| **The repo** | What the system *is* | Code + module READMEs + state files. Committed to git. |
| **Linear (for now)** | What *should happen* | Tickets, intent, gate workflow state. Behind an adapter, swappable. |

The journal's serious job is reasoning capture, not activity logging. The repo says what the code is, Linear says what the tickets were, and the journal says why you made the calls you made. That last one is what actually evaporates today and what no other tool captures. Annual review recall is the cheap demo of that value.

State files and READMEs are committed to git. A teammate clones the repo and their AI driver picks up full context immediately. Context lives with the code, not in anyone's head or any one subscription.

## 5. The tree

The project tree is the interface. It renders every directory in the project that is not ignored, so the first thing anyone sees on a fresh repo is their whole codebase in gray, waiting to be pacted.

**What is in the tree:** every directory git would not ignore. Traversal honors `.gitignore` at every level, hidden directories, and global excludes, plus Warlock's own `.warlock/`. This is deliberately not a skip list Warlock maintains. If a project does not ignore `node_modules`, Warlock walks into it, and that is the project's call to fix.

**Colors:**
- **Gray** = not pacted. Outside Warlock's management.
- **Yellow** = pacted and stale. Files at or below this node's README have changed since the last freshness grant.
- **Green** = pacted and fresh. An AI has checked and granted freshness.

**The state model is deliberately two-way, with no limbo:**
- Stale is mechanical. The subtree hash broke, so it is stale. Immediately, by definition.
- Fresh is earned. Only an AI pass can grant it, by reading the diff and either confirming the docs still hold or updating them until they do. Either outcome ends green.
- There is no "unjudged" third state, because unjudged *is* stale.

**Files are shown, and take their module's color.** A file has no state of its own. It is green because the module holding it is green, and when it changes that module goes yellow. Those are the same fact stated twice, since a changed file is exactly what breaks a subtree hash. There is still no fourth state and no per-file pact.

Showing files is a toggle, off by default. The default view is modules, because that is the altitude the work happens at, but nothing is hidden from someone who wants to read code. Section 9's point stands: read files to direct an agent well, not to live in them.

**Collapsing is core navigation, not a convenience.** A real repo rendered whole is hundreds of rows. Space collapses and expands the selected directory. A filter to show only pacted nodes is the fast path to "just the part I am working on," and it is a view, not a rule about what the tree contains.

**Navigation:** viewing and reading is the primary mode. You click through the tree, open files, think, and judge. Editing is possible but is not the star of the show.

**Modular invocation:** run from any subdirectory or from root. Scope resolves from where it is invoked and auto-balances when run higher in the tree. No privileged root concept baked in.

## 6. Freshness: hash as trigger, AI as judge

Whether a README needs updating is a **subjective** call, and that is fine. A human engineer faces the identical judgment today: does a trivial change warrant a doc update? There is no mechanical answer and never was. Warlock is not automating away a solved problem, it is giving visibility to a judgment that was always being made silently.

We are not trying to be *correct*. We are trying to be *visible*.

Call this subjective coding, and keep it separate from correctness when pitching. The code still has to work. It is the *map* that is a judgment call. Documentation freshness being subjective is not the same as making correctness negotiable.

**Mechanics:**
- The subtree hash is the **trigger**, not the judge. Hash at grant time; a broken hash means "something happened here, go look." It gates the expensive AI pass behind a free mechanical check, and it catches edits made outside the tool.
- **Refresh is always manual.** No refresh on startup. The user triggers it on a given module or at root, and root refreshes every pacted module. This keeps launch instant and puts the cost where the user chose to spend it.
- The refresh pass is short-lived: parse, update, exit. Context stays small. Its purpose in life is to be a brief batch job.

**Known tradeoff:** with manual-only refresh, yellow means "changed since last check," not "changed recently." On a repo nobody refreshes for a week, the whole tree goes yellow and the signal degrades. The pre-push guard is therefore doing more work than it looks like, since in practice it is often what prompts a refresh.

## 7. Blessing (the human gate)

**One human gate, on the work.** The human blesses the change, and the documentation updating is a mechanical consequence of that same act. Nobody sits there separately confirming "yes, update my README." Freshness turns green as a byproduct.

A gate is a human decision about a ticket that has consequences for the repo. Today that decision lives in your head and your Enter key: real, but it leaves no trace, is not tied to the specific delta, and nothing stops a merge that skipped it. Blessing promotes it from a keystroke to a recorded, linked artifact: this human approved this delta as satisfying this ticket.

- The **gate decision** is repo state, a fact about what happened to the software.
- Linear holds whether a gate is **pending or cleared**, as workflow status.
- Doc updates land in git as a normal diff. **The git diff is the review surface for documentation**, since there is no in-app approval moment for it.

## 8. Pacts and expansion

A pact is the boundary of what Warlock manages. Where that boundary starts is a policy choice, not a rule the product enforces.

**Two adoption paths, one mechanism:**

- **Greenfield: pact everything on day one.** A new project has no reason to start partially managed. Pact at root, the AI works down the tree writing a README per module, and the whole thing comes up green. Full coverage is the default here, not an aspiration.
- **Existing codebase: pact per module.** Leadership at an established company is unlikely to bless the whole repo up front, and should not have to. Scoped adoption is a first-class path: pact one module, prove it, expand when the work demands it.

Both are the same pacting operation run over a different number of nodes. Warlock does not need to know which story a team is telling, and nothing in the product is tuned to prefer one.

**Who owns the guardrail.** A pact is a committed diff. If someone pacts far more than their team wanted, that is caught in review like any other overreaching change. Warlock does not police merges, and building it to would be inventing a problem that source control already has an answer for. Section 3 applies: visible and easy, never impossible.

**Crossings are events, not errors.** Most real changes touch more than one module. If work needs an unpacted module, that is not something to reject, it is a **pact expansion proposal** that a human blesses. The boundary grows on purpose, with a record, through the same human-gated act as everything else. Never auto-expand.

**Three checkpoints, each asking "does this fit the current boundary, and if not, here is the expansion to bless":**

1. **Project creation.** The AI drafts the project and estimates the module footprint. Unpacted modules are flagged at the cheapest possible moment, before tickets exist.
2. **Ticket cutting.** Each ticket carries a predicted footprint. Tickets that stay in-pact flow normally. Tickets that would cross are marked "requires pact expansion" and cannot proceed until the expansion is blessed.
3. **Implementation.** Not a check but an invariant. Earlier gates already settled the boundary; implementation just operates inside it.

**Backstop:** footprint prediction is a guess and will sometimes be wrong. If implementation reaches an unpacted file, stop and surface rather than auto-expanding. Predict early to make this rare; backstop at the end to stay honest when the prediction misses.

**The adoption ratchet**, which is the scoped-adoption path and not a universal law. Where a team starts small, pact expansion only ever points outward, so coverage only grows. You pact ApiServer, the auth dependency forces an expansion, auth's dependencies are the next tickets' crossings, and so on. A real codebase's dependency graph is connected, so following the work pulls the whole project under management over time. On this path nobody ever has to decide "let's Warlock the whole repo," because coverage arrives as a side effect of doing tickets, and no module gets pacted before there is concrete work needing it. Adoption is not sold, it is produced.

This describes what happens when a team will not bulk adopt. It is not an argument against bulk adoption: a team that is ready to pact root should, and a greenfield project should not be made to crawl through a ratchet it never needed.

**Risk:** on a tightly-coupled codebase the first real ticket may demand a huge expansion, making "incremental" feel like a lie. Have an answer for this even if the answer is "bless one big initial expansion and move on."

**Crossings as an architecture signal.** Crossing count is a diagnostic, but it conflates two different failures:
- Many crossings across *many different* modules from *many different* tickets suggests poor task scoping.
- The *same two modules* crossing repeatedly, ticket after ticket, is not bad tasks. Those two modules should probably be one, or the boundary is drawn in the wrong place.

Surface the second case explicitly ("auth and api have crossed in nine of your last ten tickets"). Do not ship a tool that shames the author. Redirect the friction from the person to the design.

## 9. The escape hatch

You *can* edit files directly, classic vim style. The hatch exists so the tool never traps you. But using it is a signal you have stepped outside the model Warlock is built for. This product is not optimized for individual-file editing and does not pretend to be. Expect small tweaks at best; anyone living in hand-edits is not the customer.

Manual edits are not fought, they are reconciled. Edit outside the garden, the subtree hash breaks, and the next refresh notices. Design for the intended path, tolerate the leak, account for it on the way back in.

## 10. Pre-push guard

A guard that dissuades pushing stale documentation. Warn, do not block. Ship a suggested CI check alongside it, but whether a team makes it blocking is their policy call, not the product's. Warlock ships the signal; the org decides enforcement.

## 11. AI invocation

Warlock does not call the Anthropic API over HTTP. It **invokes the `claude` CLI as a subprocess**. Spawn, feed prompt and scoped context, read stdout.

This means Warlock holds no credentials and is inert without a logged-in `claude` binary present. The user's subscription is literally the patron.

Two invocation lifetimes:
- **Freshness pass:** short-lived. Parse, update, exit. Small context.
- **The project-and-ticket pipeline:** longer-lived, the shape already defined and tested in the Red and Forman prototypes, roughly ten minutes of real talking and thinking with a human.

To decide during implementation: headless/print mode per task vs. a longer session, and how context is fed (piped stdin vs. files). Scoping the pacted tree into each invocation is the actual differentiator: maximal relevant context, minimal waste.

## 12. Technical shape

**Rust**, TUI via **Ratatui**. Good fit: filesystem walking, subtree hashing, parsing, watching, and single-binary distribution. Strong types make the unpacted/stale/fresh state machine hard to get wrong. Subprocess management via `std::process::Command` is straightforward, and since there is no HTTP API layer there is no serde-heavy client to build.

**Build order matters.** Build the freshness/state engine as a standalone, headless, tested library **first**, before any TUI. It is the load-bearing logic and the part where AI-assisted development most needs tight feedback loops. The TUI is a thin renderer over it. Inverting this means debugging semantics through a UI.

**Workspace layout:** engine crate + TUI crate. This also happens to be exactly the module separation needed for the free/paid split, so keep the boundary clean from day one.

## 13. Smallest shippable version

A TUI that:
1. renders the project tree, every non-ignored directory, collapsible,
2. colors pacted nodes from the hash-based freshness check,
3. lets an AI propose README updates on manual refresh.

Collapsing is in the first version rather than deferred, because section 5 renders whole repos and a few hundred uncollapsible rows is not a shippable view. The file toggle is not in it: files are specified in section 5 but nothing in the loop above needs them, so they wait.

That is already more than Red + Forman + a folder of prompts.

## 14. Licensing and repos

**Open repo, Apache 2.0:** the engine and the TUI. The complete standalone solo tool. Apache over MIT for the explicit patent grant.

**Private repo, proprietary:** the project-and-ticket pipeline, the Linear adapter, the gate-decision store, anything multi-person. The pipeline is modelled on Red and Forman — separate public prototypes that proved the shape — but it is a Warlock-native implementation of that process, not those tools vendored in.

**The seam: free where value is individual, paid where value is coordination.** One person driving their own AI is free. Multiple people coordinating through tickets and gates is paid. That maps almost exactly onto who has budget.

Keep the boundary **architectural, not licensing-enforced**. The paid layer is a separate crate talking to the open one through a defined interface. If license checks have to be sprinkled through shared code, the boundary was drawn wrong.

Do not build licensing infrastructure yet. Ship the free tool, get it good, see if anyone uses it.

## 15. Business shape

Bring your own Claude subscription. Warlock does not resell inference; it is the harness that spends an existing subscription well, scoped to the pacted tree.

Naming note: the warlock draws power from a patron it does not own. Vocabulary falls out cleanly if wanted: *patron* is the model, *pact* is the binding, *invocations* are agent runs.

## 16. Demo script

Film the loop closing. Nobody has to believe it, they watch the tree change color.

1. Frame it before opening the tool, in one line: "I do not write most of my code anymore, I direct AI that does, and this is the thing that keeps that from being a mess."
2. Open on the codebase. Show a gray node in passing so the pact concept lands without explanation.
3. "Some auth would be nice, or a new endpoint. But how do I do that?" Open the project flow, describe the task, watch it broken down into tickets.
4. Go to Linear. **Show human-written tickets next to AI-written ones.** This is the strongest beat. Let it sit on screen. Everyone watching has received the bad one.
5. Back to the code. Pull a ticket and work. Watch parts of the tree turn yellow as the AI makes changes.
6. Refresh. Narrate the latency rather than cutting it: "this is an AI reading the diff and updating its own understanding of the project." Yellow goes green. Loop closed.
7. Cherry on top: open `~/.warlock/<project>/<date>`. Read the line out loud, slightly surprised. "Today I worked on the API server, added a route and some auth." Cut. Do not explain it.

Order matters: freshness lands *after* the audience understands the AI reads these docs to work. Show it too early and green is just a color trick with no stakes.

## 17. Open questions

- Manifest schema fields. Decide early; migrations hurt.
- Validator as a separate CI-facing tool sharing the engine's freshness definition, or one tool with two entry points. Leaning: separate entry points, shared logic.
- How far the project-structure spec goes: lightweight convention plus CI check, or full opinionated scaffold.
- Fixed README section skeleton. The agent reads positionally, so the skeleton should be rigid even where the prose is loose. Candidate sections: purpose, public interface, dependencies, invariants.
- Pact granularity. Section 5 specifies one README per directory: node, directory and pact are the same thing. The alternative is a pact covering a *subtree*, one README at its top applying downward until the next pact, which lets the AI decide what counts as a module instead of the filesystem deciding for it. Directory-granular is simpler and is what is specified. Revisit if generated READMEs for trivial directories turn out to be noise that dilutes the signal, which is the failure mode to watch for on a root pact over a large repo.
- `claude` invocation mode and context-feeding strategy.
- Whether "bless" survives as the gate verb given the warlock theme, or becomes seal / sign / ward.
