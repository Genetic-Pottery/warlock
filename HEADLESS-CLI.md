# The headless CLI

Warlock with no subcommand opens the tree. With one, it does that one thing and
exits: no alternate screen, nothing drawn. What each has to say is lines on
stdout, so a script, a CI job or an agent reads the answer through a pipe
rather than into a repaint.

One subcommand takes the terminal for its read, and only when there is a person
at it: `warlock key add` turns the echo off for the length of the read so a key
is not typed onto a visible screen. Every other prompt reads a cooked line. A
prompt that follows a model's work — `draft`'s questions and reviews, `brief`'s
next turn — first throws away at a terminal whatever was typed while the model
worked, so a word typed early is never taken as the answer to a question it was
typed before. Everywhere a script runs — stdin redirected or piped — it reads a
plain line and the terminal is never touched.

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
| `warlock check <path>` | Say which scope covers `path`, where work under it is filed, what this machine holds, and whether the two meet — or, with `--gate`, refuse a closed scope instead of describing it | nothing |
| `warlock unpact <path>` | Drop the pact on a directory and every pact below it | one manifest write |
| `warlock scope add <path> <scope>` | Write a scope onto a pacted directory, and — `--team`, `--review-state`, `--label` — the `[[scope]]` record routing it, when nothing records the name yet | one manifest write |
| `warlock scope remove <path>` | Clear the scope on a pacted directory | one manifest write |
| `warlock pact <path>` | Describe a directory and everything below it, a `WARLOCK.md` each | a model pass per directory |
| `warlock refresh <path>` | The same over only the directories that are not fresh | a model pass per stale directory |
| `warlock brief` | Argue a brief in one conversation at the shell, and write it where `briefs.toml` says | a turn per blank line, one more for `/write`, and the document it writes |
| `warlock push <path>` | File the brief at `path` as a project on the board this machine's sigil names | one project on somebody's board and one record write |
| `warlock draft <path>` | Cut the project filed for the brief at `path` into issues on the board that holds it | a drafting session per uncut slice with a turn for each answer or piece of feedback, a proposing pass per question, and for each accepted slice its issues and a record write |
| `warlock pull <SCOPE>` | Work the next ready ticket in that scope's queue to an open pull request | a splitting pass, a model pass per sub-task, a commit each, a pushed branch, a pull request, and two moves on somebody's board |
| `warlock resume <TICKET>` | Put the sub-tasks a halted run stopped on back to `pending`, so the next pull of that ticket finds work | one run-record write |

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
work here is filed to `Data Plane`, labelled `area/data-plane`, and a finished pull moves the ticket to `In Review`
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
question rather than a failure to reach one, so `check` without `--gate` exits 0
either way, and so do an unbound checkout, a bound name the store has never
heard of and a path no scope covers. That is what leaves the exit status free:
`warlock check <path> --json | jq -e '.opens'` spends `jq`'s status on the
verdict, and `warlock check <path> --json | jq -e '.opens and .key_found'`
spends it on "this machine may work here and can file the ticket" — warlock
spends none of its own on saying no either way. The same goes for an empty
listing — nothing stale is an answer, and it is a 0.

`warlock check --gate` is the one exception, and the paragraph above is the rule
it is the exception to. A shell about to write a file, and a `PreToolUse` hook
about to let Claude Code write one, cannot read five lines of prose and cannot
be stopped by a field, so `--gate` asks exactly the same question and answers it
by refusing. It prints no envelope and takes no `--json` — `--gate --json` is
refused at the command line rather than quietly ignored, because an empty stdout
piped into `jq` costs a debugging session. `check` without `--gate` is unchanged
in every respect: the same five lines, the same fields in the same order, the
same 0.

Given a path, a gate is one line on stderr and **exit 3** when the scope
covering that path does not open to this machine's sigils, and nothing at all
and **exit 0** when it does:

```sh
$ warlock check --gate crates/engine/src/lib.rs
warlock: crates/engine/src/lib.rs is scoped `data-plane` — hold that sigil to work here, with `warlock config`
$ echo $?
3

# the same path, holding `data-plane`: nothing to say and nothing in the way
$ warlock check --gate crates/engine/src/lib.rs && echo may write
may write
```

The 3 and the line are the boundary's own, the ones `unpact` and `scope add`
refuse with, so nothing was invented for the gate. A path no scope covers exits
0, and so does a path in a directory nothing has pacted: an unscoped path is
open to anyone, exactly as the verdict line above says it is.

With no path at all, `--gate` is the hook form. It reads a Claude Code
`PreToolUse` payload on stdin and takes the path from `tool_input.file_path`,
and it answers on stdout rather than in a status — a closed scope is exactly one
deny object on one line, in Claude Code's vocabulary rather than warlock's
envelope:

```sh
$ warlock check --gate < payload.json
{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"warlock: `crates/engine/src/lib.rs` is scoped `data-plane` — hold that sigil to work here, with `warlock config`"}}
$ echo $?
0
```

The reason is the boundary's one sentence with `warlock: ` in front of it, so it
names the path, the covering scope and — a sigil and the scope it opens share a
name — the sigil the write wants. The prefix is there because the line Claude
Code shows says only that permission was denied, and warlock is the program to
go and argue with.

An open scope, a path no scope covers, a payload that will not parse and a
payload with no `tool_input.file_path` in it all write nothing at all: a hook
cannot refuse a write it cannot name. Every one of them exits **0**, and so does
the refusal. That asymmetry with the path form is the point rather than an
oversight: **2** is the only status Claude Code honours from a hook, and it
means "block this tool call" for every event rather than "the scope is closed",
so a hook refusing with the boundary's 3 would be a write waved through. The
refusal travels in the JSON and the status stays 0 either way, which is what
leaves one flag serving both a shell that reads statuses and a hook that reads
objects, off one verdict, rather than two flags that can come to disagree about
a path.

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

## Briefing

`warlock brief` is step zero of brief → push → draft → pull, and it takes no
argument and no flag. There is nothing for either to name: what the
document is about is what the conversation decides, and where it goes is
`.warlock/briefs.toml`'s. There is no `--json` — `push`, `draft`, `pull` and
`resume` each decline one, and what a script reads after a brief is the
document it wrote — and no way out of brief mode, because this command *is*
brief mode.

Both of the repository's brief files are read before a word is sent:
`.warlock/brief-template.md`, for the shape the model is asked for and the
document is held to, and `.warlock/briefs.toml`, for the directory a written
brief lands in. Neither is required — a repository that has written neither
gets warlock's own shape and `docs` — but a file that is there and will not
read is a refusal with nothing spent, rather than a conversation opened and
then found to have no shape to converge on. The template is read first, so a
repository with both files broken is one refusal and not warlock's reading
order read off a screen.

One agent and one session for the whole run. The register is said once, and
then the instruction paragraph goes out as its own first turn — the panel's
own, built from the shape that was just read — so the conversation opens on the
model's question rather than on a bare cursor:

```sh
$ warlock brief
warlock: brief mode — this conversation is converging on a document
warlock: What is the change?
> 
```

Typed lines accumulate at the `> ` cursor and a blank line sends them as one
turn. A brief is argued in paragraphs and a terminal has no other way to say "I
have not finished typing", so the cost is written down rather than hidden: a
turn cannot itself contain a blank line. The end of a line is trimmed and the
start of one is not, so an indented list reaches the model indented.

```sh
> the headless CLI cannot be spoken to
>   both are one mechanism
> 
warlock: Two changes, one mechanism.
```

Enter on a line with nothing typed above it sends nothing at all — an empty
turn is the model asked to answer silence, and it would cost somebody money to
be told so. A turn that failed is a line and the cursor again rather than the
end of the run: what reaches it is a missing binary, a timeout and a cancel,
the session id has not moved, and ending an argument twenty turns old over any
of the three would throw away the one thing the command exists to produce.

`/write` is the only command at the prompt and it takes nothing after it. Every
other line is the brief, and a command word that is not it is refused on a line
rather than sent, so a mistyped command costs a line here instead of a turn:

```sh
> /push docs/brief.md
warlock: /write is the only command here and takes nothing after it — every other line is the brief; `warlock push` and `warlock draft` are commands of their own
```

It is the panel's own parser that reads the line, so `/write ` with the space a
hand leaves behind is the command, `/WRITE` is not, and a line opening with a
path — `/tmp/notes is where I keep them` — is prose. Lines typed and not yet
sent are left exactly where they are: a `/write` is about the conversation that
has happened, and lines nobody has sent are not part of it.

A `/write` asks the conversation for the document, prints the reply as every
other reply is printed, and then proposes a path for it: the directory
`briefs.toml` names, the next number among the names already in it, and a slug
off the document's first `# ` line. Enter accepts the proposal and any other
text replaces it entirely — nothing weighs the two against each other. The file
is what the conversation was for, so the prompt does not come back after it:
the run ends on the line that named the path, and that is a **0**.

```sh
> /write
warlock: # Give the headless CLI a voice
…
warlock: the document goes to `docs/warlock-brief-01-give-the-headless-cli-a-voice.md` — Enter writes it there, another path replaces it
> 
warlock: wrote docs/warlock-brief-01-give-the-headless-cli-a-voice.md — 3.4 KB
```

Two refusals at that prompt, and they are answered differently because they are
fixed in different places. A path that already has a file is the rule and then
the same offer again, because a path is the one mistake a reader fixes by
typing; a document missing a section of the shape is not fixable by typing
another path, so it goes back to the conversation and the next turn is where it
gets fixed. Nothing is written by either, and a missing section is never
repaired into the document — a brief with a section missing reads perfectly
well and nobody finds out for days:

```sh
warlock: docs/taken.md already exists — nothing was written
warlock: the document is missing ## Success criteria, ## Constraints and ## Scope, so nothing was written
```

EOF — Ctrl-D at a terminal, an exhausted pipe everywhere else — ends the run
with nothing written, at the conversation's cursor and at the path prompt
alike. Nothing is kept either: the session id this process never wrote down is
a conversation that ends when the process does.

```sh
> 
warlock: the conversation is over
```

Ctrl-C needs no code at all here, because nothing in this command holds the
terminal: the prompt is a cooked line off stdin, with no alternate screen and no
panic hook, which is what lets a whole conversation be driven from a script with
no terminal anywhere. At a terminal, lines typed while the model was answering
are thrown away before the cursor comes back, so they are not sent as the next
turn.

The statuses are two. A write prints the path it wrote and the size and exits
**0**, and so does a run that ended at an EOF with nothing written — nobody
asked for a file. The two file refusals are an ordinary **1** with nothing
sent, the conversation never opened and the register never said:

```sh
warlock: could not read `/repo/.warlock/brief-template.md`: Permission denied (os error 13)
warlock: malformed brief config at `/repo/.warlock/briefs.toml`: TOML parse error at line 1, column 6 …
```

The boundary's **3** is never spent by a brief and could not be: nothing in the
command asks a scope or holds a sigil up against one, so there is nothing here
for the boundary to refuse. A script reading a 3 from warlock is reading it
from something else.

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
than guesses. A repository that records no `[[scope]]` at all is the first of
the four ways of having none, and it is answered before what this machine holds
is read, because no sigil would make a board here and a sentence about holding
one would be advice that cannot work; then nothing held, something held this
repository has never heard of, and a scope held with no `[[scope]]` record:

```sh
warlock: `/repo/.warlock/pacts.toml` records no `[[scope]]` record, so there is nowhere to file: add one
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

## Drafting

`warlock draft <PATH>` is the other half of that one. The project a push
recorded for the brief at `path` is read back, its `## Scope` section is parsed
into slices, the slices `.warlock/filed.toml` already holds a cut record for
are skipped, and every other one is drafted by a session of its own and filed
as issues on the same board. The project's status is not moved in either
direction, here or anywhere: a draft creates issues, writes the relations
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
them. Past that, each remaining slice in turn: one session drafts it, every
question it stops to ask is put to whoever started the run and answered with
the line they type, the drafts it comes back with are printed and a line is
read, and only `accept` goes any further — the team key becomes an id, the
team's `Backlog` state and the label are resolved, the issues are created, the
relations between them are written, and that slice's cut record is saved before
the next slice begins. The reading comes before all of that rather than inside
it, so a slice nobody accepted costs no request. Saving per slice rather than
once at the end is what stops a run that fails halfway from filing its first
slices a second time. After the last slice, and only when something was filed,
one comment on the project names the issues this run made and says the status
was not moved.

Progress is one line per slice as it is drafted, whatever it asked and was
answered with, its drafts by title, what can be said back about them, and one
line naming what was filed. The fraction is the place in the cut order and the
position is where the slice sits in the document, so a reader can find it in
the brief — the two differ exactly when a `depends_on` line moved something.
The bare `> ` is where the run stops and waits for a line:

```sh
$ warlock draft docs/warlock-brief-23-cut-a-planned-project-into-tickets.md
warlock: [1/3] slice 1 `The project fetch` — already cut as `WAR-121`, `WAR-122`, so nothing was sent
warlock: [2/3] slice 2 `The scope parser` — drafting
warlock: slice 2 `The scope parser` asked: is a `## Scope` heading with no `### ` slices under it an empty scope or a refusal?
warlock: slice 2 `The scope parser` — warlock's answer: a refusal, which is what the brief's own list of refusals asks for.
> a refusal, and it names the heading
warlock: slice 2 `The scope parser` was answered: a refusal, and it names the heading
warlock: slice 2 `The scope parser` — drafted `Parse the scope block`, `Refuse a scope with no slices`
warlock: slice 2 `The scope parser` — `accept` to file these, Enter or `skip` to leave it for another run, or say what these drafts should be instead
> accept
warlock: cut `The scope parser` into `WAR-123`, `WAR-124`
warlock: [3/3] slice 3 `The drafting session` — drafting
warlock: slice 3 `The drafting session` — drafted `Open a session per slice`
warlock: slice 3 `The drafting session` — `accept` to file these, Enter or `skip` to leave it for another run, or say what these drafts should be instead
> accept
warlock: cut `The drafting session` into `WAR-125`
```

`--dry-run` prints the project, the status it is in, the board, how many slices
there are and how many are already cut, then the slices themselves in the order
they would be cut in — and stops. No session is opened, no model pass is
bought, nothing is sent past the read that fetched the project and no record is
written:

```sh
$ warlock draft docs/warlock-brief-23-cut-a-planned-project-into-tickets.md --dry-run
warlock: would cut `Cut a planned project into tickets`, which is `Planned`, into `WAR` under the scope `warlock-team` — 3 slices, 1 already cut, and nothing was drafted
warlock: [1/3] slice 1 `The project fetch` — already cut as `WAR-121`, `WAR-122`
warlock: [2/3] slice 2 `The scope parser`
warlock: [3/3] slice 3 `The drafting session`
```

Somebody started the run and is watching it, so a session that stops to ask is
relayed rather than refused: each one gets three rounds, counted by warlock and
not by the model. The question goes out in the words it was asked, warlock's
own attempt at it goes out under that — made in a second conversation of its
own and never in the slice's — and then the bare `> `, where the run waits for
as long as whoever started it takes to answer. An empty line sends the
proposal, and a line with anything on it is sent instead of it whatever it
says: nothing weighs the two against each other, which is the whole of what
keeps the attempt an offer rather than a decision. What was sent is said in the
words it was sent in and never which of the two it was:

```sh
warlock: slice 2 `The scope parser` asked: is a `## Scope` heading with no `### ` slices under it an empty scope or a refusal?
warlock: slice 2 `The scope parser` — warlock's answer: a refusal, which is what the brief's own list of refusals asks for.
> 
warlock: slice 2 `The scope parser` was answered: a refusal, which is what the brief's own list of refusals asks for.
```

A question warlock has nothing to offer on is put all the same. Where the
brief, the slice and the repository do not settle it, the proposing session
answers with one fixed sentence and that sentence is what is printed; an
attempt that never came back at all — a missing binary or a cancel — is one
line naming why. Either way the read below it happens anyway, and the answer is
entirely the reader's:

```sh
warlock: slice 2 `The scope parser` — The brief, this slice and the repository do not settle this question.
warlock: slice 2 `The scope parser` — no answer was proposed: the model pass was cancelled before it finished
```

A question nothing answers leaves the slice uncut and the run goes on to the
next slice. Ctrl-D, or a pipe that has run out, is nobody there: nothing is
sent, the proposal included. Enter at a question warlock had no proposal for is
that same ending in its own words, because the alternative is a session asked
to draft on silence:

```sh
warlock: slice 2 `The scope parser` was not drafted: nobody answered its question
warlock: slice 2 `The scope parser` was not drafted: nothing was typed and warlock had no answer to propose
```

A question relayed after the session's rounds are spent is the one thing this
cannot pass on, and it leaves the slice uncut with a line saying exactly that.
The count is read before each turn rather than after, so the case is named
rather than run into — a session holds itself to its own rounds, so this is a
line nobody should see:

```sh
warlock: slice 2 `The scope parser` was not drafted: the session asked a question after its last round was spent
```

Nothing a slice drafts becomes an issue on its own. The drafts are printed by
title, what can be said back about them is printed under that, and a line is
read before anything is sent to the board: `accept` files them, case folded;
Enter or `skip` leaves the slice uncut and recorded nowhere, so a later run
reaches it again; and anything else is feedback the same session drafts again
from. Feedback is not read for a command or weighed against the drafts — it
goes back as that session's next turn, and what comes back comes up for review
again:

```sh
warlock: [3/3] slice 3 `The drafting session` — drafted `Open a session per slice and relay its questions`
warlock: slice 3 `The drafting session` — `accept` to file these, Enter or `skip` to leave it for another run, or say what these drafts should be instead
> the relay is its own ticket, split it off
warlock: slice 3 `The drafting session` is being redrafted: the relay is its own ticket, split it off
warlock: slice 3 `The drafting session` — drafted `Open a session per slice`, `Relay one question to the shell`
warlock: slice 3 `The drafting session` — `accept` to file these, Enter or `skip` to leave it for another run, or say what these drafts should be instead
> accept
warlock: cut `The drafting session` into `WAR-125`, `WAR-126`
```

A skip says so, and a pipe that ended before the review says which of the two
it was, because what a reader wants to know tomorrow is whether the next draft
will offer this slice again — and it will:

```sh
warlock: slice 3 `The drafting session` was skipped; nothing was recorded for it
warlock: slice 3 `The drafting session` was skipped: nobody said what to do with its drafts, so nothing was recorded for it
```

The review runs before any request, which is the whole of what makes a skip
free: a skipped slice sends nothing, creates no issue and writes no cut record,
so `.warlock/filed.toml` is left holding nothing that names it.

A slice that comes back with something other than drafts is a reported line and
the next slice rather than the end of the run — the slices left are other work,
and they were ordered so that nothing is filed before what it waits on. The same
goes for a relation Linear turned down and for the project's comment: the issues
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
turned down. The boundary's **3** is never spent by a draft and could not be,
for the push's reason: a sigil here picks which board the project is on rather
than opening a directory to be written, so there is no path being acted on for
the boundary to refuse.

No key value is printed by any of this either, not in the progress, not in a
refusal and not in the dry run. The value is read on exactly one line — the one
that builds the client — and only key *names* ever reach a line.

## Pulling

`warlock pull <SCOPE>` is the third step of the workflow and the only subcommand
that commits, branches or pushes. It takes the next ready ticket off that
scope's queue and leaves a pull request open behind it: the ticket moves to
`In Progress`, a session splits it into sub-tasks, a session of its own works
each sub-task, every one that finishes inside this machine's scopes is a commit,
and then the branch is pushed, opened as a pull request and its URL commented on
the ticket, which moves to the `review_state` the `[[scope]]` record names. The
scope is required for the push's reason: a pull is about one queue, and a
machine holding two sigils would have to guess which board's work to start.

Two flags and no more, as push and draft have two. `--ticket <TICKET>` works
that ticket instead of choosing one, and `--dry-run` says which ticket would be
taken without touching anything. There is no `--json`, for push's reason one
step on: what a script reads after a pull is the run record, and that is a file
under the home directory rather than a stream to catch. There is no `--any`
either — the queue is read by assignee, and taking a teammate's ticket is a
reassignment a human makes on the board.

The run record is one directory per ticket, under the same home the sigils and
the key binding for this checkout sit under:

```
~/.warlock/repo-f447b89a747182e2/pulls/WAR-140/
├── state.json      the record: the status, the branch, `pr_url`, a sub-task each
├── manifest.md     rendered from it on every write, for a person to read
└── WAR-140.01.md   one brief per sub-task, and what its session did
```

It is under the home and never in the repository, because a file recording how
far a pull got would otherwise turn up in the diff of the very commit it is
describing. The ticket identifier is the directory name, so one checkout holds
at most one run per ticket and a second pull of the same one picks the first up
rather than starting a run beside it. `state.json` is the record and the other
two are rendered from it on every save, so a hand edit to either is gone at the
next sub-task. Each brief's `## Execution log` is the one exception: the run
appends the session's lines under it as they happen and re-renders around it, so
the account of a sub-task is in its brief as well as on stdout.

The order of the work is the promise rather than an arrangement. The `[[scope]]`
record, the sigils, the key and the working tree are all settled before a socket
is opened, so every refusal in that stretch costs nothing — no request, no
branch, no session. A scope name nothing records is the first of them, and the
recorded names lead because one of them is almost certainly what was meant:

```sh
$ warlock pull warlock-tem
warlock: nothing in `.warlock/pacts.toml` records the scope `warlock-tem`, so there is no queue to read: this repository records `warlock-team`, `control-plane`
```

A recorded scope this machine's sigils do not open is the boundary's refusal and
keeps the boundary's **3**. It is asked here, in its own words, rather than left
to the machinery that resolves the key: that answers "not a scope this machine
can file to", which would send somebody to fix a record that is right.

```sh
$ warlock pull control-plane
warlock: the scope `control-plane` is not one this machine's sigils open, so nothing was pulled: this machine holds `warlock-team` — hold that sigil with `warlock config`
$ echo $?
3
```

The key half is the two refusals push prints, carried word for word rather than
reworded here: an unbound checkout is sent to `warlock key use` and a bound name
the store has never heard of to `warlock key add`. A dirty tree is the last of
the four, and it names every entry in `git status`'s own two-letter codes,
because that is the command the reader runs next:

```sh
$ warlock pull warlock-team
warlock: the working tree is not clean, so no ticket was pulled: ` M crates/engine/src/lib.rs`, `?? notes.md` — warlock commits what a session writes, and what is already there is yours
```

Nothing is stashed, reset or cleaned by a pull, here or later in a run.

Past those four the queue is read — every ticket on the record's team carrying
the record's label and assigned to the user the key belongs to — and every
ticket the pass walked past is a line with the reason on it. A halted run this
machine holds is passed over and names `warlock resume <TICKET>`, which is the
command that frees it; nothing a pull does releases a run by itself. A run this
machine holds as `resumed` is taken before any ticket with no record at all,
because somebody has already looked at it and there is a branch waiting on it.
So is a run left `in_progress` by a pull that was killed or crashed part-way:
the sub-task it was on goes back to pending, and the run carries on from there.
Nothing ready is an answer rather than a failure, and it is a **0**:

```sh
$ warlock pull warlock-team
warlock: passed over `WAR-141` — halted — `warlock resume WAR-141` releases it
warlock: passed over `WAR-142` — in `In Review`, which is waiting on a human
warlock: passed over `WAR-143` — in progress elsewhere — this machine holds no run record for it
warlock: passed over `WAR-144` — blocked by WAR-12 (Cole)
warlock: nothing in the queue for `warlock-team` is ready to work
$ echo $?
0
```

`--ticket` reads that one ticket instead of the queue, and the three filters the
queue applies on Linear's side are checked here instead, so a refusal can say
which of them the ticket failed rather than only that it is not in the queue. It
picks among your own work and there is no flag past that. Each of these is an
ordinary **1**, and the reasons the chooser skips by are the same sentences the
pass-over lines carry:

```sh
warlock: `WAR-9` was not pulled: on team `DAT`, and this scope routes to `WAR`
warlock: `WAR-9` was not pulled: not labelled `warlock` — it carries area/docs
warlock: `WAR-9` was not pulled: assigned to Cole and not to you
warlock: `WAR-9` was not pulled: halted — `warlock resume WAR-9` releases it
```

Three more are about a ticket that was never in the queue to be skipped. A word
that is not an identifier is turned down before a request is made, because
asking the board about `banana` would turn a typo into a round trip and a vaguer
answer; an identifier the board has no issue for is named as that; and a ticket
that is finished says the state it finished in rather than a rule it also
happens to fail. A ticket nobody has taken is refused for the assignee's reason
with a sentence of its own, since there is nobody in it to name:

```sh
warlock: `banana` was not pulled: `banana` is not a ticket identifier, which reads like `WAR-9`
warlock: `WAR-9000` was not pulled: the board has no `WAR-9000`
warlock: `WAR-9` was not pulled: in `Done`, which is finished
warlock: `WAR-9` was not pulled: assigned to nobody, and `pull` works your own tickets
```

`--dry-run` stops at the queue. It prints the ticket it would take and every
ticket it passed over with the reason, writes nothing, raises no session, makes
no `git` call at all — the tree read included — and exits 0:

```sh
$ warlock pull warlock-team --dry-run
warlock: would pull `WAR-140` — Add `warlock pull <SCOPE>`, and nothing was written, no `git` ran and no session was raised
warlock: passed over `WAR-141` — halted — `warlock resume WAR-141` releases it
```

The missing `git status` is the point rather than an omission: what a dry run
answers is which ticket would be taken, and the dirty-tree refusal above would
be a command it promised not to run. With `--ticket` it says whether that ticket
would be taken and why not, and refusing to take one is still a 0 — nothing was
refused, it was described:

```sh
$ warlock pull warlock-team --ticket WAR-141 --dry-run
warlock: would not pull `WAR-141`: halted — `warlock resume WAR-141` releases it
$ echo $?
0
```

A run that goes ahead prints in `running.rs`'s shape: a header per section — the
split, each sub-task, the pull request — with the session's own lines under the
header they happened under.

```sh
$ warlock pull warlock-team
warlock: splitting `WAR-140` — Add `warlock pull <SCOPE>`
warlock: [1/2] `WAR-140.01` Add the pull loop's module, seams and pure decisions
warlock: Read crates/warlock-tui/src/pulling.rs
warlock: writing · 12 KB
warlock: [2/2] `WAR-140.02` Work the run
warlock: thinking
warlock: `war-140/add-warlock-pull-scope-work-a-ticket` is pushed, opening a pull request
warlock: `WAR-140` is in review: https://github.com/acme/warlock/pull/149
```

A cost report is the one thing a session reports that is not a line, following
the panel's account card: it is a fact about the pass rather than something the
session did, and a column of money down the middle of the work is not what the
run is being read for.

A run that stops short of a pull request halts, and a halt is not the loop
failing: the branch holds one commit per finished sub-task, `state.json` holds
the rest, and the ticket keeps one comment listing what finished, then each
sub-task that stopped with its status and reason, then the ones never started.
The ticket is left where it is. One `blocked` or `failed` sub-task is not on its
own the end — the run carries on to any sibling that does not wait on it, and
halts when nothing is runnable.

```sh
$ warlock pull warlock-team
warlock: the run for `WAR-140` halted, so the ticket has not moved: its comment lists what finished and what did not, and `warlock resume WAR-140` releases it
$ echo $?
1
```

A session that commits by itself halts the run the same way, with that sub-task
recorded as `failed` and both short commit ids in the reason: `HEAD` moving
means what the session wrote is in a commit warlock did not make and cannot
check, so nothing further is committed.

A crossing is the one halt that is a boundary, and the second of the two **3**s.
A session that wrote under a scope this machine does not hold stops the run
where it stands — nothing is committed, the tree is left exactly as that session
left it, and what happens to the work is a person's to decide:

```sh
$ warlock pull warlock-team
warlock: `WAR-140.01` wrote under a scope this machine does not hold, so the run for `WAR-140` stopped with nothing committed: the working tree is exactly as that session left it, and the ticket's comment names the paths
$ echo $?
3
```

A resumed run is picked up on its own branch, and a clean tree is required there
too. That refusal is the loop's own and names the branch it just checked out,
the entries in it and that what a halted sub-task left is yours to keep or to
drop: warlock will not fold somebody's half-finished edit into the next
sub-task's commit under that sub-task's message.

Three things a run reports and works past. A team whose workflow has no
`In Progress` state, and one with no state matching the record's `review_state`,
are each a line and a run that carries on, because a board nobody has finished
setting up is warlock's to report rather than to stop over:

```sh
warlock: the team `WAR` has no `In Progress` state, so the ticket was not moved
warlock: the team `WAR` has no `In Review` state, so the ticket was not moved into review
```

No `gh` on this machine is the third, and it is still a finish said as what
happened: the branch is pushed, the body of the pull request goes on the ticket
as a comment for whoever opens the request by hand, `pr_url` stays `null`, the
ticket still moves and the run still exits 0.

```sh
warlock: `WAR-140` is in review, and there is no `gh` on this machine — the branch is pushed and the pull request's body is a comment on the ticket
```

A run record under `pulls/` that will not read is named the same way and the
pass carries on without it; only a `pulls/` directory that cannot be listed at
all stops a pull, because then nothing can say which runs this machine is
holding.

Exit status is **0** when a pull request was opened and when nothing was ready —
an empty queue is an answer, exactly as an empty listing is. It is **3** twice,
and both are the boundary's: a scope this machine's sigils do not open, refused
at the start with nothing spent, and a sub-task that wrote past one, which is a
run stopped with nothing committed. Everything else is an ordinary **1** — the
unrecorded scope, the two key refusals, a dirty tree either side of a resume, a
named ticket the queue's rules turn down, a halt, and whatever `git`, Linear or
the run record said when it would not answer. A missing scope is clap's **2**,
as a missing path is everywhere else.

No key value is printed by any of this. The loop is handed a board that is
already open rather than a key, so nothing inside a run has one to print, and
the value is read on the one line that builds the client.

## Resuming

`warlock resume <TICKET>` is a person saying they have looked at a halt, and it
is the only thing that turns one back into work. A halted run waits on that and
on nothing else: its `failed`, `blocked` and `crossed` sub-tasks stay in those
states, so a pull picking the run up again would find nothing runnable and halt
a second time having done no work. A resume puts those sub-tasks back to
`pending` and the run to `resumed`, which is the state a pull takes ahead of any
ticket with no record at all. The `done`, `pending` and `in_progress` sub-tasks
are left exactly as they were — a resume is about what stopped.

The ticket is required and there is no whole-queue spelling for an omitted one
to mean: a halt is one run, and the run records are keyed by ticket.
`--failed-only` is the one flag, for the halt where half of it has been dealt
with: it puts only the `failed` sub-tasks back and leaves a `blocked` or
`crossed` one with the status and the reason it is carrying, so the test failure
that has been fixed becomes runnable and the key that is still missing stays in
the way. There is no `--json`, for the pull's reason — what a script reads after
a resume is the run record, which is a file rather than a stream to catch — and
no `--dry-run`, because a resume that would change nothing is a refusal that
writes nothing, so the dry run is the run.

```sh
$ warlock resume WAR-142
warlock: `WAR-142.02` was `failed` and is `pending` again — `cargo test` came back red
warlock: `WAR-142.03` was `blocked` and is `pending` again — the Linear key for `control-plane` is not on this machine
warlock: `WAR-142.04` was `crossed` and is `pending` again — wrote `crates/control/src/lib.rs`
warlock: `warlock pull warlock-team --ticket WAR-142` works the ticket again
$ echo $?
0
```

One line per sub-task it changed and none for the ones it left alone. Each names
the status that sub-task had and the reason that status carried, and that line
is the only surviving copy of the reason: the reset drops it from the record,
because a `pending` sub-task still saying why it stopped last time reads as one
that is still stopped. The hand-off is last, and the scope on it is read out of
the record rather than asked of anything — the run knows which queue took it,
and sending the reader to the board for a fact the file is holding would be a
worse line.

The record is the only thing written: `state.json` is saved and the
`manifest.md` and briefs beside it are re-rendered from it, exactly as any other
save of a run does, and nothing inside the repository is touched. The save
happens before a word is printed, which is the opposite of the order a push
prints in and for the opposite reason — nothing here has happened until
`state.json` lands, and lines printed first would tell somebody their halt was
released by a command that then failed to write it.

Nothing else is spent at all: no Linear request, no `git`, `gh` or `claude`, no
ticket moved and no comment. That is why the scope and the sigils are never
asked — there is nothing here for a boundary to gate — so a ticket whose scope
this machine does not hold resumes, a dirty tree resumes, and a resume never
exits **3**. Both of those questions belong to the pull that picks the run up,
which asks them of the checkout it is about to work in.

Two refusals, and both are ordinary **1**s. A ticket this machine holds no run
for names the directory that was looked in, because the answer is almost always
the wrong checkout or the wrong machine rather than the wrong ticket:

```sh
$ warlock resume WAR-9
warlock: this machine holds no run for `WAR-9`, so there is nothing to resume: warlock looked in `/home/you/.warlock/repo-f447b89a747182e2/pulls/WAR-9`, and a record is written there by the `warlock pull` that starts the ticket
$ echo $?
1
```

A run with nothing to put back names the status the record was holding when it
was read, which is the answer — a run in `in_review` has finished and one in
`pulled` has not stopped — and then which of the two modes was asked, since the
flag is half of why there was nothing in the run to release:

```sh
$ warlock resume WAR-142
warlock: the run for `WAR-142` is `in_review` and has no sub-task to put back, so nothing was written: a resume puts the `failed`, `blocked` and `crossed` sub-tasks back
$ warlock resume WAR-142 --failed-only
warlock: the run for `WAR-142` is `halted` and has no sub-task to put back, so nothing was written: `--failed-only` puts a `failed` sub-task back and leaves a `blocked` or `crossed` one as it is
$ echo $?
1
```

After either of them `state.json` is byte-identical to what was read: the reset
lives in memory until it is saved, and a refusal never saves. A record that is
there and will not parse is neither refusal and says what would not read: a
record broken by a hand edit describes a branch that may be holding somebody's
uncommitted work, and being told there is no run would send the reader off to
start the ticket over.

## Exit statuses

| Status | What it means |
| --- | --- |
| `0` | Completed. The question was answered or the write happened, whatever the answer turned out to be — an empty listing, a queue with nothing ready to pull, and a scope closed to this machine included |
| `1` | Warlock could not do it, or would not: the repository will not resolve, the manifest will not parse or will not save, the path has no repository-relative spelling, a scope name nothing records yet was given without all three record flags or with a blank one, a name that already has a record was given any of them, a brief's template or `briefs.toml` will not read, a push has no board or more than one, the brief is not one or is already filed, a draft's brief is not recorded in `.warlock/filed.toml`, its project is one Linear does not know or is not `Planned`, the scope block will not cut or has nothing left to cut, the team has no `Backlog` state, a pull's scope is one nothing records, the working tree is dirty, a named ticket is one the queue's rules turn down, a run halted, a resume was asked for a ticket this machine holds no run for or a run with nothing to put back, or Linear refused what was sent. The line on stderr is the thing to go and read |
| `2` | The command line was never a request. Clap's status and its wording, for a word warlock has no place for |
| `3` | The sigil boundary, in the three places it is reached: this machine's sigils do not open the scope covering the path, they do not open the scope a pull was asked for — both refused at the start with nothing spent — or a pull's sub-task wrote under a scope they do not open, which stops the run with nothing committed and the tree as that session left it. Retrying changes nothing, and the road out is `warlock config` |
| `4` | Completed with failures: a run wrote the documents it could and saved the manifest, and the lines above the count name the directories that did not come out of it |
| `130` | Cancelled: somebody pressed Ctrl-C during a run, and what had finished by then is saved and granted. 128 plus SIGINT, so a shell, `make` and CI read it as interrupted without being told anything about warlock |

The three that are not 1 are not 1 because they want different things done about
them. A 3 says this checkout is outside that boundary, so stop and go and get
the sigil — and where a pull crossed one, the tree is still holding what that
session wrote. A 4 says the work is partly on disk, so re-run over the part that
is not. A 130 says somebody decided to stop it, so nothing should retry it at
all.
Telling those apart by their wording would be telling them apart by parsing
prose.