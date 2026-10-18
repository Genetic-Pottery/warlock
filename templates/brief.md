# A title line naming the change, in 80 characters at most

Open with the problem, in prose and before any heading: what is wrong now, in
this repository, naming the files and the behaviour. Say what it costs to leave
it alone.

## Outcome

What somebody sees once the change is made, written as something a reader can
watch happen: a session, a screen, a command and what it prints. The work goes
further down.

## Success criteria

Facts a reader can mark done or not done, gathered into bolded groups:
`**One part of the change**` on a line of its own, then its bullets. One group
per part.

## Constraints

What must not change and what the work may not reach for: dependencies,
architecture, decisions already settled. Each one a rule the work is made under.

## Out of scope

What is deliberately not being done, each with the reason it was refused.

## Scope

The work as numbered slices. Each slice is `### N. What the slice does`, then a
line `depends_on: [<slice numbers it needs first>]`, then `scope: <scope name>`,
then paragraphs: what exists now, naming files and functions; what changes and
why this shape over the alternatives the conversation weighed; and what a later
edit must not break. A slice lands on its own and stays inside one scope. Work
that spans two scopes is two slices, one per scope, linked by `depends_on`.
