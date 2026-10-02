![Warlock](assets/warlock-logo.png)

# warlock

(Currently under development, internal release phase)

See your codebase the way your AI does. A TUI where documentation is the interface.

> **warlock** *(n.)* one who draws power from a pact with an entity greater than
> themselves. The patron is the model. The pact is the boundary. Invocations are
> agent runs.

## What this is

AI tool which medium to large sized companies could use as a team effectively.
I imagined what would an IDE look like 20-30 years from now and came up with this.
AI is the star of the show and looking at code is not the primary concern.

## What you get

Text User Interface (TUI)
- pacts: A directory under the control of warlock, it scans the directory and creates an artifact to help itself reason about the code.
- scope: The ability to restrict work to specific directories, which helps guard against an AI overstepping.
- sigil: A local per-user array of string which allow you and your AI to access a specific repos scope.

Workflow
- brief: The ability to talk to the AI and discuss large feature of work you wish to create, you will have you assumptions pushed up against and at the end of the conversation you can write a document to the repo.
- draft: The ability to turn a brief into multiple tickets with dependency graph relationships.
- pull: Warlock will pull the next available ticket, cut it into sub-tickets locally and work within your allowed scope. There is local state and ability to recover from going over limits or crashes.

## Walk through

### File tree navigation
![file tree demo](./assets/file-tree.gif)
> move up / down, collapse directories, show / hide only pacted directories, show / hide files

### Brief creation
![brief creation demo](./assets/brief.gif)
> type /brief and begin a conversation defining a large scope of work where the AI challenges you for more details of the scope

### Writing a brief
![writing a brief demo](./assets/write.gif)
> type /write once your shared understanding with the AI has been reached and a document will be written which eventually will be turned into tickets

### Pushing a brief
![pushing a brief demo](./assets/push.gif)
> type /push [PATH_TO_BRIEF] to push a brief to Linear as a Project

![pushed brief in Linear demo](./assets/push-linear.png)
> the Project will land in Linear with a set format which future workflows can digest and turn into Issues

### Drafting into issues
![drafting into issues demo](./assets/draft.gif)
> type /draft [PATH_TO_BRIEF] to draft a Linear Project into Linear Issues, if a model requires more information to make a well scoped Issue it will ask you to provide clarification

![drafted issues in Linear demo](./assets/draft-linear.png)
> Issues appear in backlog with a customizable label, Project association, and assigned to self

### Pulling issues
![pulling an issue demo](./assets/pull.gif)
> work is pulled for specific scopes; an Issues is pulled which has no blocking tickets, is assigned to you, and has the custom label

![pulled issue finished in Linear demo](./assets/pull-linear.png)
> when work is being worked on it is In Progress and once it is completed it is moved to a customizable final state, in this case In Review

![pulled issue finished in Github demo](./assets/pull-github.png)
> once work is complete the Linear ticket and dialog will display the Github URL where the PR is awaiting a human review

## Why this exists

Editors and AI-enabled IDEs bolt AI on the side. The file tree is still the
interface, what they accelerate is how fast you type rather than how well the
model understands, and the model itself is a helper you summon into the margin
of a window built for typing.

Meanwhile most people are shipping AI-written code and sanding it down to look
hand-written. Stripping the em dashes. Not letting the model commit. A whole
industry using AI while performing restraint, which means using it poorly: no
structure, no shared context, no record. The pretending is the waste.

Warlock admits what the game is. The interface is not your filesystem, it is
your project rendered as the AI understands it: a tree of module documents,
coloured by whether that understanding is still true. That document is a
`WARLOCK.md`, one per directory, written by the AI and committed alongside the
code it describes. You can still open files and read them. That is no longer the
main event.

## What it promises

- **Documentation that is actually current.** The document the AI use to reason about the code
  turn stale as soon as the code refernced in the document changes. Your documents are always up to date.
- **A record that survives you.** Every change lands in the ticket tracker of youre choice.
  Or is commited within the repo. Everything is visible.
- **Process artifacts as a byproduct.** Most teams LARP process: the ticket
  exists, the doc exists, and both are one-sentence husks. Warlock's artifacts
  are real by construction, because the work runs through them rather than
  around them. I order to use the tool you MUST talk to the AI to challenge your assumptions,
  turn that resulting document into tickets visible to the team, and work on tickets and leave a PR for 
  human review.
- **Your subscription, your leverage.** Warlock holds no credentials of its own
  and resells no inference. It drives the `claude` CLI you are already paying
  for, which has to be on your `PATH` and logged in before Warlock can do
  anything with a model.

## What it is not

- **Not autonomous.** Nothing merges because a model felt good about it. Every
  consequential step is a human decision, and the point of the framework is that
  the decision leaves a trace instead of evaporating.
- **Not a project generator.** It will not turn one sentence into a shipped
  feature, and it does not manufacture thought. A lazy one-liner inflated into a
  beautifully formatted page is still a lazy one-liner, now with better
  typography.
- **Not an editor.** You can hand-edit anything, and the next refresh will
  notice and reconcile. But the tool is not optimised for code editing, and
  anyone who wants to is not the customer.

## Troubleshooting

**Why are my borders not even?**

Your terminal is almost certainly falling back to a second font. Warlock draws
its panes with Unicode box-drawing characters, one per cell, and a font with no
glyphs for them hands the job to whichever font the terminal finds next — whose
glyphs are rarely exactly one cell wide or aligned on the same baseline. The
result is segments that sit slightly high, low or short, and corners that do not
meet. Nothing is wrong with the layout: every border is a single character in a
single cell, and it lines up the moment the font can draw it.

Use a monospace font with box-drawing coverage — most programming fonts have it
— or turn on your terminal's own line-drawing, which some emulators offer as a
setting for exactly this reason. If it is only *some* rows that are off, check
whether your terminal is configured to treat ambiguous-width characters as
double width.

## Contributing

Run these three checks before pushing. CI runs exactly the same three commands on
every push and pull request, so if they pass locally they pass there too:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
