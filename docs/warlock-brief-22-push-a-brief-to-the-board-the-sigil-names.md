# Push a brief to the board the sigil names

A brief ends at `writing.rs:142`. The document is written into the directory
`briefs.rs` reads from `.warlock/briefs.toml`, the thread says
`wrote docs/… — 8.1 KB`, and warlock has nothing else to offer. Everything
after that is a person with two windows open: a Linear project made by hand,
the title retyped, the body pasted, the team picked from a dropdown.

Brief 20 built the whole answer to *where* and stopped one step short of
sending anything. `.warlock/pacts.toml` holds `[[scope]]` records — this
repository's says `name = "warlock-team"`, `team = "WAR"` — `~/.warlock/keys.toml`
holds named Linear keys, the checkout binds one of them, and `route.rs` joins
the three. `warlock check crates/warlock-tui` already prints
``filing to `WAR` would use the key `work`, which this machine stores``. Warlock
can name the board, name the key, and send nothing to either.

So the twenty-one documents in `docs/` were filed by Red, and the evidence is
committed in this repository: `.red/7a6083f447d2/state.json` records brief 21 as
Linear project `b229262b-22aa-444a-a8af-0a2a3f4ef100`. Red picks its team from
`LINEAR_TEAM_KEY` in `~/.config/forman/.env` — one flat file, one team, one key,
per machine rather than per repository. That is the arrangement brief 20 refused
by name, and warlock's own briefs go through it.

Leaving it alone costs the retyping every time, and once in a while it costs the
expensive failure: the project lands on a board nobody on that work reads, and
the first person to notice is somebody else.

## Outcome

Somebody finishes a brief in the panel and types `/write`. The file lands, and
they type `/push`. A dialog names what is about to happen — the project
`Repair the answer instead of refusing the document`, the team `WAR`, the label
`warlock`, the key `work` — with No under their finger. They press Left and Enter, the thread says
`filing to WAR`, and a line lands with the URL of a project that holds the whole
document.

At a shell the same file goes up with
`warlock push docs/warlock-brief-22-push-a-brief-to-the-board-the-sigil-names.md`,
which prints the team, the project and the URL. Run again, it refuses and prints
the URL of the project that file already made.

On a machine holding `warlock-team` and `data-plane`, in a repository that
records both, the push refuses without sending anything: it names the two scopes
and says `--scope` picks one. On a machine holding no sigil at all it refuses and
says a sigil is what picks a board — `warlock config` is where one is written.
Neither refusal reaches Linear, because neither one has a board to reach.

## Success criteria

**Choosing the board**

- The candidates are the scope names this machine holds a sigil for that
  `.warlock/pacts.toml` also records. Exactly one candidate is the board.
- No candidate is a refusal that says whether the machine holds nothing, holds
  sigils this repository records no scope for, or holds one whose scope has no
  `[[scope]]` record.
- More than one candidate is a refusal naming every one of them.
- The wildcard sigil `*` is treated as every recorded scope, so it is one
  candidate in a repository recording one scope and a refusal in a repository
  recording two.
- A name given explicitly is used when it is among the candidates, and refused
  when it is not, with the candidates named.
- An unbound checkout, and a bound name absent from `~/.warlock/keys.toml`, stay
  the two distinct refusals `route.rs` already words.
- Every refusal above happens before a socket is opened.

**What is sent**

- The project's name is the document's first `# ` line, with the marker
  stripped, and the project's content is the rest of the document verbatim.
- A document missing any section of the repository's brief template is refused
  through `missing_sections`, the function `/write` already refuses with.
- The record's `team` is read as a Linear team key and resolved to an id through
  the API. A key the API does not know is a refusal naming it and
  `.warlock/pacts.toml`.
- The project is created with the status named `Backlog` when the workspace has
  one, and with no status when it does not.
- The project carries the record's `label` — `warlock` here. It is the only mark
  on a project saying warlock filed it, and a pull that reads the workspace
  reads the labelled projects and nothing else.
- A label the workspace already has is reused by id. One it does not have is
  created once and reused after that, so a second push adds no second label of
  the same name.
- A label that does not attach fails the push, records the project and prints
  the URL with a line saying the label is missing. A project warlock filed and
  did not mark is a project no pull will ever see.
- No summary, no description and no field warlock would have to invent.

**The record**

- A successful create writes `.warlock/filed.toml`: the brief's
  repository-relative path, the project id, the URL, the scope, the team and the
  time, with a `version` key refused when it is not the one this warlock writes.
- A path the file already records is refused before anything is sent, and the
  refusal prints the recorded URL.
- The write is atomic through `write_and_sync` and a rename, like every other
  file this workspace saves, and leaves records it did not add byte-identical.
- A create that succeeds and a record that will not save still prints the URL,
  says the record failed, and fails: the one thing that must never be lost is
  the address of a project that now exists.

**`warlock push`**

- `warlock push <PATH>` takes the brief's path, required, and `--scope <NAME>`
  picks among candidates.
- `--dry-run` prints the board, the key name, the project name and the size of
  the content, opens no socket and writes no record.
- Success prints the team, the label, the project name and the URL.
- Every refusal here is an ordinary **1**. The boundary's **3** is not spent on
  any of them.
- `push` takes no `--json`, matching the other writing subcommands.
- `HEADLESS-CLI.md` documents the subcommand, the flags, every refusal and the
  exit status, in the table and in a section of its own.

**`/push` in the panel**

- `/push` is a fourth command, and `Submitted::refusal`'s one sentence names all
  four.
- `/push` pushes the file `/write` wrote in this session. A session that has
  written nothing refuses and names `/write`.
- More than one candidate opens a field for the scope name before the dialog,
  refusing a name that is not a candidate with the candidates on the rule line.
- The dialog names the project, the team and the key name, and answers No to
  Esc and to an immediate Enter.
- The request runs off the event loop, the way a pass does. Frames keep drawing
  while it is in flight, and one push is in flight at a time.
- The URL lands on the thread. A failure lands there too, in one line, and tears
  nothing down.

## Constraints

- One new dependency: a blocking HTTP client with a rustls backend, taken in
  `warlock-tui` beside `claude.rs`. No async runtime — `claude.rs` states that
  rule for the one other thing warlock talks to — and nothing that links a
  system TLS library, for the reason the `arboard` entry gives.
- `warlock-engine` gains no HTTP and no Linear vocabulary. Its manifest says so
  in the comment above `[dependencies]`, and choosing a board is manifest,
  sigils and keys, which the engine already owns.
- A key reaches the `Authorization` header and nowhere else. Not a line, not an
  error, not a `Debug`, not a `--dry-run`. Only key names are ever printed, as
  brief 20 settled.
- Linear personal keys take the key as the bare `Authorization` value, with no
  `Bearer` prefix. That is a wire fact nothing in the code can show.
- No model pass. The document goes up as its author wrote it.
- `scope_covering` and `scope_opens_to` stay the only implementation of the
  boundary rule, and `route.rs` keeps wording the key refusals.
- `.warlock/pacts.toml` is not written by any of this. A push reads the manifest
  and writes only its own record.
- Existing command spellings, `--json` envelopes and exit statuses do not move.
- One request per operation, one timeout, no retry and no backoff. A failure is
  reported and the operator runs it again.
- The comment rules in `CLAUDE.md` apply. The wire format above earns a comment;
  a walk through the request does not.
- The repository root carries the `warlock-team` scope, so every part of this
  work is under it. Confirm what this machine holds with `warlock config` before
  starting.

## Out of scope

- **Filing the slices as issues.** Red splits that deliberately: a project is
  created in `Backlog`, a person reads it in Linear and edits it there, and only
  a project they have moved to `Planned` is turned into issues. Issues cut from a
  document nobody has read in Linear are issues cut twice. It is the next brief.
  The record's `review_state` is what that brief is for and stays unread here;
  the `label` is read now, because a pull that cannot tell warlock's projects
  from everybody else's is a pull that cannot be written.
- **Updating a project already filed.** `projectUpdate` would make warlock the
  second writer of a document people edit in Linear, and the one that overwrites
  without reading. A brief that changed after it was filed is edited where it is.
- **Pulling anything back.** Nothing in this brief reads Linear's state, and one
  machine moving state other people can see is where a hard block belongs rather
  than a warning. Deciding that here settles it before the case exists.
- **A lead, members, milestones or a priority on the project.** Every one is a
  field warlock would have to guess at, and a guess that is wrong is a fact
  somebody has to go and unset.
- **A `--json` envelope on `push`.** The writing subcommands take none, and the
  answer that is worth parsing — the URL — is in `.warlock/filed.toml`, which is
  a file rather than a stream that has to be caught.
- **Choosing the board from the brief's text.** A document naming
  `crates/warlock-engine` looks like it says which team owns the work, and
  inferring one puts a wrong assumption into a project nobody has read yet. The
  sigil is a statement; the prose is not.
- **An interactive gate on the CLI.** Red asks create, edit or quit at a
  terminal. `warlock push` is documented headless like every other subcommand,
  and `--dry-run` answers the same question without blocking a pipeline.
- **Retries, backoff and rate limiting.** One project per push is one request
  that either lands or does not, and a retry around a create is the shape of
  mistake that files two projects.

## Scope

### 1. The Linear client

depends_on: []

A module beside `claude.rs` holding the transport and the four operations this
brief needs: resolve a team key to an id, find the `Backlog` project status,
resolve the label to an id and create it when the workspace has none by that
name, and create the project.

Whether a Linear project takes its label on the create input or through a
second mutation is the schema's answer and is not settled here. What is settled
is the order: the label exists before the project does, so a create that
succeeds is a project that is marked. It sits in `warlock-tui` rather than in a third crate for
the reason `claude.rs` sits there: the boundary between warlock and the outside
world is already the binary's, and the engine's manifest refuses HTTP outright. A
third crate was considered and rejected — it would buy a test seam that a trait
over three functions buys more cheaply, and the workspace's two crates have one
edge between them worth keeping.

This slice decides that the key is a parameter into the client and never
something it reads, exactly as the home is a parameter through `sigils.rs`, so
no test can reach a real credential and no error can carry one.

### 2. Choosing the board

depends_on: []

An engine function from the manifest, the machine's sigils and an optional name
to one filing target or one named refusal: the scope, its record, the key name
and the key. It is the same shape as `resolve_route` and the same argument for
it — one implementation, so two callers cannot come to disagree about which
board a repository files to.

The decision this slice makes is that the candidate set is the intersection of
what the machine holds and what the repository records, and that anything other
than exactly one candidate refuses rather than picks. A brief is not about a
directory — brief 12 refused to infer a footprint from the cursor for the same
reason — so there is no path to walk up and no nearest scope to win. Ambiguity
here is somebody's project on the wrong board, found out later by somebody else,
which is worth one refusal and one `--scope`.

### 3. The filed record

depends_on: []

`.warlock/filed.toml` in the engine beside `manifest.rs`: read, append, save,
with the same version key, the same atomic write and the same round-trip promise
that records it did not add come back byte-identical.

Committed rather than machine-local, and inside `.warlock/` rather than in the
document. The repository is where the fact belongs — a colleague's clone knows
what has already been filed without asking anybody — and `.warlock/` is left out
of the digest by name in `walk.rs`, so recording a push makes nothing stale. A
footer appended to the brief would have done the same job and made the brief
directory stale wherever `docs/` is not ignored, which is most repositories.

### 4. The document that becomes a project

depends_on: []

Title off the first `# ` line, content is the rest, sections checked through
`missing_sections`. The decision is that warlock sends the document and invents
nothing: no description, no summary, no status beyond `Backlog`. The alternative
is a model pass to summarise, which costs a turn and puts prose in front of a
reader that nobody wrote and nobody checked.

### 5. `warlock push`

depends_on: [1, 2, 3, 4]

The subcommand, its two flags, its refusals and its documentation. It follows
the conventions the other headless verbs set: resolve the environment once
through `Standing`, return a `Result`, let `main` print the failure, and add the
new `Error` variants to `status_for` in the ordinary **1**. The decision here is
that no refusal in this command is a **3**: the sigil picks the board rather
than gating a directory, and sending a script to `warlock config` over an
ambiguous `--scope` would be the wrong road.

### 6. `/push` in the panel

depends_on: [1, 2, 3, 4]

The fourth slash command, the one sentence in `submission.rs` that lists them,
the scope field when there is a choice, the confirmation dialog, and the request
driven off the event loop the way `claude.rs` drives a pass. The decision is
that the panel confirms and the CLI does not: the panel is where the brief was
written seconds earlier by somebody who has not seen the board named yet, and a
dialog that answers No to Esc and to a reflex Enter costs one keystroke to get
past. A shell has already said what it meant by typing the path.
