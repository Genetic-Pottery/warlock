# File briefs, tickets and pulls to a Jira board

`crates/warlock-tui/src/linear.rs` holds the only `Board` in warlock, and every flow is written in Linear's words. `push.rs:173` resolves a team key to a team id and hands it to `create_project`; `cut.rs:88-101` resolves a team id, a backlog workflow state and an issue label id and hands all three to `create_issue`; `pulling.rs:565-577` resolves a team, then a workflow state by name, then moves an issue by state id. The trait at `linear.rs:155` is nineteen verbs, five of which exist only to turn a name into a global id that a later verb consumes.

Brief 29 recorded a `tracker` on each `ScopeRecord` and made `push`, `draft` and `pull` refuse a Jira scope at `LinearScope::of`, before a key is read or a socket is opened. That refusal is the whole of warlock's Jira support today: a person who records a Jira scope can bind a key to it and run `warlock check` against it, and nothing else. The refusal was always meant to be temporary — brief 29 deferred widening `Opens::open`, deferred choosing between an enum board and `Box<dyn Board>`, and deferred the panel's Linear-shaped push line, all on the grounds that they should be designed against a working second board rather than guessed at.

Leaving it alone costs more than the missing feature. The id-passing protocol in the trait is not portable and cannot be made portable: a Jira status id is reachable only through `GET /issue/{key}/transitions` on one specific issue, and a Jira label has no id at all. Every week the trait stays as it is, more flow code is written against a shape only one vendor can satisfy, and the second board's cost goes up.

## Outcome

A scope recorded against a Jira Cloud site works through the whole lifecycle.

```
$ warlock push payments docs/brief-30.md
warlock: filed `File briefs, tickets and pulls to a Jira board` to `PAY`, labelled `area/pay`: https://acme.atlassian.net/browse/PAY-14
```

Opening `PAY-14` in Jira shows an Epic in To Do carrying `area/pay`, and its description renders: `Outcome` and `Success criteria` are real headings, `**must not**` is bold, the fenced block is a code block with no stray backticks, and the slice list is a list.

```
$ warlock draft payments
1. File briefs, tickets and pulls to a Jira board — PAY-14
...
```

`draft` lists the Epics with the label whose status category is `new`. Cutting a slice files child issues under the Epic — each with `fields.parent` set to `PAY-14`, each in a `new`-category status, blocked by the issues their `depends_on` names — and then moves `PAY-14` out of To Do into an `indeterminate` status. A comment on `PAY-14` records which slice was cut, so a second cut does not offer it again.

```
$ warlock pull payments
warlock: PAY-15 "Convert Markdown to ADF and back" moved to In Progress
...
warlock: PAY-15 moved to In Review
```

When the last child of `PAY-14` is in review or done, `PAY-14` moves to In Review too. A scope whose `review_state` names a status no transition reaches is refused by name:

```
$ warlock pull payments
warlock: `payments` records the review state `Shipped`, which no transition from `In Progress` reaches on this board; the reachable statuses are `To Do`, `In Review`, `Done`
```

The panel says the same things. `p` over a Jira scope notes `filing to PAY` while the worker runs and `filed to PAY: https://...` when it lands.

## Success criteria

**Markdown and ADF convert both ways, and neither direction fails**

- `adf.rs` exposes a Markdown-to-ADF conversion and an ADF-to-Markdown conversion, neither returning a `Result`.
- Headings, paragraphs, bullet lists, ordered lists, fenced code blocks, and the `strong`, `code` and `link` marks round-trip from Markdown to ADF and back to equivalent Markdown.
- A Markdown construct outside that subset — a table, a blockquote, an image — becomes a paragraph holding its literal source text, and the push is not refused.
- An ADF node type the reader does not know — `panel`, `table`, `mediaSingle`, `mention`, `emoji` — contributes its descendant text and nothing else.
- A brief whose body is run through both conversions still parses as a brief: `## Scope` and each `### N.` survive.
- `adf.rs` names no board type and no HTTP type.

**The board seam is in its own module, in warlock's words**

- `board.rs` holds the `Board` trait, `Category`, the shared value types, `Opens`, `Opener`, `Boards` and the error type; `linear.rs` holds `Posts`, the GraphQL client and `Linear`.
- The trait's nouns are `brief` and `ticket`; the board-side types are `NewBrief`, `FetchedBrief` and `FiledBrief`, and no `Board` method names a project or an issue.
- One error enum carries the board's name, so a transport failure against Jira reads `could not reach Jira:` and against Linear reads `could not reach Linear:`.
- No board type is declared in `linear.rs` other than `Posts`, `Client` and `Linear`.

**No verb hands an id to another verb**

- `Board` has no `team_id`, `backlog_status`, `backlog_state`, `workflow_state`, `project_status` or `issue_label_id`.
- Moving a ticket or a brief takes the target status by name and answers whether a status by that name was reached, so the caller still prints the line it prints today when a board has no such column.
- One board-owned pre-flight verb resolves whatever that board needs before a cut writes anything, and answers a value the caller passes back without reading; `cut` still refuses a destination it cannot file into before the first ticket is created.
- `Opens::open` takes the `ScopeRecord` and the key; no `Board` verb takes a team, a project key or a label as an argument.
- `Category` has exactly `New`, `Started` and `Settled`; a status word neither board's list contains maps to `Started`.
- `Boards` is an enum over `Linear` and `Jira` with a forwarding impl, and `Opener::open` chooses the variant from the record's tracker.

**The scope view is board-neutral**

- `LinearScope` is gone; the view replacing it is constructible from any `ScopeRecord` and exposes `name`, `label`, `review_state` and one ticket-key-prefix accessor.
- The branch name in `puller.rs`, the team comparison in `queue.rs` and the confirm dialog in `puller.rs` all read the prefix rather than a team key.
- `pushing.rs` prints `filing to <prefix>` and `filed to <prefix>: <url>`.
- Nothing in the crate refuses a scope for being a Jira scope.

**A Jira key is a pair, stored as one opaque line**

- The Jira client splits the stored value at the first colon and sends the halves as basic auth against the record's `base_url`.
- A stored value with no colon, or with either half empty, is refused in warlock's own words naming the `email:token` format, before any request is sent.
- `warlock key add`'s preamble states that a Jira key is typed `email:token`.
- No key value reaches any line printed, any error raised or any `Debug` output.

**Push files an Epic**

- `warlock push <jira-scope> <path>` creates an Epic in the record's project, carrying the record's label, with the brief's body as ADF, and prints its browse URL.
- The Epic lands in a `new`-category status.
- A second push of a brief with the same name finds the existing Epic and refuses, as the Linear path does.

**Draft and cut file children**

- `warlock draft <jira-scope>` lists the Epics carrying the label whose status category is `new`, read through `/rest/api/3/search/jql` and paged by `nextPageToken`.
- Cutting a slice creates issues with `fields.parent` set to the Epic's key, each carrying the label, each in a `new`-category status.
- A slice's `depends_on` becomes `Blocks` issue links, with the blocking issue as the inward issue.
- Cut notes are written as comments on the Epic and read back on the next cut, so a slice already cut is not offered again.
- After the first cut, the Epic is moved to an `indeterminate` status by transition.

**Pull works a ticket**

- `warlock pull <jira-scope>` reads the queue with `ORDER BY priority DESC, Rank ASC`, filtered to the label and to the account the key belongs to.
- Jira's `Highest`, `High`, `Medium`, `Low` and `Lowest` map to `Urgent`, `High`, `Medium`, `Low` and `None`; an absent or unrecognised priority maps to `None`.
- A ticket is moved to In Progress and then to the record's `review_state`, each by a transition chosen on `to.name` and posted by id.
- A `review_state` no transition reaches is refused naming the statuses that are reachable.
- When every child of an Epic is in review or done, the Epic is moved to the `review_state`.
- A JQL search Jira refuses is reported with Jira's own words and is not retried.

## Constraints

No new dependency. The ADF converter is hand-written against the subset warlock emits. `pulldown-cmark` was weighed and rejected: it helps only the write side, and the read side is the one `cut`'s `## Scope` parse depends on. The root `Cargo.toml` requires a justification comment for every dependency, and this one would not earn it.

Jira Cloud and REST v3 only. Server and Data Center serve no v3 and are not addressed. The v2 decision recorded in brief 29 is overturned by two verified facts: Jira renders v2 text as wiki markup, so `## Scope` arrives as a numbered list item, `**bold**` keeps its literal stars and a fence keeps its literal backticks; and a description stored as ADF — which any edit in the Jira UI produces — reads back through v2 as wiki markup (`h2. Scope`), which breaks `cut`'s parse. An ADF issue created through v3 rendered correctly.

Status is matched by category and never by name, so a board that renames To Do still works. `review_state` is the one exception and is matched by name, because it is the name the person wrote in the record.

No async runtime and no retry. These are `linear.rs`'s existing rules and they apply to `jira.rs` unchanged: a mutation is one request, a failed call is reported and the person decides, a read that pages is one request per page, and a failed page fails the read.

No key value is printed, logged or carried in an error. `Debug` on anything holding a key stays hand-written, as it is for `linear::Client` and `linear::Key`.

The manifest schema from brief 29 does not change. `tracker`, `base_url`, `project_key`, `review_state` and `label` are read as they are recorded; no value is parsed, normalised or judged by this work.

The flows keep their existing refusal ordering: a scope warlock cannot file to is refused before a key is read, a client is built or a brief is opened, and no refusal arrives after a mutation.

## Out of scope

**Jira Server and Data Center.** They serve no REST v3, so supporting them means a second wire format and a second description encoding behind the same verbs. Nobody has asked for it and no site is available to verify it against.

**OAuth, and any auth other than basic with an API token.** The token flow is verified working and needs no browser round trip, no refresh and no client registration. Adding OAuth now is a second credential lifecycle in the key store for no capability warlock lacks.

**Tables, panels and other rich ADF on the write side.** The converter emits six node types. A table in a brief becomes paragraphs of its source text, which is plainer than intended and never wrong; teaching the converter tables means owning ADF's table model, including merged cells, for a construct no warlock-generated brief contains.

**Preserving unknown ADF on read.** A brief heavily restructured in the Jira UI comes back with its headings intact and its decoration gone. Keeping unknown nodes would mean round-tripping a tree warlock does not model, and the only consumer of the read path is a Markdown parse that wants headings.

**Telling a cancelled ticket from a finished one.** `Category::Settled` folds Linear's `canceled` into `completed` and takes Jira's whole `done` category. Nothing downstream asks the question today — `StateType::settled` already answers both the same way — so a four-state enum would exist for a feature nobody has described.

**Migrating anything already on a Linear board to Jira, or a scope that files to both.** A scope records one tracker. There are no users and nothing to migrate.

**Writing Jira custom fields, story points, sprints or components.** `NewTicket` carries what a draft decided and nothing more, which is the rule `linear.rs:1760` already states: a field warlock would have to invent a value for is a decision taken away from the person who owns the board.

**Dropping the order clause when a search is refused.** The endpoints, the `Blocks` link type, `fields.parent`, the transitions and `ORDER BY priority DESC, Rank ASC` are each verified by curl against the KAN site, where `Rank` is `customfield_10019` and is orderable. `Rank` is a Jira Software field, so a site without it may refuse that clause and take the whole queue read down. The refusal stands with Jira's words. If such a site appears, the open question is whether the clause is dropped on a refused search — not whether the refusal is silent, which it is not.

**Whether a partially filed cut resumes cleanly on Jira.** Cut notes on the Epic are the same mechanism as on Linear, so the same resume behaviour should follow, but it is not separately designed here and the first live cut is where it gets exercised.

## Scope

### 1. Convert Markdown to ADF and back

depends_on: []

Nothing in the workspace converts Markdown. `push.rs:188` sends `brief.content()` straight into `NewProject`, because Linear takes Markdown in `content`. Jira v3 takes an Atlassian Document Format tree and nothing else, and `cut.rs` later parses the description it reads back, so the conversion has to work in both directions or a cut can never find `## Scope`.

This slice adds `adf.rs`, holding two conversions and no board type, no HTTP type and no `Result`. The write direction covers the subset warlock emits: `heading`, `paragraph`, `bulletList`, `orderedList`, `codeBlock`, and `text` carrying the `strong`, `code` and `link` marks. The read direction walks an ADF tree back to Markdown.

Both directions are total, and that is the decision this slice exists to make. A Markdown construct outside the subset becomes a paragraph holding its literal source text; an ADF node the reader does not know contributes its descendant text and nothing else. A push must not be refused because somebody put a table in a brief, and a brief a person lightly edited in the Jira UI must still parse back to its headings. Refusing unsupported markup was the alternative and was rejected: it makes the set of documents warlock accepts depend on a converter's completeness, and it strands a brief at exactly the moment somebody wants it filed. The cost is stated plainly — a table in a brief looks like plain lines on the board, and a heavily restructured brief comes back plainer than it went out. Nothing round-trips wrong; it round-trips plainer.

The converter is hand-written for the reason the constraints give, and it lands first because it is pure, testable in memory, and the thing most likely to be wrong when a real brief meets a real site.

A later edit must not make either direction fallible, and must not add a node type to the writer without adding it to the reader: a tree warlock can produce and cannot read back is a brief that files and never cuts.

### 2. Move the board seam into its own module

depends_on: []

`linear.rs` is 2,214 lines holding four unrelated things: the GraphQL transport (`Posts`, `Client`), the `Board` trait at line 155, the shared value types (`Queue`, `QueuedIssue`, `Blocker`, `Priority`, `StateType`, `IssueProject`, `FetchedProject`, `Listing`, `NewProject`, `Project`, `NewIssue`, `Issue`), and `Opens`/`Opener` at line 281. A second board would have to import all of it from a module named after the first.

This slice creates `board.rs` and moves the trait, the shared value types, `Opens`, `Opener` and the error type into it, leaving `Posts`, `Client` and `Linear` in `linear.rs`. The nouns change with the move: `project` becomes `brief` and `issue` becomes `ticket` throughout the trait, and the board-side types become `NewBrief`, `FetchedBrief` and `FiledBrief`. The collision with `brief::Brief` is accepted — the board record and the document are the same thing one layer apart, and naming them alike states that relationship. `epic` was rejected because it is Jira's word for something Linear does not have, and `file_epic` returning a project URL would read as a bug. `plan` and `parent` were rejected as words neither board's reader would recognise, with `parent` also colliding with Jira's literal `fields.parent`.

The error enum gains the board's name as a `&'static str` set by whichever client builds it, so one type's `Display` reads `could not reach Jira:` or `Linear refused the request:`. Two error enums behind a wrapper were rejected: every caller in the crate would then match on the wrapper to say the same five things twice. The cost of the chosen shape is that every error value carries a field that is constant per client, and a third board has to remember to set it.

No behaviour changes here and no verb signature changes, so the existing tests pass untouched. The test fakes and the cancellable decorator at `puller.rs:1025-1097` move with the names they forward.

A later edit must not put a vendor's word back into `board.rs`, and must not let `linear.rs` re-export the shared types: the point of the split is that `jira.rs` never names `linear.rs`.

### 3. Stop handing ids between verbs

depends_on: [2]

Six of the trait's verbs exist to turn a name into a global id another verb consumes: `team_id`, `backlog_status`, `project_status`, `backlog_state`, `workflow_state` and `issue_label_id`. `cut.rs:88-101` runs three of them before its first create; `pulling.rs:565-577` runs two before a move; `push.rs:173` runs one before a create. That protocol is Linear's and no Jira implementation can honour it: a Jira status id is reachable only from `GET /issue/{key}/transitions` on one specific issue, and a Jira label has no id at all. A Jira board implementing today's trait would have to return the status name as its own "id" and resolve for real later, which makes `move_issue(issue, state)` mean a different thing per board and silently drops `cut`'s refuse-before-filing property.

This slice collapses each resolve-and-act pair into a verb taking the name. Moving a ticket or a brief takes the target status by name and answers whether a status by that name was reached, so `pulling.rs` keeps printing the line it prints today when a board has no such column — the `Option<id>` becomes a value the caller reads, not a silent success. The label is passed to the ticket create by name. One board-owned pre-flight verb resolves whatever a board needs before a cut writes anything and answers a value the caller hands straight back without inspecting; that is what preserves `cut.rs`'s property that a destination it cannot file into is refused before the first ticket exists. Two traits with no shared vocabulary were rejected: `cut`, `planned` and `pulling` would each carry two copies of their control flow.

`Opens::open` takes the `ScopeRecord` and the key, so the board captures its own destination — a team key for Linear, `base_url` and `project_key` for Jira — and `team` and `label` leave every verb signature: `scope_queue(team, label, assignee)` becomes a queue read taking the assignee, and `project_named(team, name)` becomes a brief lookup taking the name. With that, `LinearScope` has nothing Linear-specific left to guard and becomes a board-neutral view over any record exposing `name`, `label`, `review_state` and one ticket-key-prefix accessor: `WAR` for Linear, `PAY` for Jira. That one accessor serves the three places outside the board that used a team key — the branch name at `puller.rs:1199`, the team comparison at `queue.rs:435` and the confirm dialog at `puller.rs:1330` — and `pushing.rs:417-425` prints it, which closes the panel push line brief 29 deferred. Printing the scope name there instead was rejected: it drops the fact the line exists to state, which is which board the brief is about to land on. Taking the record at open means it is owned or cloned where a `&str` crosses onto a worker today.

`StateType` is replaced by `Category { New, Started, Settled }`, each board mapping its own words in: Linear's `triage`, `backlog` and `unstarted` to `New`, `started` to `Started`, `completed` and `canceled` to `Settled`; Jira's `new`, `indeterminate` and `done` as they come. A word neither list contains maps to `Started`, never to `New` or `Settled`, which is the conservative corner — an unrecognised status keeps a blocker blocking and keeps a brief out of draft's list, rather than letting work through or offering a brief that is already cut. That rule is what the comment at `linear.rs:1311` is protecting, written into the type instead of into a string.

Everything is still Linear-only at the end of this slice and a Jira scope is still refused. Every call site and every fake is rewritten in this one change, which is affordable because there are no users to keep compatible.

A later edit must not reintroduce a verb whose answer is an id another verb consumes, and must not let a flow read inside the pre-flight value: the moment a caller interprets it, it is holding one board's protocol again.

### 4. File an Epic with `warlock push`

depends_on: [1, 3]

Nothing in the workspace speaks REST or holds a Jira credential. `error.rs:400-407` refuses every Jira scope at `LinearScope::of`, and `Opener` at `linear.rs:293` returns a `Linear<Client>` whatever the record says.

This slice adds `jira.rs`: a `Calls` seam taking a method, a path and a JSON body against the record's `base_url`; a client on the same `ureq` agent configuration and `REQUEST_TIMEOUT` as `linear.rs`, authenticating with basic auth; and a `Jira` board. The stored key is split at the first colon into email and token — an email cannot contain a colon, so the first one is the boundary — and a value with no colon or with either half empty is refused in warlock's own words naming the `email:token` format, before any request is sent. The key store keeps its rule of judging a key by nothing, which `key.rs:183` states deliberately; `key.rs`'s preamble gains one line saying a Jira key is typed that way, because that screen is where somebody decides what to paste and the alternative is the format living only in this document. A `--jira` flag on `key add` was rejected: the store is per-machine, a key can be bound to several scopes, and the flag could disagree with the record. Making `Opens::open` fallible so the split could be refused before a board exists was also rejected — it touches every `open` call site for one string split, and the first-request refusal still names the format before anything goes out.

`Boards` is added as an enum over `Linear` and `Jira` with a forwarding impl, `Opener::open` chooses the variant from the record's tracker, and the refusals in `error.rs` and `puller.rs` come out. An enum rather than `Box<dyn Board>` because the trait after slice 3 is small enough that the forwarding arms are cheap, and nothing then has to be object-safe or boxed per run.

The Jira board implements the push verbs: find a brief by name, and create one as an Epic in the project, carrying the label, with the body converted by slice 1. Every other verb returns a refusal naming what warlock cannot do with a Jira scope yet; slices 5 and 6 delete those arms as they fill them. Each verb is tested against an in-memory `Calls` stand-in asserting the path and the body, the way `linear.rs`'s own tests assert the GraphQL document.

Push lands before cut and pull because this is the first slice where a brief's body goes through ADF for real. The Epic can be opened in Jira and read, which is the one thing a curl probe could not answer, and it answers it three slices before the rest is written.

A later edit must not put the key into a header without the split, and must not let a request go out before the format refusal: a malformed pair sent as auth is a 401 that says nothing about what was wrong.

### 5. List Epics and cut tickets under them

depends_on: [4]

After slice 4 the Jira board refuses `draft`. `planned.rs:297` lists the briefs a draft can read and `cut.rs:88-187` files the tickets, their blocking links and the cut note, then moves the brief on.

This slice implements those verbs for Jira. Listing uses `/rest/api/3/search/jql`, paged by `nextPageToken`, filtered to the label and to Epics whose status category is `new`; `/rest/api/2/search` answers 410 Gone and is not an option. Fetching one Epic reads its description back through slice 1's ADF reader and reads its warlock cut and skip comments, matched by the same `CUT_NOTE` and `SKIP_NOTE` prefixes `linear.rs:765` defines, so a slice already cut is not offered twice. Children are created with `fields.parent` set to the Epic's key and found again with the JQL `parent = PAY-14`. A slice's `depends_on` becomes `Blocks` issue links, with the blocking issue as the inward issue — the direction lives entirely in which key goes in which field, and swapping them compiles, answers the same shape and inverts every dependency in the slice. After the first cut the Epic is moved to an `indeterminate` status by transition.

The pre-flight verb from slice 3 is where Jira earns it. Jira sets a created issue's status from the workflow's initial transition rather than from the create body, so there is no status id to resolve and hand to a create; the pre-flight instead verifies that the project has a `new`-category status and that the Epic is reachable, so a cut that cannot file is refused before the first child exists. A child that lands outside a `new` category is transitioned into one.

A later edit must not change the cut-note prefixes: they are the whole of what records which slices are cut, and changing one orphans every note on every board, which offers those slices again.

### 6. Work a ticket with `warlock pull`

depends_on: [5]

After slice 5 the Jira board refuses `pull`. `pull.rs:350-368` reads the queue, `pulling.rs:565-577` moves the ticket through In Progress and the record's `review_state`, and `pulling.rs:921-947` moves the parent on when its children are done.

This slice implements the remaining verbs. The queue is a `search/jql` read filtered to the label and to the account the key belongs to, read from `/myself`, ordered by `ORDER BY priority DESC, Rank ASC` — verified returning 200 against the KAN site, where `Rank` is `customfield_10019` and orderable — so priority leads and tickets tying on it keep the order people dragged them into on the board. Jira's `Highest`, `High`, `Medium`, `Low` and `Lowest` map to `Urgent`, `High`, `Medium`, `Low` and `None`; an absent priority field, which a team-managed project can have, and an unrecognised name both map to `None`, which is where Linear's own "no priority" already sorts. Blocking links are read unfiltered and to the end, as `linear.rs:976` requires: a blocker list cut off midway makes a held-up ticket look ready.

Moves are transitions — `GET /issue/{key}/transitions`, chosen by the target status name on `to.name`, posted by id. `review_state` is the one thing matched by name rather than by category, because it is the name the person wrote in the record, and a `review_state` no transition reaches is refused naming the statuses that are reachable; a silent no-op would leave a ticket in In Progress with nothing on screen saying why. When every child of an Epic is in review or done, the Epic moves to the `review_state` the same way.

A search Jira refuses is reported with Jira's own words and nothing retries. A retry without the order clause was weighed and rejected: the queue would honour board rank on some runs and not others with nothing to say which, and `Rank` is verified orderable on the site this was built against.

A later edit must not match a status by name where the category would do, and must not let a failed move abandon a run: the board is a courtesy and the work is the point, which is the rule `move_issue` already carries at `linear.rs:1964`.
