//! Every seam one pull spends arrives in [`Pulling`], and nothing in this module
//! reaches for one: no environment variable is read, no socket is opened, no
//! process is spawned and no [`Standing`](crate::standing::Standing) is
//! resolved. That is what lets a whole run — each halt and the crossing included
//! — be driven by values, with no board, no repository and no `claude` on the
//! machine.
//!
//! The tree check is deliberately not a seam of its own. What a session wrote is
//! read through [`Repository::dirty`] — already a seam — and judged by
//! [`crossings_in`], which is a function over values; what the loop cannot read
//! for itself is the manifest and the scopes open on this machine, so those arrive
//! as fields beside the rest. A trait wrapping the pair was the alternative, and
//! it would have had to answer with an owned mirror of [`Crossings`]: a second
//! record of one finding, which [`pull_request_body`] then wants borrowed again.
//!
//! [`next_runnable`] and [`halt_comment`] are the two decisions a run makes that
//! are not requests, and both are functions of a [`PullRun`] alone. The record is
//! the whole input because the record is the whole state — it is written before
//! and after every sub-task and read back by the next invocation, minutes or days
//! later — so a second source for either would be a second opinion about a run
//! that somebody's working tree is holding.
//!
//! The record is machine-local: it lives under the home directory and never
//! inside the repository, which is why [`Pulling`] carries both paths. A file
//! recording how far a pull got would otherwise turn up in the diff of the very
//! commit it describes.
//!
//! The activity port is the caller's to bridge. [`Activities`] takes a
//! `Fn(Activity) + Send + Sync + 'static` and the progress sink here is a
//! `&mut dyn FnMut`, so a door that wants a session's activities on its
//! [`PullEvent`] stream owns whatever joins the two.
//!
//! [`Activities`]: crate::claude::Activities
//! [`Crossings`]: crate::crossings::Crossings
//! [`Repository::dirty`]: crate::git::Repository::dirty
//! [`crossings_in`]: crate::crossings::crossings_in
//! [`pull_request_body`]: crate::git::pull_request_body

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;

use warlock_engine::working::{Accepted, Reported};
use warlock_engine::{
    LinearScope, Manifest, PullRun, PullSubtask, RunStatus, SubtaskStatus, now_rfc3339, pulls,
};

use crate::board::{Board, Error as LinearError};
use crate::claude::{Activity, Sibling, Split, Worked, working_opening};
use crate::crossings::{Crossing, crossings_after};
use crate::git::{
    Dirty, Error as GitError, Finished, Forge, Head, PullRequest, Repository, Touched, branch_name,
    commit_message, pull_request_body, pull_request_title,
};
use crate::queue::{IN_PROGRESS, same_name};

/// Everything one pull is allowed to touch, built by whichever door is pulling.
///
/// Both halves of git are here because only one of them may be missing: a
/// [`Repository`] has to work, and a [`Forge`] may come back with no `gh` at all.
/// The board arrives opened, so no key value reaches this module and none can be
/// printed from it.
///
/// No `Debug`: a sink is not a value to print, and the rest of this is paths and
/// seams a failing assertion would dump a screenful of.
pub(crate) struct Pulling<'a, B: Board, R: Repository, F: Forge, S: Splits, W: Works> {
    pub(crate) board: &'a B,
    pub(crate) repo: &'a R,
    pub(crate) forge: &'a F,
    pub(crate) split: &'a S,
    pub(crate) sessions: &'a W,
    /// The scope the ticket was pulled under, read as a Linear scope: the team
    /// to file against, the label the queue is read by, and the state a finished
    /// run moves the ticket to are all on it, and asking the manifest again here
    /// would be a second reading of a record the door already resolved.
    ///
    /// The view and not the record, because a view can only be had from a record
    /// that files to Linear: every board request below is made against a team
    /// key, and this is what says the door answered that question before the
    /// first one was sent.
    pub(crate) scope: LinearScope<'a>,
    /// What the check after a session judges a changed path against, with
    /// `held`. The manifest in hand rather than one loaded here: reading it
    /// again could disagree with the boundary the door already judged.
    pub(crate) manifest: &'a Manifest,
    /// The flattened open scopes, as
    /// [`crossings_in`](crate::crossings::crossings_in) and
    /// [`permits`](crate::boundary::permits) take them — a config that would not
    /// read is the door's to say and not a third answer to give in here.
    pub(crate) held: &'a [String],
    /// The branch this checkout set with `warlock branch use`. `None` is the
    /// remote's default, detected at each point it is needed.
    pub(crate) base: Option<&'a str>,
    pub(crate) root: &'a Path,
    /// Where the run record goes, never under [`root`](Self::root).
    pub(crate) home: &'a Path,
    pub(crate) progress: &'a mut dyn FnMut(PullEvent),
}

impl<B: Board, R: Repository, F: Forge, S: Splits, W: Works> Pulling<'_, B, R, F, S, W> {
    // The one place the base is decided, so `start` cutting from one branch and
    // `finish` opening the pull request against another cannot happen.
    fn base(&self) -> Result<String, Error> {
        match self.base {
            Some(branch) => Ok(branch.to_owned()),
            None => self.repo.default_branch().map_err(Error::git),
        }
    }

    pub(crate) fn report(&mut self, event: PullEvent) {
        (self.progress)(event);
    }

    /// The whole of one pull: the work, and then — only if every sub-task
    /// finished — the pull request.
    ///
    /// What a door calls. The two halves are separately callable because they
    /// fail differently and are worth driving apart in a test, but nothing
    /// outside this module has a use for a [`Reached`]: a run that stopped is
    /// already recorded and commented, and the only thing left to do with it is
    /// spend its [`status`](Pulled::status).
    pub(crate) fn pull(&mut self, ticket: &Ticket<'_>) -> Result<Pulled, Error> {
        match self.work(ticket)? {
            Reached::Worked { run, touched } => self.finish(ticket, run, &touched),
            Reached::Stopped(ending) => Ok(ending),
        }
    }

    /// One pull, from the ticket to the last sub-task: the run record loaded or
    /// made, the branch checked out or cut, the ticket moved, the ticket split,
    /// and then every sub-task the split ordered, one session at a time.
    ///
    /// It stops where the work stops. What is left afterwards — the push, the
    /// pull request, the review state — needs every sub-task finished and is
    /// [`Reached::Worked`]'s to do; every other ending is a halt this function
    /// has already recorded and commented, which is why the two come back as one
    /// value rather than as an `Ok` and an `Err`. A halt is not a failure: the
    /// branch holds the commits the run did make.
    ///
    /// The clock is read here and is not a seam. Nothing in a run branches on the
    /// time — the three instants are recorded for a person reading `state.json`
    /// afterwards — so a stand-in clock would buy an assertion nobody makes and a
    /// field on every call site.
    pub(crate) fn work(&mut self, ticket: &Ticket<'_>) -> Result<Reached, Error> {
        let mut run =
            match PullRun::find(self.home, self.root, ticket.identifier).map_err(Error::record)? {
                Some(mut held) => {
                    self.take_up(&held)?;
                    // A run whose process stopped without a halt left the
                    // sub-task it was on `in_progress`; it is run again.
                    for reset in held.release_interrupted() {
                        self.report(PullEvent::Repair {
                            note: format!(
                                "`{}` was in progress when the last run stopped, and is \
                                 pending again",
                                reset.id()
                            ),
                        });
                    }
                    held
                }
                None => self.start(ticket)?,
            };
        // Whichever road it came in on. `resumed` is the status of a run waiting
        // to be picked up, and this is the picking up: the brief's rule is that
        // only `resume` moves a run to `resumed` and only a pull moves it on.
        run.set_status(RunStatus::InProgress);

        // Before the split, so the board is honest while the model works — and
        // before the record is first saved, because a team with no such state is
        // a line in the output rather than a thing the record has to remember.
        if !self.move_ticket(ticket.id, IN_PROGRESS)? {
            self.report(PullEvent::NoStartState {
                team: self.scope.team_key().to_owned(),
            });
        }
        // Saved before the split, as Forman saves its state before it
        // decomposes: a run killed while the split is thinking is then a record
        // this machine holds and the next pull picks up, rather than a ticket
        // in `In Progress` with no record that every pull skips as somebody
        // else's.
        self.save(&run)?;

        // A resumed run already holds its split, and splitting it again would
        // spend a session and append a second `.01` beside the first. A held run
        // with no sub-tasks is one whose split halted or never finished, and is
        // owed the split.
        if run.subtasks().is_empty()
            && let Some(reached) = self.split_into(ticket, &mut run)
        {
            return reached;
        }

        self.save(&run)?;
        self.through_subtasks(ticket, run)
    }

    /// The split, with its sub-tasks pushed onto the run, or the halt it ended
    /// in.
    fn split_into(
        &mut self,
        ticket: &Ticket<'_>,
        run: &mut PullRun,
    ) -> Option<Result<Reached, Error>> {
        self.report(PullEvent::Heading(Heading::Split {
            ticket: ticket.identifier.to_owned(),
            title: ticket.title.to_owned(),
        }));
        match self
            .split
            .split(ticket.identifier, ticket.title, ticket.description)
        {
            Split::Subtasks {
                subtasks,
                repairs,
                summary,
            } => {
                for note in repairs {
                    self.report(PullEvent::Repair { note });
                }
                if !summary.is_empty() {
                    run.set_summary(summary);
                }
                for subtask in subtasks {
                    run.push_subtask(PullSubtask::from(subtask));
                }
                None
            }
            // The record is saved all the same, holding a branch and no
            // sub-tasks. Nothing was committed and nothing is lost, but the
            // branch was cut and the ticket was moved, and a halt with no record
            // behind it would leave the next pass calling this ticket "in
            // progress elsewhere" — which is a lie about a run on this very
            // machine. `Unsplit` already writes the sentence that goes on the
            // ticket, so the sub-task account a halt usually posts would have
            // nothing to list.
            Split::Halted(unsplit) => {
                let comment = unsplit.to_string();
                Some(self.halt(ticket, run, &comment, Pulled::halted(ticket)))
            }
        }
    }

    /// Every sub-task the split ordered, serially, in the order
    /// [`next_runnable`] answers.
    ///
    /// The record is saved on both sides of every session, which is what makes a
    /// run killed anywhere resumable: the save before says a sub-task is in
    /// progress, and the save after says what came of it.
    fn through_subtasks(
        &mut self,
        ticket: &Ticket<'_>,
        mut run: PullRun,
    ) -> Result<Reached, Error> {
        let total = run.subtasks().len();
        let mut touched: Vec<TouchedScope> = Vec::new();

        while let Some((id, goal, position)) = next_up(&run) {
            self.report(PullEvent::Heading(Heading::Subtask {
                id: id.clone(),
                goal: goal.clone(),
                position,
                total,
            }));

            let subtask = run.subtask_mut(&id).expect(IN_THE_RUN);
            subtask.set_status(SubtaskStatus::InProgress);
            subtask.set_started_at(now_rfc3339());
            self.save(&run)?;

            let opening = opening_for(&run, ticket, &id);
            let before = self.repo.head().map_err(Error::git)?;
            let worked = self.sessions.work(&opening);
            let after = self.repo.head().map_err(Error::git)?;
            let (answer, stopped) = read(worked);

            run.subtask_mut(&id)
                .expect(IN_THE_RUN)
                .set_finished_at(now_rfc3339());
            if let Some(answer) = &answer {
                record_answer(&mut run, &id, answer);
            }

            // `HEAD` before the tree, because a session that committed has moved
            // its own work out of the tree: whatever `git status` says next is no
            // longer an account of what that session wrote, so the crossing check
            // would be reading a clean tree and answering "nothing crossed".
            if let Head::Moved { from, to } = Head::between(&before, &after) {
                let moved = format!(
                    "the session committed. `HEAD` moved from {} to {}, so what this sub-task \
                     wrote is in a commit warlock did not make and cannot check. Nothing further \
                     was committed.",
                    from.short(),
                    to.short(),
                );
                run.subtask_mut(&id)
                    .expect(IN_THE_RUN)
                    .set_status(SubtaskStatus::Failed(moved));
                let comment = halt_comment(&run);
                return self.halt(ticket, &mut run, &comment, Pulled::halted(ticket));
            }

            // Whatever the session reported, and before any commit: the gate hook
            // never sees a `sed` inside a `Bash`, so this is the only check that
            // covers every way a byte reaches the tree.
            let mut changed = Vec::new();
            let crossings = crossings_after(
                self.repo,
                &mut changed,
                self.root,
                self.manifest,
                self.held,
                Some(self.scope.name()),
            )
            .map_err(Error::git)?;

            if !crossings.crossed.is_empty() {
                let crossed = crossed_reason(&crossings.crossed);
                run.subtask_mut(&id)
                    .expect(IN_THE_RUN)
                    .set_status(SubtaskStatus::Crossed(crossed));
                let comment = halt_comment(&run);
                // The tree is left exactly as the session left it: no commit, no
                // reset, no stash. The work is under a boundary this machine does
                // not hold, and what happens to it is a person's to decide.
                return self.halt(
                    ticket,
                    &mut run,
                    &comment,
                    Pulled::Crossed {
                        ticket: ticket.identifier.to_owned(),
                        subtask: id,
                    },
                );
            }

            // Kept across the sessions rather than read again at the end: every
            // commit below empties the tree these paths were read from, so by the
            // time the pull request body wants them there is nothing left to ask.
            remember(&mut touched, &crossings);
            let wrote = !changed.is_empty();

            match stopped {
                // A session that reported itself done and left nothing in the
                // tree is still done — a sub-task can finish by finding that the
                // work is already there — and `git commit` with nothing staged
                // refuses, so a commit here would end a run over an empty diff.
                None => {
                    let subtask = run.subtask_mut(&id).expect(IN_THE_RUN);
                    if wrote {
                        let message = commit_message(ticket.identifier, &id, &goal);
                        self.repo.commit_all(&message).map_err(Error::git)?;
                    } else {
                        subtask.set_wrote_nothing();
                    }
                    subtask.set_status(SubtaskStatus::Done);
                }
                // Nothing is committed and nothing is undone. The next sibling
                // runs in the tree this one left, which is what
                // [`working_retry`](crate::claude::working_retry) tells a session
                // about.
                Some(status) => {
                    run.subtask_mut(&id).expect(IN_THE_RUN).set_status(status);
                }
            }
            self.save(&run)?;
        }

        // Nothing runnable is two endings: every sub-task finished, or something
        // stopped and everything left waits on it.
        if run
            .subtasks()
            .iter()
            .all(|subtask| *subtask.status() == SubtaskStatus::Done)
        {
            return Ok(Reached::Worked { run, touched });
        }

        let comment = halt_comment(&run);
        self.halt(ticket, &mut run, &comment, Pulled::halted(ticket))
    }

    /// Everything after the last sub-task's commit: the push, the pull request,
    /// the URL on the ticket and in the record, and the ticket moved to the
    /// scope's review state.
    ///
    /// The order is the promise, and every step of it is one a later step reads:
    /// the pull request is opened from what was pushed, and the record is
    /// written before the ticket is told anything. The board comes last for the
    /// reason [`halt`](Self::halt) puts it last — a ticket carrying a URL that
    /// `state.json` does not hold is a pull request the next invocation would
    /// open a second time.
    ///
    /// No `gh` on the machine is a finish all the same. The body is commented on
    /// the ticket instead, the record keeps a `pr_url` of `null`, and the ticket
    /// still moves: the work is done and pushed, and what is missing is a
    /// program, not a step of the run.
    pub(crate) fn finish(
        &mut self,
        ticket: &Ticket<'_>,
        mut run: PullRun,
        touched: &[TouchedScope],
    ) -> Result<Pulled, Error> {
        // Asked again rather than carried from `start`: a resumed run never
        // called it, and the branch a pull request merges into is not a thing to
        // guess at from a record written on another day.
        let base = self.base()?;

        // Asked of the branch rather than of this invocation's sessions, so a
        // resumed run whose commits were made on an earlier day still counts
        // them.
        if self
            .repo
            .changed_against(&base)
            .map_err(Error::git)?
            .is_empty()
        {
            return self.unchanged(ticket, run, &base);
        }

        self.report(PullEvent::Heading(Heading::PullRequest {
            branch: run.branch().to_owned(),
        }));

        self.repo.publish(run.branch()).map_err(Error::git)?;

        let title = pull_request_title(ticket.identifier, ticket.title);
        let body = pull_request_body(
            run.summary().unwrap_or_default(),
            &finished_in(&run),
            &scopes_in(touched),
        );
        let opened = self
            .forge
            .open_pull_request(PullRequest {
                base: &base,
                head: run.branch(),
                title: &title,
                body: &body,
            })
            .map_err(Error::git)?;

        run.set_status(RunStatus::InReview);
        if let Some(url) = opened.url() {
            run.set_pr_url(url);
        }
        self.save(&run)?;

        let comment = match opened.url() {
            Some(url) => format!(
                "`{}` is pushed and its pull request is open: {url}",
                run.branch()
            ),
            None => without_gh(run.branch(), &body),
        };
        self.board
            .comment_on_ticket(ticket.id, &comment)
            .map_err(Error::board)?;

        self.move_to_review(ticket)?;

        Ok(Pulled::Opened {
            ticket: ticket.identifier.to_owned(),
            url: opened.url().map(ToOwned::to_owned),
        })
    }

    /// Every sub-task finished and the branch holds no change: the work was
    /// already there. Nothing is pushed, since `gh` refuses a pull request with
    /// no commits, and the ticket still moves to review with what the sessions
    /// found, because closing it is a person's call and not warlock's.
    fn unchanged(
        &mut self,
        ticket: &Ticket<'_>,
        mut run: PullRun,
        base: &str,
    ) -> Result<Pulled, Error> {
        run.set_status(RunStatus::InReview);
        self.save(&run)?;

        self.board
            .comment_on_ticket(ticket.id, &unchanged_comment(base, &finished_in(&run)))
            .map_err(Error::board)?;

        self.move_to_review(ticket)?;

        Ok(Pulled::Unchanged {
            ticket: ticket.identifier.to_owned(),
        })
    }

    /// A run this checkout already holds, picked up where it was left: its own
    /// branch checked out, and the tree it left required to be clean.
    ///
    /// The clean tree is the whole point of the refusal. A halted sub-task's
    /// uncommitted work is still in that tree, and a run that carried on over it
    /// would fold somebody else's half-finished edit into the next sub-task's
    /// commit under that sub-task's message. Refusing is what gets a human to
    /// look, and it commits nothing on the way out.
    fn take_up(&mut self, run: &PullRun) -> Result<(), Error> {
        self.repo.switch_to(run.branch()).map_err(Error::git)?;
        let dirty = self.repo.dirty().map_err(Error::git)?;
        if dirty.is_empty() {
            Ok(())
        } else {
            Err(Error::Dirty {
                branch: run.branch().to_owned(),
                dirty,
            })
        }
    }

    /// A ticket with no run on this machine: the branch cut from the base, and
    /// the record that will hold everything after it.
    ///
    /// The base is switched to before it is caught up, and that order is not
    /// incidental: [`Repository::catch_up`] runs a `git pull --ff-only`, which
    /// merges into whatever is checked out, so pulling the base from somewhere
    /// else would fast-forward the wrong ref.
    fn start(&mut self, ticket: &Ticket<'_>) -> Result<PullRun, Error> {
        let base = self.base()?;
        self.repo.switch_to(&base).map_err(Error::git)?;
        self.repo.catch_up(&base).map_err(Error::git)?;

        let branch = branch_name(self.scope.team_key(), ticket.number, ticket.title);
        self.repo.cut_branch(&branch, &base).map_err(Error::git)?;

        Ok(PullRun::new(
            ticket.identifier,
            ticket.title,
            self.scope.name(),
            branch,
            now_rfc3339(),
        ))
    }

    fn move_to_review(&mut self, ticket: &Ticket<'_>) -> Result<(), Error> {
        let review = self.scope.review_state();
        if !self.move_ticket(ticket.id, review)? {
            self.report(PullEvent::NoReviewState {
                team: self.scope.team_key().to_owned(),
                state: review.to_owned(),
            });
            return Ok(());
        }
        if let Some(line) = project_review(self.board, ticket.id, review) {
            self.report(PullEvent::Project { line });
        }
        Ok(())
    }

    /// The ticket moved to the team's state of that name, or `false` when the
    /// team has no such state.
    ///
    /// `false` and not an error, for both callers: a board whose workflow nobody
    /// has given an `In Progress` or a review state is a board warlock reports on
    /// and works past, and a run that stopped over it would be the board wagging
    /// the pull.
    fn move_ticket(&self, issue: &str, state: &str) -> Result<bool, Error> {
        // The scope record holds the team key and `workflow_state` filters on
        // the team id, so the key is resolved first. Linear refuses a key where
        // it wants an id, and the stand-in board takes either.
        let Some(team) = self
            .board
            .team_id(self.scope.team_key())
            .map_err(Error::board)?
        else {
            return Ok(false);
        };
        let found = self
            .board
            .workflow_state(&team, state)
            .map_err(Error::board)?;
        let Some(state) = found else {
            return Ok(false);
        };
        self.board
            .move_ticket(issue, &state)
            .map_err(Error::board)?;
        Ok(true)
    }

    /// Every halt goes through here, and the order of the three things it does is
    /// the promise: the record is written first, then the ticket is commented,
    /// and the ticket is not moved at all.
    ///
    /// The record first because it is the source of truth — a comment naming
    /// sub-tasks a `state.json` that failed to write does not agree with is a
    /// comment about a run nobody can resume. The ticket is left where it is
    /// because a halt is warlock stopping, not the work going backwards, and one
    /// comment because the comment is the account of the whole run.
    fn halt(
        &mut self,
        ticket: &Ticket<'_>,
        run: &mut PullRun,
        comment: &str,
        ending: Pulled,
    ) -> Result<Reached, Error> {
        run.set_status(RunStatus::Halted);
        self.save(run)?;
        self.board
            .comment_on_ticket(ticket.id, comment)
            .map_err(Error::board)?;
        Ok(Reached::Stopped(ending))
    }

    /// The record, and everything rendered beside it, under the home this pull
    /// was built with — never under the repository.
    fn save(&self, run: &PullRun) -> Result<(), Error> {
        run.save(self.home, self.root).map_err(Error::record)
    }
}

/// What [`Pulling::work`] leaves behind, in the two shapes the rest of a pull
/// cares about.
///
/// Not a `Result`, because neither of these is a failure: a halt is a run that
/// did as much as it could, wrote its record and said so on the ticket. The
/// [`Error`] beside it is for the failures — a `git` that refused, a board that
/// would not answer, a record that would not write.
#[derive(Debug)]
pub(crate) enum Reached {
    /// Every sub-task finished and committed. The record is `in_progress`, the
    /// branch holds one commit per sub-task, and what is left is the push and the
    /// pull request.
    Worked {
        run: PullRun,
        /// The open scopes the sessions wrote under, other
        /// than the one the ticket was pulled under — which the pull request body
        /// names. Owned and accumulated as the run went, because each commit
        /// empties the tree the paths were read from.
        touched: Vec<TouchedScope>,
    },
    /// The run stopped. The record is `halted`, the ticket carries one comment
    /// saying what finished and what did not, and the ticket stays in progress
    /// rather than moving to review.
    Stopped(Pulled),
}

/// One open scope a session wrote under, with the paths written
/// under it.
///
/// The owned twin of [`Touched`], which borrows the `git
/// status` entries it was read from. Those entries are gone by the next
/// sub-task — the commit that ends this one empties the tree — so a run that
/// wants to name these scopes in a pull request body at the end has to have kept
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TouchedScope {
    pub(crate) scope: String,
    pub(crate) paths: Vec<String>,
}

/// The ticket one pull works, in what a run needs of it and nothing else.
///
/// Values rather than a [`QueuedTicket`](crate::board::QueuedTicket), because the
/// number is wanted as a number, which [`branch_name`] takes. Whoever chose the
/// ticket supplies them, and this module asks the board for nothing about the
/// ticket it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ticket<'a> {
    /// The board's own identifier for the issue, which is what a move and a
    /// comment are addressed to. Never the `WAR-140` sort.
    pub(crate) id: &'a str,
    /// `WAR-140`: the run record's directory, the first word of every commit and
    /// the stem every sub-task is numbered from.
    pub(crate) identifier: &'a str,
    /// The number in the identifier, which is the branch name's middle.
    pub(crate) number: u32,
    pub(crate) title: &'a str,
    /// What the ticket says, as the context the split and every sub-task session
    /// are given, as `forman pull` gives them the parent ticket. Empty is a
    /// ticket nobody described.
    pub(crate) description: &'a str,
}

/// What stopped a pull short of an ending of its own.
///
/// Three of the four are somebody else's failure carried whole, so the sentence a
/// door prints is the one the module that failed wrote. The fourth is the one
/// refusal this module makes itself, and it is a refusal rather than a halt
/// because nothing has happened yet: no session has run, so there is nothing to
/// record and nothing to say on the ticket that the tree does not already say.
#[derive(Debug)]
pub enum Error {
    Git {
        source: GitError,
    },
    Board {
        source: LinearError,
    },
    Record {
        source: pulls::Error,
    },
    /// A run was picked up, its branch checked out, and the tree is not clean.
    Dirty {
        branch: String,
        dirty: Vec<Dirty>,
    },
}

impl Error {
    // Named constructors so every call site is `.map_err(Error::git)` rather
    // than a closure spelling the struct field out again.
    fn git(source: GitError) -> Self {
        Self::Git { source }
    }

    fn board(source: LinearError) -> Self {
        Self::Board { source }
    }

    fn record(source: pulls::Error) -> Self {
        Self::Record { source }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // No preamble on the three carried failures: `git.rs`, `linear.rs`
            // and `pulls.rs` each name what they were doing, and a sentence here
            // would say "the pull failed" over the top of the reason.
            Self::Git { source } => write!(f, "{source}"),
            Self::Board { source } => write!(f, "{source}"),
            Self::Record { source } => write!(f, "{source}"),
            Self::Dirty { branch, dirty } => {
                write!(
                    f,
                    "`{branch}` is checked out for this run and its working tree is not clean, so \
                     nothing was committed and no session was raised. What a halted sub-task left \
                     is yours to keep or to drop:",
                )?;
                for entry in dirty {
                    write!(f, "\n  {entry}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Git { source } => Some(source),
            Self::Board { source } => Some(source),
            Self::Record { source } => Some(source),
            Self::Dirty { .. } => None,
        }
    }
}

// The one panic message in the loop, and it says why it cannot happen: every
// identifier reached for is one `next_runnable` just answered with, out of the
// very record being written to.
const IN_THE_RUN: &str = "the sub-task the run just offered is in the run";

/// The splitting session, behind a seam.
///
/// Takes the ticket rather than being built around it, because the ticket is a
/// parameter of the run and the seams are built before one is chosen. One call,
/// one [`Split`], and no `Result`: a session that never answered is already one
/// of [`Split::Halted`]'s endings.
pub(crate) trait Splits {
    fn split(&self, ticket: &str, title: &str, description: &str) -> Split;
}

/// One sub-task session per call.
///
/// A factory and not a session: a run opens one session per sub-task and drops it
/// at the end of the sub-task, so nothing one sub-task said reaches the next —
/// which is what [`working_opening`] is written on
/// the assumption of.
///
/// The retries are the implementation's, not the caller's:
/// [`Working::run`](crate::claude::Working::run) already owns which stopping earns
/// another attempt and what a turn limit does to the next one, and a loop that
/// asked again itself would be a second retry policy on top of that one.
pub(crate) trait Works {
    fn work(&self, opening: &str) -> Worked;
}

/// What a whole pull came to, in the three endings that cost different money.
///
/// The [`status`](Pulled::status) is here rather than worked out at the door for
/// the reason the enum exists: a crossing and a halt both leave a branch with
/// commits on it and a ticket with a comment, and the only thing that tells them
/// apart afterwards is which variant was answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Pulled {
    /// The run reached a pull request. `None` is
    /// [`Opened::NoGh`](crate::git::Opened::NoGh), which is still a run that
    /// did the work: the branch is pushed and the body is on the ticket.
    Opened { ticket: String, url: Option<String> },
    /// Every sub-task finished and the branch changed nothing, so there was no
    /// pull request to open. The ticket is in review with what the sessions
    /// found.
    Unchanged { ticket: String },
    /// Nothing was runnable, and the ticket already carries
    /// [`halt_comment`]'s account of why.
    Halted { ticket: String },
    /// A session wrote under a scope closed on this machine. Named apart
    /// from a halt because the tree still holds that work uncommitted, and
    /// because it is the one halt that is a boundary rather than a failure.
    Crossed { ticket: String, subtask: String },
}

impl Pulled {
    /// What the shell spends on it.
    ///
    /// The numbers are [`status_for`](crate::error::status_for)'s and mean what
    /// they mean there: **0** the question was answered, **1** warlock could not
    /// do it, **3** a boundary closed on this machine. A crossing is a
    /// **3** for that last reason and not because it is worse than a halt.
    ///
    /// Read by the tests rather than by the door, and deliberately so: `main.rs`
    /// spends the status off an [`Error`](crate::error::Error), because a halt and
    /// a crossing are both printed as a line before the process exits. That the
    /// two agree is an assertion in `tests/pull.rs` — this is the statement of
    /// what the loop thinks each ending is worth, held against what the shell
    /// actually returns.
    #[cfg(test)]
    pub(crate) const fn status(&self) -> u8 {
        match self {
            Self::Opened { .. } | Self::Unchanged { .. } => 0,
            Self::Halted { .. } => 1,
            Self::Crossed { .. } => 3,
        }
    }

    /// The ordinary halt, named from the ticket being worked: every halt but the
    /// crossing, which carries the sub-task that crossed as well.
    fn halted(ticket: &Ticket<'_>) -> Self {
        Self::Halted {
            ticket: ticket.identifier.to_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ticket(&self) -> &str {
        match self {
            Self::Opened { ticket, .. }
            | Self::Unchanged { ticket }
            | Self::Halted { ticket }
            | Self::Crossed { ticket, .. } => ticket,
        }
    }
}

/// What a run is seen doing, for whichever door is watching.
///
/// A [`Heading`] opens a section and everything else belongs to the section open
/// at the time, so which events start one is a fact about the type rather than a
/// convention each door has to learn again.
///
/// The two board lines are variants rather than failures because neither is one:
/// a team with no state to move the ticket to has a ticket that does not move,
/// and a run that stopped over it would be the board wagging the pull.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PullEvent {
    Heading(Heading),
    /// The session's own, carried rather than worded: two doors word one
    /// differently — a line on a pipe and a work line on the panel's thread —
    /// and a line rendered in here would be the shell's wording sent to both.
    Activity(Activity),
    /// One repair warlock made to what the split answered, in the engine's own
    /// words. Said rather than swallowed: the sub-tasks a run works are not quite
    /// the ones the session wrote, and a manifest nobody was told had been mended
    /// reads as a manifest the model produced.
    Repair {
        note: String,
    },
    /// The team has no workflow state named
    /// [`IN_PROGRESS`]. No state is carried because
    /// there is only one spelling this looks for; the review state's is the
    /// scope record's, so that one is said.
    NoStartState {
        team: String,
    },
    NoReviewState {
        team: String,
        state: String,
    },
    /// What became of the ticket's project once the ticket reached review,
    /// worded here because both doors say it the same way.
    Project {
        line: String,
    },
}

fn no_start_state(team: &str) -> String {
    format!("the team `{team}` has no `In Progress` state, so the ticket was not moved")
}

fn no_review_state(team: &str, state: &str) -> String {
    format!("the team `{team}` has no `{state}` state, so the ticket was not moved into review")
}

impl PullEvent {
    /// The line both doors say, or `None` for an activity, which each words for
    /// itself.
    pub(crate) fn line(&self) -> Option<String> {
        match self {
            Self::Heading(heading) => Some(heading.line()),
            Self::Activity(_) => None,
            Self::Repair { note } => Some(note.clone()),
            Self::NoStartState { team } => Some(no_start_state(team)),
            Self::NoReviewState { team, state } => Some(no_review_state(team, state)),
            Self::Project { line } => Some(line.clone()),
        }
    }
}

/// The ticket's project moved to the review state when it is `In Progress` —
/// every slice drafted — and each of its issues is in review or closed. `None`
/// when there is nothing to say: no project, a project still being drafted, or
/// an issue still open.
///
/// Every failure is a line, never an error: the ticket is in review by now, and
/// a pull is not undone by the project it belongs to.
fn project_review(board: &impl Board, issue: &str, review: &str) -> Option<String> {
    let project = match board.ticket_brief(issue) {
        Ok(Some(project)) => project,
        Ok(None) => return None,
        Err(error) => {
            return Some(format!(
                "the ticket's project was not read, so it was not moved: {error}"
            ));
        }
    };
    let drafted = project
        .status()
        .is_some_and(|status| same_name(status, IN_PROGRESS));
    if !drafted {
        return None;
    }
    let name = project.name();
    let reviewed = project
        .states()
        .iter()
        .all(|(state, kind)| kind.settled() || same_name(state, review));
    if !reviewed {
        return None;
    }

    let moved = board.brief_status(review).and_then(|status| {
        status
            .map(|status| board.move_brief(project.id(), &status))
            .transpose()
    });
    let every = format!("every issue in `{name}` is in review or closed");
    Some(match moved {
        Ok(Some(_)) => format!("{every}, so the project moved to `{review}`"),
        Ok(None) => format!(
            "{every}, and the workspace has no project status called `{review}`, so the project \
             was not moved"
        ),
        Err(error) => format!("{every}, and the project was not moved to `{review}`: {error}"),
    })
}

/// The three sections a run has, and the whole of what opens one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Heading {
    Split {
        ticket: String,
        title: String,
    },
    /// The fraction is one-based and its denominator is the split's answer: a
    /// sub-task that is re-attempted does not move it, because the run's size is
    /// not a running total.
    Subtask {
        id: String,
        goal: String,
        position: usize,
        total: usize,
    },
    PullRequest {
        branch: String,
    },
}

impl Heading {
    pub(crate) fn line(&self) -> String {
        match self {
            Self::Split { ticket, title } => format!("{ticket} — splitting {title}"),
            Self::Subtask {
                id,
                goal,
                position,
                total,
            } => format!("[{position}/{total}] `{id}` {goal}"),
            Self::PullRequest { branch } => format!("`{branch}` is pushed, opening a pull request"),
        }
    }
}

/// The sub-task to work next: the first [`Pending`](SubtaskStatus::Pending) one
/// every dependency of which is [`Done`](SubtaskStatus::Done), or `None` when
/// nothing is runnable.
///
/// In the record's own order, which is the split's: `number` put every sub-task
/// after everything it waits on, so the first runnable one found walking forward
/// is the one the split meant to come next.
///
/// A `depends_on` naming a sub-task the run does not hold counts as unmet. The
/// record is hand-editable — it is read by a person beside a halted run — and the
/// two readings differ only for a file somebody has mistyped, where refusing to
/// run is a halt the operator is told about and the other road is warlock
/// building on work that was never done.
pub(crate) fn next_runnable(run: &PullRun) -> Option<&PullSubtask> {
    run.subtasks().iter().find(|subtask| {
        *subtask.status() == SubtaskStatus::Pending
            && subtask.depends_on().iter().all(|id| {
                run.subtask(id)
                    .is_some_and(|needed| *needed.status() == SubtaskStatus::Done)
            })
    })
}

/// The sub-task to work next with what the header above its session says: its
/// identifier, its goal and its one-based place in the record.
///
/// Owned, and that is what it is for: the loop is about to write to the very
/// record [`next_runnable`] borrowed from, and the three facts it needs afterwards
/// are these. The position is the sub-task's place in the record rather than a
/// count of sessions, so a run that passed over a blocked sub-task and came back
/// to a later sibling still numbers each one where the manifest has it.
fn next_up(run: &PullRun) -> Option<(String, String, usize)> {
    let next = next_runnable(run)?;
    let at = run
        .subtasks()
        .iter()
        .position(|subtask| subtask.id() == next.id())
        // Found by walking this same list, so it is there. One rather than a
        // panic if it somehow is not: the numerator of a progress header is not
        // worth ending a run over.
        .map_or(1, |at| at + 1);
    Some((next.id().to_owned(), next.goal().to_owned(), at))
}

/// The opening turn of one sub-task's session: its own brief, the ticket as
/// context, and what its finished siblings reported.
///
/// Rendered from the record rather than read from the brief on disk, though the
/// two are the same text at this point: the file also carries whatever the last
/// session appended under its execution log heading, and a session shown its own
/// half-written log would be reading yesterday's notes as today's instructions.
fn opening_for(run: &PullRun, ticket: &Ticket<'_>, id: &str) -> String {
    let brief = run
        .subtask(id)
        .expect(IN_THE_RUN)
        .to_brief_string(ticket.identifier);
    // A sub-task that finished and logged nothing is left out rather than carried
    // as an empty summary: a session shown an id with silence under it reads it as
    // a sibling that finished and had nothing to say.
    let finished: Vec<Sibling<'_>> = run
        .subtasks()
        .iter()
        .filter(|sibling| *sibling.status() == SubtaskStatus::Done)
        .filter_map(|sibling| Some((sibling.id(), sibling.log()?)))
        .collect();
    let notes: Vec<Sibling<'_>> = run
        .subtasks()
        .iter()
        .filter(|sibling| *sibling.status() == SubtaskStatus::Done)
        .filter_map(|sibling| Some((sibling.id(), sibling.review_notes()?)))
        .collect();
    working_opening(&brief, ticket.title, ticket.description, &finished, &notes)
}

fn record_answer(run: &mut PullRun, id: &str, answer: &Accepted) {
    let subtask = run.subtask_mut(id).expect(IN_THE_RUN);
    subtask.set_log(answer.summary());
    if let Some(tested) = answer.tested() {
        subtask.set_tested(tested);
    }
    if let Some(notes) = answer.review_notes() {
        subtask.set_review_notes(notes);
    }
    // Only a finished sibling's note, and never this sub-task's own: the notes
    // on offer were the ones the opening showed.
    for resolved in answer.resolved_notes() {
        if let Some(sibling) = run.subtask_mut(resolved)
            && resolved != id
            && *sibling.status() == SubtaskStatus::Done
        {
            sibling.clear_review_notes();
        }
    }
}

/// One session's outcome as the record takes it: its answer, to log, and the
/// status to set — where `None` is a session that reported itself done.
///
/// `None` is not "no status": it is the one claim the session does not get to
/// settle. What the working tree holds decides a `done`, so the status for it is
/// set after the check rather than here.
///
/// A session that never answered is `failed` carrying what stopped it. Nothing is
/// retried here — [`Works`] has spent every attempt by the time this is read, and
/// a second policy on top of that one would be warlock asking twice as patiently
/// as it decided to.
fn read(worked: Worked) -> (Option<Accepted>, Option<SubtaskStatus>) {
    match worked {
        Worked::Answered(accepted) => {
            let stopped = match accepted.reported() {
                Reported::Done => None,
                Reported::Blocked(reason) => Some(SubtaskStatus::Blocked(reason.to_owned())),
                Reported::Failed(reason) => Some(SubtaskStatus::Failed(reason.to_owned())),
            };
            (Some(accepted), stopped)
        }
        Worked::Halted(why) => (None, Some(SubtaskStatus::Failed(why.to_string()))),
    }
}

/// What a `crossed` sub-task's reason says: every path the session wrote under
/// a scope closed on this machine or under no scope, each with the scope that
/// covers it or the fact that none does.
///
/// Every path and not the first, because the operator's next move is to look at
/// all of them, and the scope on each because two crossed paths are commonly under
/// two different scopes.
fn crossed_reason(crossed: &[Crossing<'_>]) -> String {
    let mut reason =
        String::from("wrote outside the scopes open on this machine, so nothing was committed:");
    for crossing in crossed {
        let _ = match crossing.scope {
            Some(scope) => write!(reason, " `{}` is scoped `{scope}`;", crossing.path),
            None => write!(reason, " `{}` is under no scope;", crossing.path),
        };
    }
    // The last separator is a full stop, so the reason reads as a sentence
    // wherever it is quoted — a halt comment, a manifest line, a `state.json`
    // somebody is reading by hand.
    if reason.ends_with(';') {
        reason.pop();
        reason.push('.');
    }
    reason
}

/// Fold one session's open-but-foreign scopes into what the run has seen, with no
/// scope and no path said twice.
///
/// Two sessions touching one scope is one entry in the pull request body and not
/// two, and the order is the order the paths were first seen, following
/// [`crossings_in`](crate::crossings::crossings_in)'s own promise about order.
fn remember(kept: &mut Vec<TouchedScope>, crossings: &crate::crossings::Crossings<'_>) {
    for foreign in &crossings.touched {
        let at = if let Some(at) = kept.iter().position(|held| held.scope == foreign.scope) {
            at
        } else {
            kept.push(TouchedScope {
                scope: foreign.scope.to_owned(),
                paths: Vec::new(),
            });
            kept.len() - 1
        };
        for path in &foreign.paths {
            if !kept[at].paths.iter().any(|held| held == path) {
                kept[at].paths.push((*path).to_owned());
            }
        }
    }
}

/// Every finished sub-task as the pull request body names it, in the record's
/// order.
///
/// A sub-task that finished and logged nothing is still named: a run's shape is
/// its sub-tasks, and a reviewer reading a body with one of them missing would
/// go looking for the commit it does not explain. The empty summary is what
/// [`pull_request_body`] already leaves out.
pub(crate) fn unchanged_comment(base: &str, finished: &[Finished<'_>]) -> String {
    let mut comment = format!(
        "No change was needed: the work this ticket asks for is already on `{base}`, so nothing \
         was pushed and no pull request was opened. What each sub-task found:\n"
    );
    for done in finished {
        let _ = write!(comment, "\n- `{}` {}", done.id, done.summary.trim());
    }
    comment
}

fn finished_in(run: &PullRun) -> Vec<Finished<'_>> {
    run.subtasks()
        .iter()
        .filter(|subtask| *subtask.status() == SubtaskStatus::Done)
        .map(|subtask| Finished {
            id: subtask.id(),
            goal: subtask.goal(),
            summary: subtask.log().unwrap_or_default(),
            tested: subtask.tested(),
            review_notes: subtask.review_notes(),
            wrote: !subtask.wrote_nothing(),
        })
        .collect()
}

fn scopes_in(touched: &[TouchedScope]) -> Vec<Touched<'_>> {
    touched
        .iter()
        .map(|held| Touched {
            scope: &held.scope,
            paths: held.paths.iter().map(String::as_str).collect(),
        })
        .collect()
}

/// What goes on the ticket when the machine that worked it has no `gh`: the body
/// the pull request would have carried, under a sentence saying why it is here
/// and naming the branch to open one from.
///
/// The whole body and not a summary of it. This is the only place that account
/// of the run exists — there is no pull request to hold it — and a reviewer
/// opening the request by hand is the person it was written for.
fn without_gh(branch: &str, body: &str) -> String {
    format!(
        "There is no `gh` on the machine that worked this ticket, so no pull request was opened. \
         `{branch}` is pushed and holds one commit per sub-task, and what the pull request would \
         have said is below.\n\n{body}"
    )
}

/// The one comment a halted run leaves on its ticket: what finished, what
/// stopped and why, what never started, and the two commands that carry the run
/// on.
///
/// The two commands are both said, in order, because neither does the other's
/// job: `resume` is the human saying they have looked, and it puts the stopped
/// sub-tasks back to `pending` without working any of them; `pull --ticket` is
/// what works them. A comment naming only the first would leave a released run
/// waiting for a queue pass that will not choose it ahead of anything.
///
/// A section with nothing in it is absent rather than an empty heading, as
/// [`pull_request_body`] leaves one out: a run
/// that halted on its first sub-task has nothing finished, and a heading saying
/// so is the bulk of the comment.
pub(crate) fn halt_comment(run: &PullRun) -> String {
    let mut comment = format!(
        "This pull halted, so the ticket stays in progress and was not moved to review. The \
         branch `{}` holds one commit per finished sub-task and nothing else was committed.",
        run.branch()
    );

    section(&mut comment, "Finished", run, |status| {
        *status == SubtaskStatus::Done
    });
    // Everything that is neither finished nor waiting, which is `blocked`,
    // `failed`, `crossed` — and `in_progress`, the status a run killed mid-session
    // leaves behind. That one has no reason to give, and it belongs here anyway:
    // a sub-task warlock started and cannot account for is a thing to look at,
    // not a thing to list as never started.
    section(&mut comment, "Stopped", run, |status| {
        !matches!(status, SubtaskStatus::Done | SubtaskStatus::Pending)
    });
    section(&mut comment, "Not started", run, |status| {
        *status == SubtaskStatus::Pending
    });

    let _ = write!(
        comment,
        "\n\n`warlock resume {ticket}` puts the stopped sub-tasks back, and then \
         `warlock pull {scope} --ticket {ticket}` works the ticket again.",
        ticket = run.ticket(),
        scope = run.scope(),
    );
    comment
}

fn section(
    comment: &mut String,
    heading: &str,
    run: &PullRun,
    wanted: impl Fn(&SubtaskStatus) -> bool,
) {
    let listed: Vec<&PullSubtask> = run
        .subtasks()
        .iter()
        .filter(|subtask| wanted(subtask.status()))
        .collect();
    if listed.is_empty() {
        return;
    }

    let _ = write!(comment, "\n\n## {heading}");
    for subtask in listed {
        let _ = write!(
            comment,
            "\n\n- `{}` {}",
            subtask.id(),
            subtask.goal().trim()
        );
        // The status is spelled on the stopped lines alone. On a finished line it
        // would be the heading said twice, and on a not-started line `pending` is
        // what "not started" means.
        let status = subtask.status();
        if !matches!(status, SubtaskStatus::Done | SubtaskStatus::Pending) {
            let _ = write!(comment, " — `{}`", status.as_str());
            if let Some(reason) = status.reason() {
                let _ = write!(comment, ": {}", reason.trim());
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/pulling.rs"]
mod tests;
