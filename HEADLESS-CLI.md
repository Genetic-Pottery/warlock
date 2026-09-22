# The headless CLI

Warlock with no subcommand opens the tree. With one, it does that one thing and
exits: no alternate screen, nothing drawn. What each has to say is lines on
stdout, so a script, a CI job or an agent reads the answer through a pipe
rather than into a repaint.

One subcommand takes the terminal, and only when there is a person at it:
`warlock key add` turns the echo off for the length of the read so a key is not
typed onto a visible screen. Everywhere a script runs — stdin redirected or
piped — it reads a plain line and the terminal is never touched.

| Command | What it does | What it spends |
| --- | --- | --- |
| `warlock init` | Write warlock's section of `CLAUDE.md` at the repository root | nothing |
| `warlock config` | Print the sigils this machine holds here, and read a line replacing them | nothing |
| `warlock key add <name>` | Read a Linear key on stdin and store it under a name | one key-store write |
| `warlock key list` | Print the names this machine holds keys for, and never a key | nothing |
| `warlock key use <name>` | Bind one of the stored names to this checkout | one config write |
| `warlock key forget <name>` | Remove a stored key from this machine by name | one key-store write |
| `warlock stale [path]` | List the pacted directories at or below `path` that are stale | nothing |
| `warlock fresh [path]` | The same for the fresh ones | nothing |
| `warlock check <path>` | Say which scope covers `path`, where work under it is filed, what this machine holds, and whether the two meet | nothing |
| `warlock unpact <path>` | Drop the pact on a directory and every pact below it | one manifest write |
| `warlock scope add <path> <scope>` | Write a scope onto a pacted directory, and — `--team`, `--review-state`, `--label` — the `[[scope]]` record routing it, when nothing records the name yet | one manifest write |
| `warlock scope remove <path>` | Clear the scope on a pacted directory | one manifest write |
| `warlock pact <path>` | Describe a directory and everything below it, a `WARLOCK.md` each | a model pass per directory |
| `warlock refresh <path>` | The same over only the directories that are not fresh | a model pass per stale directory |
| `warlock push <path>` | File the brief at `path` as a project on the board this machine's sigil names | one project on somebody's board and one record write |
| `warlock pull <path>` | Cut the project filed for the brief at `path` into issues on the board that holds it | a model pass per uncut slice, the issues they become, and a record write each |

The two listings take the repository root when the path is left off. Every other
path is required, and on `unpact` and `pact` that is the point rather than an
omission: the largest thing warlock can do to a repository must not also be the
thing an absent argument does by itself. `warlock pact .` is somebody having
said so.

`init` and `config` are the two that are about the checkout rather than the
ledger. `warlock init` writes warlock's section of the `CLAUDE.md` at the
repository root and says which file it wrote, and whether it was created or
updated. `warlock config` prints the repository, the file this machine keeps
sigils in and what is held now, then reads one line: the sigils on it replace
everything held for this repository, a blank line clears it, and Ctrl-C or EOF
changes nothing. It is the only road to a sigil, and a sigil is the only thing
that opens a scope.

### Keys

`warlock key` is the other half of what this machine holds, and it is a
different thing from a sigil: the Linear keys, stored by name in
`~/.warlock/keys.toml`, owner-only on unix. The binding — which of those names
this checkout uses — lives in the per-project `config.toml` under the same
directory, beside the sigils. Neither file is in the repository and neither is
committed; both are the machine's.

`key add` is the only verb that reads a key and no verb prints one. `list`
prints names, `use` writes a name, `forget` removes one, and no key value
reaches a line, an error or a JSON value from any of them. There is no field
for one in the envelope and there is not going to be.

`warlock key add <name>` takes the secret on stdin and never as an argument:
argv is readable by every process on the box for as long as the command runs,
and is in a shell history afterwards. It prints the name, the file the key
lands in and what is stored under that name now, then reads one line.

How that line is read depends on what stdin is. At a terminal the echo is
turned off for the length of the read and the key is shown as bullets, so
typing or pasting it is safe on a screen somebody else can see:

```sh
$ warlock key add acme
key `acme`
stored at `/home/you/.warlock/keys.toml`
no key is stored under this name yet
what you type is not shown
Ctrl-C or EOF changes nothing
key> ••••••••••••••••••••••••••••••••••••••••
warlock: stored a key for `acme` in `/home/you/.warlock/keys.toml`
```

Anything else on stdin — a redirect, a pipe, a CI step — is read as a plain
line in cooked mode, and the preamble says so rather than claiming a hiding it
is not doing:

```sh
$ warlock key add acme < key.txt
key `acme`
stored at `/home/you/.warlock/keys.toml`
no key is stored under this name yet
the line is echoed, so `warlock key add acme < key.txt` is how to keep it off the screen
Ctrl-C or EOF changes nothing
key> warlock: stored a key for `acme` in `/home/you/.warlock/keys.toml`
```

This is the one place in the family that touches the terminal. It takes raw
mode for the read and puts it back on every way out, a panic included.

EOF — Ctrl-D at a terminal, an empty pipe everywhere else — writes nothing and
says so, and a name that is not a name is refused before the preamble is
printed, so nobody pastes a live credential at a prompt that was always going
to refuse the name afterwards. A line that is typed and holds nothing is
refused instead of stored: a checkout binding an empty key would look bound and
fail at the API.

`warlock key list` prints one name a line, nothing else on the line, and never
a key. `--json` is the same names in one object:

```sh
$ warlock key list
acme

$ warlock key list --json
{"command":"key list","names":["acme"]}
```

A machine nobody has run `key add` on holds no keys, which is an empty answer
and a 0, exactly as nothing stale is.

`warlock key use <name>` binds one of those names to this checkout, written
into the per-project `config.toml` with the sigils already there left where they
were. A checkout that has bound none is *unbound* rather than defaulted:
warlock does not reach for whichever key happens to be first in the store, and
`use` is how a checkout stops being unbound. One checkout binds one name, and
the name is the only thing written — the key itself stays in `keys.toml`.

`warlock key forget <name>` removes a key from the machine. When this checkout
was the one bound to it, the removal still happens and the line says so: this
checkout is unbound and will refuse until another `use` binds a key. No other
checkout is read and none is warned, because warlock keeps no list of them to
walk.

`add`, `use` and `forget` take no `--json`, matching `unpact` and the two
`scope` verbs. A name the store has never heard of is refused by `use` before a
byte reaches the config and by `forget` with nothing removed, one line on
stderr and an ordinary **1** — not the 3, which is a scope this machine's
sigils do not open and which no name in a key store has anything to do with.
`use` and `forget` also want a repository, because a binding belongs to a
checkout; `add` and `list` want none and answer for the same store from
anywhere.

### Asking

`stale`, `fresh` and `check` only read. None of them writes to
`.warlock/pacts.toml`, none spawns a process and none runs a model pass, so
asking costs no tokens, no minutes and no risk of a manifest left in a state
nobody asked for — which is what makes them safe to put in a CI job or an
agent's hands. What they read is what the tree itself reads: the same walk, the
same staleness rule, the same coverage, never a second opinion written on the
shell side.

The listings print one path a line and nothing else on the line, relative to the
repository root and spelled the way the manifest spells them:

```sh
$ warlock stale
.
crates
crates/warlock-engine
crates/warlock-engine/src

# nothing under the engine is behind its code
$ test -z "$(warlock stale crates/warlock-engine)"
```

`check` walks up from one path and answers in five lines — the scope covering
it, where work under that scope is filed, what this machine holds, whether the
scope and the sigils meet, and which stored key name this checkout is bound to:

```sh
$ warlock check crates/engine
`crates/engine` is scoped `data-plane`
work here is filed to `Data Plane`, as `In Review`, labelled `area/data-plane`
holding `data-plane`
`data-plane` is open to this machine
filing to `Data Plane` would use the key `work`, which this machine stores
```

The first two lines are the repository's and true for anyone who clones it; the
last three are this machine's. All five print every time, so an answer with
something missing is a line that says what is missing rather than one fewer
line to count: a scope with no `[[scope]]` record names the scope and
`.warlock/pacts.toml`, a path no scope covers says there is nothing to route to
and that a scope would fix it, and an unbound checkout is sent to
`warlock key use` and `warlock key add`. A closed scope keeps its whole route,
with the closed line beside it rather than in place of it — somebody covering
for a colleague is told both what they would be crossing and where the work
files. Only key *names* are ever printed, here and everywhere else.

All three take `--json` and answer as one object on one line instead:

```sh
$ warlock stale --json
{"command":"stale","directories":[{"path":".","state":"stale"}]}

$ warlock check crates/engine --json
{"command":"check","path":"crates/engine","scope":"data-plane","sigils":["data-plane"],"opens":true,"team":"Data Plane","review_state":"In Review","label":"area/data-plane","key":"work","key_found":true}
```

`path` is repository-root-relative, `scope` is the covering scope or `null`,
`sigils` is what this machine holds (`null`, and never `[]`, when the config
would not read), and `opens` is whether the two meet. The five beside them are
the route and the key, flat in the same object rather than nested, so nothing
has to know which half of the answer a field was added with:

| Field | What it says | When it is `null` |
| --- | --- | --- |
| `team` | The team slug in the covering scope's `[[scope]]` record | No scope covers the path, or the covering scope has no record |
| `review_state` | The review state that record files work in — spelled as the record spells it, never `state` | The same two cases |
| `label` | The label that record puts on work | The same two cases |
| `key` | The name this checkout's key is bound to, never a key | Nothing is bound to this checkout |
| `key_found` | Whether that name resolves in `~/.warlock/keys.toml` — a `bool`, never `null` | — |

The three record fields are one record spread flat, so they are `null`
together and never `""`: an empty string would tell a script it was filed to a
team whose name is the empty string. `key` and `key_found` tell the two ways of
having no usable key apart, because they are fixed in different places:
`key: null, key_found: false` is nothing bound and `warlock key use` is the
fix, while `key: "work", key_found: false` is a name this machine has never
stored and `warlock key add work` is.

The verdict is a field and never a status. A closed scope is the answer to the
question rather than a failure to reach one, so `check` exits 0 either way, and
so do an unbound checkout, a bound name the store has never heard of and a path
no scope covers. That is what leaves the exit status free:
`warlock check <path> --json | jq -e '.opens'` spends `jq`'s status on the
verdict, and `warlock check <path> --json | jq -e '.opens and .key_found'`
spends it on "this machine may work here and can file the ticket" — warlock
spends none of its own on saying no either way. The same goes for an empty
listing — nothing stale is an answer, and it is a 0.

## Writing

`unpact`, `scope add` and `scope remove` are `.warlock/pacts.toml` rewritten and
nothing else. No terminal, no process, no model pass, and every `WARLOCK.md`
left exactly where it was — un-pacting drops the record, not the documents.

All three ask the boundary first, before they look at whether the path has an
entry at all, so a command aimed inside a scope this machine does not hold
answers with the refusal and never with what the manifest holds. A refusal is
one line on stderr and **exit 3**, with the file byte-identical to what was
read. There is no `--force` and no environment variable past it:
`warlock config` is the one road, here exactly as it is in the panel.

An un-pact has a second refusal that is not that one. When the boundary over the
path itself is open but something *below* it carries a scope this machine does
not hold, what is being refused is the blast radius rather than the place — so
it is an ordinary **1**, and the sentence offers the road that needs no sigil:
un-pact the parts you hold.

`scope add` has two refusals of its own beside the boundary's, both of them
about the `[[scope]]` record rather than the place. `--team`, `--review-state`
and `--label` are that record's three values, and whether they are required is a
fact about the manifest rather than about the command line: a scope name nothing
records yet is written with all three or not at all, and a name that already has
a record takes none of them. Both refusals are an ordinary **1** and both leave
the file byte-identical — not the 3, which is the sigil boundary's alone and
would send a script to `warlock config` over a missing `--team`.

A new name with a record is one manifest write and not two. The scope on the
pact and the `[[scope]]` record are built together and saved once, so no run
leaves a name on a pact with nothing to route it, and there is nothing to undo
when the second half is the half that fails. The refusal names every flag that
was left out, so the command is retyped once rather than three times:

```sh
$ warlock scope add src platform --team 'Platform'
warlock: nothing records `platform` yet, so nothing was written: writing a scope by that name needs a team, a review state and a label, given as `--review-state` and `--label`
```

The same refusal, worded for what actually happened, when a flag was given a
value that holds nothing: a flag nobody passed and a flag passed `''` are
different mistakes, and one sentence for both would send somebody looking for a
shell problem they do not have.

```sh
$ warlock scope add src platform --team ' ' --review-state 'In Review' --label ''
warlock: `--team` and `--label` cannot be blank, so nothing was written
```

Not blank is the whole rule. A team, a review state and a label belong to
somebody else's tracker, so warlock judges blankness on a trimmed copy and
stores the string exactly as it was typed — nothing is trimmed, folded or
checked against a list of review states warlock does not have. Only the scope
*name* is lower-cased and judged, by the same rule the panel's `s` key uses, and
the record is filed under that folded name.

The other way round, a name the file already records is the flagless run and
only that:

```sh
$ warlock scope add src data-plane
warlock: src is scoped `data-plane`

$ warlock scope add src data-plane --team 'Data Plane'
warlock: `data-plane` already has a record in `.warlock/pacts.toml`, and warlock does not rewrite one: run without `--team`, `--review-state` and `--label` to write the scope, or edit the file to change the record
```

A value handed to a name that already routes would be a value dropped on the
floor, which is why it is refused rather than ignored. Warlock does not rewrite,
merge or delete a record from here at all: the file is the road to changing one.
A scope name already in use with no record stays legal and stays unrouted —
`scope add` offers nothing about the names it did not just create.

## Running

`pact` and `refresh` are the two subcommands that spend anything: minutes, one
`claude --print` per directory, a `WARLOCK.md` written beside each of them, and
one manifest save at the end. They pass the same gate the cheap writes do, asked
before a single directory is walked — a boundary asked any later would be asked
after somebody's tokens were spent and somebody else's prose overwritten, and no
exit status puts that back.

Which directories a `refresh` describes is the engine's judgement, the same one
the `r` key gets: the ones that are not fresh, no wider and no narrower.

Progress is two lines a directory on stdout, and the denominator does not move
for the length of the run:

```
warlock: [3/6] documenting crates/warlock-engine/src
warlock: documented crates/warlock-engine/src
```

A directory whose pass failed never gets its second line. What it gets instead
is stderr, where every failing directory is named, one line each, and then
counted:

```
warlock: crates/warlock-tui/src — nothing was written for `/repo/crates/warlock-tui/src`: the model pass produced no answer: …
warlock: 1 of 6 directories failed — the manifest holds what the rest earned
```

That run exits **4**, and the manifest is saved either way: the grants the rest
of the subtree earned are on disk, so the thing to do about a 4 is re-run over
what failed rather than buy the whole descent again. The split between the two
streams is what makes it readable — `warlock pact . > run.log` puts the descent
in the file and leaves what went wrong on the terminal.

Ctrl-C is the only key a headless run has, and it is the panel's two answers in
order. The first press is Esc: the `claude` in flight is killed, so the stop
takes milliseconds rather than the rest of a five-minute pass, the descent ends
at the next directory rather than part way through one, and everything that
finished is hashed, granted and saved before the process leaves with **130**.
The second press is `q`: it exits at once, saving nothing and printing nothing.
Nothing is corrupted by taking it — every document and the manifest are written
beside and renamed over, so what is on disk is always a whole file.

## Pushing

`warlock push <PATH>` is the only subcommand that opens a socket of its own,
and the only one that leaves something behind in a workspace that is not this
repository. The brief at that path becomes a project on the board this
machine's sigil names — the team, the label and the scope are the `[[scope]]`
record `warlock check` prints — and the address of that project is appended to
`.warlock/filed.toml`, which is how the repository knows a brief is filed.

Two flags and no more. `--scope <NAME>` picks the board when this machine can
file to several, and `--dry-run` says what would be sent without opening a
socket. There is no `--json`, matching `unpact` and the two `scope` verbs: the
answer worth parsing here is the record, and that is a file in the repository
rather than a stream to catch.

The order of the work is the promise rather than an arrangement. The
repository, the home the sigils and the key store sit under, the board, the
`.warlock/filed.toml` records and the document itself are all resolved first,
and only then is the socket opened — so every refusal below costs nothing,
reaches nobody's workspace and leaves no half-filed brief behind. Past that
line the order is team key to team id, the `Backlog` project status (a
workspace with no status by that name files the project with none rather than
failing), the label, the project, and last the record. The label is resolved —
and created when the workspace has not got one — *before* the project, because
the label is the only mark on a project saying warlock filed it and nothing
here can take a project back.

`--dry-run` prints the board, the key name, the project name and the size of
what would be sent, and the size is the content's own bytes rather than the
file's: the title line is sent as the project's name, not as part of its body.

```sh
$ warlock push docs/warlock-brief-22-push-a-brief-to-the-board-the-sigil-names.md --dry-run
warlock: would file `Push a brief to the board the sigil names` to `WAR`, under the scope `warlock-team`, with the key `warlock` — 15 KB of content, and nothing was sent
```

A push that lands says the same four things it is worth knowing afterwards —
the name it filed, the team, the label and the URL — on one line, and exits 0:

```sh
$ warlock push docs/warlock-brief-22-push-a-brief-to-the-board-the-sigil-names.md
warlock: filed `Push a brief to the board the sigil names` to `WAR`, labelled `warlock`: https://linear.app/acme/project/push-a-brief-to-the-board-the-sigil-names-8f2c1d
```

A brief is filed once. A path already in `.warlock/filed.toml` is refused
before anything is sent, and the refusal leads with the address of the project
it made the first time, because that is what the reader wants from the line:

```sh
$ warlock push docs/brief.md
warlock: `docs/brief.md` is already filed at https://linear.app/acme/project/a-brief-8f2c1d, so nothing was sent: a brief that changed after it was filed is edited where it is
```

Which board is the question with the most ways of having no single answer, and
each of them is its own sentence because each is fixed in a different file by a
different person. A brief is not about a directory, so there is nothing to walk
up and no nearest scope to win: the sigils are the whole statement of which
board this is, and anything other than exactly one candidate refuses rather
than guesses. Nothing held, something held this repository has never heard of,
and a scope held with no `[[scope]]` record are the three ways of having none:

```sh
warlock: this machine holds no sigil, so nothing says which board to file to: hold one with `warlock config`
warlock: this machine holds `platform`, and `/repo/.warlock/pacts.toml` records no scope of any of those names
warlock: this machine holds `platform`, and that scope has no `[[scope]]` record: add one to `/repo/.warlock/pacts.toml`
```

More than one candidate names them all and asks for the flag; a `--scope` that
is not one of them names them all as well, so no answer is lost by giving a
name that was never going to work:

```sh
$ warlock push docs/brief.md
warlock: this machine can file to `data-plane`, `web`: pick one with `--scope <name>`

$ warlock push docs/brief.md --scope billing
warlock: `billing` is not a scope this machine can file to here: `data-plane`, `web`
```

Both are decided before the key store is touched, so a checkout that is both
ambiguous and unbound is told about the board first: `--scope` is the half that
is about this push. The key half is the two refusals the engine already words
for a route, carried here rather than rewritten — an unbound checkout, and a
bound name this machine has never stored:

```sh
warlock: no key is bound to this checkout in `/home/you/.warlock/repo-f447b89a747182e2/config.toml`: bind one with `warlock key use <name>`
warlock: this checkout is bound to `work`, which is not in `/home/you/.warlock/keys.toml`: store it with `warlock key add work`
```

The document is the last thing read before the socket. A file that is not
there, a file with no `# ` title line to take a project name from, and a file
missing sections of the shape this repository's briefs take are three refusals
with nothing sent:

```sh
warlock: there is no file at `/repo/docs/nope.md`, so there is nothing to push
warlock: `/repo/docs/plain.md` has no `# ` title line, so there is no project name to push
warlock: `/repo/docs/thin.md` is missing ## Outcome, ## Success criteria, ## Constraints, ## Out of scope and ## Scope, so nothing was pushed
```

Past the socket, what Linear says is carried through as the line it said it
in — `could not reach Linear: …`, `Linear answered 429` and
`Linear refused the request: …` — one request per operation, with nothing
retrying behind it. Two refusals out there are warlock's own. The first is a
`team` Linear does not know, which names the value and the file it is written
in, because a `team` in a `[[scope]]` record is a team *key* — `WAR` — and the
usual cause of this is a record carrying a team's name instead:

```sh
warlock: Linear knows no team with the key `Data Plane`, so nothing was filed: the `[[scope]]` record in `/repo/.warlock/pacts.toml` is where that key is written
```

The second is the one failure in the whole family that happens after something
was spent: the project exists and the record of it would not save. The URL is
printed before the record is written, precisely so that it survives this — it
goes to stdout as a push that worked, and the failure goes to stderr under it,
non-zero:

```sh
$ warlock push docs/brief.md
warlock: filed `A brief` to `DAT`, labelled `area/data-plane`: https://linear.app/acme/project/a-brief-8f2c1d
warlock: the project is at https://linear.app/acme/project/a-brief-8f2c1d, and warlock could not record it: could not read or write `/repo/.warlock/filed.toml`: Permission denied (os error 13)
```

A label is not a third. Because it is resolved before the create, there is no
run in which a project lands and its label then fails — a label that will not
resolve is refused with nothing created, no URL and no record, which is the
honest report of what happened rather than a project nothing will ever find.

Every one of these is an ordinary **1**. The boundary's **3** is never spent by
a push and could not be: a sigil here picks which board to file to rather than
opening a directory to be written, so there is no path being acted on for the
boundary to refuse. A script reading a 3 from warlock is reading it from
something else.

No key value is printed by any of this — not on a line, not in an error, not in
`--dry-run`, not in a `Debug`. Only key *names* are, here as everywhere else:
the `warlock` in the dry-run line above is the name this checkout is bound to
and never what is stored under it, and the value is read on exactly one line —
the one that builds the client.

## Pulling

`warlock pull <PATH>` is the other half of that one. The project a push
recorded for the brief at `path` is read back, its `## Scope` section is parsed
into slices, the slices `.warlock/filed.toml` already holds a cut record for
are skipped, and every other one is drafted by a session of its own and filed
as issues on the same board. The project's status is not moved in either
direction, here or anywhere: a pull creates issues, writes the relations
between them and says one comment, and nothing else.

Two flags and no more, spelled as the push's are. `--scope <NAME>` picks the
board when this machine can file to several, and the three no-board refusals,
the ambiguous one and an unknown `--scope` are push's word for word — a brief
is not about a directory there and it is not about one here. `--dry-run` says
what would be cut without drafting anything. There is no `--json`, for the
push's reason: the answer worth parsing is the record, and that is a file in
the repository.

The order of the work is the promise rather than an arrangement. The
repository, the home, the board, the key and the brief's own record in
`.warlock/filed.toml` are resolved first, so every refusal in that stretch
costs nothing and sends nothing. Then one read fetches the project — the only
request a dry run makes — and the status gate, the scope parse and the skips
are all decided off what that answer carried, with no second request behind
them. Past that, each remaining slice in turn: one session drafts it, the team
key becomes an id, the team's `Backlog` state and the label are resolved, the
issues are created, the relations between them are written, and that slice's
cut record is saved before the next slice begins. Saving per slice rather than
once at the end is what stops a run that fails halfway from filing its first
slices a second time. After the last slice, and only when something was filed,
one comment on the project names the issues this run made and says the status
was not moved.

Progress is one line per slice as it is drafted and one naming what it filed.
The fraction is the place in the cut order and the position is where the slice
sits in the document, so a reader can find it in the brief — the two differ
exactly when a `depends_on` line moved something:

```sh
$ warlock pull docs/warlock-brief-23-cut-a-planned-project-into-tickets.md
warlock: [1/3] slice 1 `The project fetch` — already cut as `WAR-121`, `WAR-122`, so nothing was sent
warlock: [2/3] slice 2 `The scope parser` — drafting
warlock: cut `The scope parser` into `WAR-123`, `WAR-124`
warlock: [3/3] slice 3 `The drafting session` — drafting
warlock: cut `The drafting session` into `WAR-125`
```

`--dry-run` prints the project, the status it is in, the board, how many slices
there are and how many are already cut, then the slices themselves in the order
they would be cut in — and stops. No session is opened, no model pass is
bought, nothing is sent past the read that fetched the project and no record is
written:

```sh
$ warlock pull docs/warlock-brief-23-cut-a-planned-project-into-tickets.md --dry-run
warlock: would cut `Cut a planned project into tickets`, which is `Planned`, into `WAR` under the scope `warlock-team` — 3 slices, 1 already cut, and nothing was drafted
warlock: [1/3] slice 1 `The project fetch` — already cut as `WAR-121`, `WAR-122`
warlock: [2/3] slice 2 `The scope parser`
warlock: [3/3] slice 3 `The drafting session`
```

There is nobody at a shell to put a question to, so each session is told up
front that it cannot ask one and is held to no rounds at all. A slice that
comes back with something other than drafts is a reported line and the next
slice rather than the end of the run — the slices left are other work, and they
were ordered so that nothing is filed before what it waits on. The same goes
for a relation Linear turned down and for the project's comment: the issues
exist and are recorded by then, and an edge that is missing is something a
person can fix on the board only if they are told it is missing.

The brief and its project are the first three refusals, all of them before the
scope is read. A path `.warlock/filed.toml` does not record has no project to
read and is sent to `warlock push`; an id the workspace does not know names the
id and the file it is written in, because that file is the only place this
machine keeps it; and a project that is not `Planned` names both statuses and
stops where the answer that said so arrived:

```sh
warlock: nothing in `.warlock/filed.toml` records `docs/brief.md`, so there is no project to read: `warlock push docs/brief.md` files it
warlock: Linear knows no project with the id `9f1c0a7e-1f2b-4c3d-8e5a-6b7c8d9e0f10`, so nothing was read: the record in `/repo/.warlock/filed.toml` is where that id is written
warlock: the project filed for `docs/brief.md` is in `Backlog` rather than `Planned`, so nothing was read: warlock reads a project back once it is planned
```

A project with no status at all gets that same sentence with `has no status` in
place of the status it is in, because a workspace that has not got the status
warlock files into leaves one behind — and no status is not `Planned` either.

The scope block is three more, and they are the parser's own sentences rather
than warlock's: a project with no `## Scope` heading, a heading with no
`### ` slices under it, and slices that wait on each other. The last names every
slice left without an order, by position and heading, because which of the two
edges the author meant is a question only the author can answer:

```sh
warlock: this project has no `## Scope` heading, so there is nothing to cut into slices
warlock: this project's `## Scope` section has no `### ` slice headings, so there is nothing to cut
warlock: these slices wait on each other, so there is no order to cut them in: slice 2 `The scope parser` and slice 3 `The drafting session`
```

A project every cut record already covers is refused before a single session is
opened, rather than being a run that succeeded quietly with nothing to do. A
slice is cut once, and the issues it became are where that work goes on:

```sh
warlock: every slice of the project filed for `docs/brief.md` is already cut, so there is nothing to draft: `.warlock/filed.toml` holds a record for each of them, and warlock cuts a slice once
```

Past the socket, what Linear says is carried through as the line it said it in,
exactly as a push carries it, one request per operation with nothing retrying
behind it. One refusal out there is warlock's own and is not a push's: a team
whose workflow has no state called `Backlog`. It is asked before the label and
before every create, so it costs no issue — the slice is still nothing rather
than half filed — and it ends the run instead of moving to the next slice,
because the next slice would only file into the same wall:

```sh
warlock: the team `WAR` has no workflow state called `Backlog`, so no issue was created: a cut slice is filed into that state, and the team's workflow in Linear is where it is named
```

A team key Linear does not know is the push's refusal, worded there and named
against `.warlock/pacts.toml` here too. And the record that would not save is
the push's last failure one layer down: the issues exist, the line that named
them has already gone to stdout, and the failure goes to stderr under it with
the identifiers in it, because that is the last place anything names them:

```sh
warlock: cut `The scope parser` into `WAR-123`, `WAR-124`
warlock: the issues `WAR-123`, `WAR-124` were created, and warlock could not record them: could not read or write `/repo/.warlock/filed.toml`: Permission denied (os error 13)
```

Every one of these is an ordinary **1** — the unrecorded path, the unknown
project id, the wrong status, all three scope-block refusals, the cycle among
them, nothing left to cut, the team with no `Backlog`, and whatever Linear
turned down. The boundary's **3** is never spent by a pull and could not be,
for the push's reason: a sigil here picks which board the project is on rather
than opening a directory to be written, so there is no path being acted on for
the boundary to refuse.

No key value is printed by any of this either, not in the progress, not in a
refusal and not in the dry run. The value is read on exactly one line — the one
that builds the client — and only key *names* ever reach a line.

## Exit statuses

| Status | What it means |
| --- | --- |
| `0` | Completed. The question was answered or the write happened, whatever the answer turned out to be — an empty listing and a scope closed to this machine included |
| `1` | Warlock could not do it, or would not: the repository will not resolve, the manifest will not parse or will not save, the path has no repository-relative spelling, a scope name nothing records yet was given without all three record flags or with a blank one, a name that already has a record was given any of them, a push has no board or more than one, the brief is not one or is already filed, a pull's brief is not recorded in `.warlock/filed.toml`, its project is one Linear does not know or is not `Planned`, the scope block will not cut or has nothing left to cut, the team has no `Backlog` state, or Linear refused what was sent. The line on stderr is the thing to go and read |
| `2` | The command line was never a request. Clap's status and its wording, for a word warlock has no place for |
| `3` | Refused, with nothing spent: this machine's sigils do not open the scope covering the path. No byte moved, retrying changes nothing, and the road out is `warlock config` |
| `4` | Completed with failures: a run wrote the documents it could and saved the manifest, and the lines above the count name the directories that did not come out of it |
| `130` | Cancelled: somebody pressed Ctrl-C during a run, and what had finished by then is saved and granted. 128 plus SIGINT, so a shell, `make` and CI read it as interrupted without being told anything about warlock |

The three that are not 1 are not 1 because they want different things done about
them. A 3 says this checkout is outside that boundary, so stop and go and get
the sigil. A 4 says the work is partly on disk, so re-run over the part that is
not. A 130 says somebody decided to stop it, so nothing should retry it at all.
Telling those apart by their wording would be telling them apart by parsing
prose.