use crate::pulls::SubtaskStatus;

use super::{
    Accepted, RESULT_PROMPT, Reported, Unreadable, accept, objects, stub_answer, stub_reply,
};

const SUMMARY: &str = "Added the result contract beside drafting.rs and its tests. Left the \
                       session plumbing alone.";

const REASON: &str = "`crates/control` is scoped control-plane and this machine holds no sigil \
                      for it";

fn done(reply: &str) -> Accepted {
    let accepted = accept(reply);
    assert_eq!(
        accepted.unreadable(),
        None,
        "expected a readable reply, got {accepted:?}"
    );
    accepted
}

#[test]
fn a_clean_done_is_read_as_done_with_its_summary() {
    let accepted = done(&stub_answer(SUMMARY));

    assert_eq!(accepted.reported(), &Reported::Done);
    assert_eq!(accepted.summary(), SUMMARY);
    assert_eq!(accepted.reported().reason(), None);
    assert_eq!(
        SubtaskStatus::from(accepted.reported().clone()),
        SubtaskStatus::Done
    );
}

#[test]
fn a_blocked_with_a_reason_keeps_the_reason() {
    let accepted = done(&stub_reply("blocked", SUMMARY, Some(REASON)));

    assert_eq!(accepted.reported(), &Reported::Blocked(REASON.to_owned()));
    assert_eq!(accepted.summary(), SUMMARY);
    assert_eq!(
        SubtaskStatus::from(accepted.reported().clone()),
        SubtaskStatus::Blocked(REASON.to_owned()),
        "the record on disk spells the same reason under the same key"
    );
}

#[test]
fn a_failed_with_no_reason_falls_to_its_own_summary() {
    let accepted = done(&stub_reply("failed", SUMMARY, None));

    assert_eq!(
        accepted.reported(),
        &Reported::Failed(SUMMARY.to_owned()),
        "the record holds no reasonless failure, so the summary is the reason"
    );
    assert_eq!(accepted.summary(), SUMMARY);
}

#[test]
fn a_failed_with_a_reason_and_no_summary_still_has_a_reason() {
    let accepted = done(&stub_reply(
        "failed",
        "",
        Some("`cargo test` did not finish"),
    ));

    assert_eq!(
        accepted.reported(),
        &Reported::Failed("`cargo test` did not finish".to_owned())
    );
    assert_eq!(
        accepted.summary(),
        "",
        "a thin summary is recorded as it stands, not filled in"
    );
}

#[test]
fn a_failed_that_says_nothing_at_all_falls_to_warlocks_own_line() {
    let accepted = done(r#"{"status": "failed", "summary": "  ", "blocked_reason": null}"#);

    let Reported::Failed(reason) = accepted.reported() else {
        panic!("expected a failure, got {accepted:?}");
    };
    assert!(
        reason.contains("said nothing further"),
        "warlock supplies the reason it cannot leave empty: {reason}"
    );
}

#[test]
fn prose_after_the_object_does_not_hide_it() {
    let reply = format!(
        "{}\n\nThat is everything the sub-task asked for.",
        stub_answer(SUMMARY)
    );

    let accepted = done(&reply);

    assert_eq!(accepted.reported(), &Reported::Done);
    assert_eq!(
        accepted.reply(),
        reply,
        "the message is kept verbatim, trailing prose and all"
    );
}

#[test]
fn prose_before_the_object_does_not_hide_it_either() {
    let reply = format!(
        "I edited two files and ran the tests.\n\n{}",
        stub_reply("blocked", SUMMARY, Some(REASON))
    );

    let accepted = done(&reply);

    assert_eq!(accepted.reported(), &Reported::Blocked(REASON.to_owned()));
}

#[test]
fn the_last_object_wins() {
    let reply = format!(
        "A first answer I then corrected:\n{}\nand the real one:\n{}",
        stub_reply("blocked", "An earlier answer.", Some(REASON)),
        stub_answer(SUMMARY),
    );

    let accepted = done(&reply);

    assert_eq!(accepted.reported(), &Reported::Done);
    assert_eq!(accepted.summary(), SUMMARY);
}

// A brace left open in prose outside the object would swallow the answer into a
// span that cannot parse, so an earlier candidate is tried rather than the whole
// reply being called unreadable.
#[test]
fn an_unbalanced_brace_in_the_prose_does_not_swallow_the_object() {
    let reply = format!("{}\n\nI also touched `fn main() {{`.", stub_answer(SUMMARY));

    let accepted = done(&reply);

    assert_eq!(accepted.reported(), &Reported::Done);
}

#[test]
fn a_brace_inside_a_string_is_not_an_object() {
    let accepted =
        done(r#"{"status": "done", "summary": "Wrote `impl Trait for Thing {}` once."}"#);

    assert_eq!(accepted.reported(), &Reported::Done);
    assert_eq!(accepted.summary(), "Wrote `impl Trait for Thing {}` once.");
}

#[test]
fn no_object_at_all_is_a_failure_keeping_the_reply() {
    const REPLY: &str = "I finished the sub-task. Everything builds and the tests pass.";

    let accepted = accept(REPLY);

    assert_eq!(accepted.unreadable(), Some(&Unreadable::NoObject));
    assert_eq!(accepted.reply(), REPLY, "the prose is kept verbatim");
    assert_eq!(
        accepted.reported().as_str(),
        "failed",
        "an unreadable answer says nothing about whether the work was done"
    );
    assert!(
        accepted.summary().starts_with("Warlock could not read"),
        "the stand-in summary opens by saying warlock wrote it: {}",
        accepted.summary()
    );
}

#[test]
fn malformed_json_is_a_failure_carrying_serdes_own_detail() {
    const REPLY: &str = r#"{"status": "done", "summary": "Finished.",}"#;

    let accepted = accept(REPLY);

    let Some(Unreadable::NotJson { detail }) = accepted.unreadable() else {
        panic!("expected malformed JSON, got {accepted:?}");
    };
    assert!(
        detail.contains("line 1"),
        "serde's message says where the object went wrong: {detail}"
    );
    assert_eq!(accepted.reply(), REPLY);
    assert_eq!(
        accepted.reported(),
        &Reported::Failed(accepted.unreadable().expect("unreadable").to_string()),
        "the reason a caller records is the reason the object could not be read"
    );
}

// A session cut off mid-object is reported as an object that would not read,
// not as a message with no object in it: what happened is visible in the detail.
#[test]
fn an_object_cut_off_mid_answer_is_malformed_rather_than_absent() {
    let accepted = accept(r#"{"status": "done", "summary": "Added the res"#);

    let Some(Unreadable::NotJson { detail }) = accepted.unreadable() else {
        panic!("expected malformed JSON, got {accepted:?}");
    };
    assert!(
        detail.contains("EOF"),
        "the detail says the object never ended: {detail}"
    );
}

#[test]
fn an_unknown_status_is_a_failure_naming_what_was_said() {
    let accepted = accept(&stub_reply("complete", SUMMARY, None));

    assert_eq!(
        accepted.unreadable(),
        Some(&Unreadable::UnknownStatus {
            status: "complete".to_owned()
        })
    );
    assert_eq!(accepted.reported().as_str(), "failed");
}

// `crossed` is warlock's own verdict from the post-session check of the tree,
// and a session cannot award or refuse it to itself.
#[test]
fn crossed_is_not_a_status_a_session_may_report() {
    let accepted = accept(&stub_reply(
        "crossed",
        SUMMARY,
        Some("wrote crates/control"),
    ));

    assert_eq!(
        accepted.unreadable(),
        Some(&Unreadable::UnknownStatus {
            status: "crossed".to_owned()
        })
    );
}

#[test]
fn a_status_written_in_another_case_is_read_as_the_status_it_names() {
    let accepted = done(&stub_reply("Done", SUMMARY, None));

    assert_eq!(accepted.reported(), &Reported::Done);
}

#[test]
fn a_blocked_with_no_reason_is_a_failure() {
    for reply in [
        stub_reply("blocked", SUMMARY, None),
        stub_reply("blocked", SUMMARY, Some("   ")),
        r#"{"status": "blocked", "summary": "Stopped."}"#.to_owned(),
    ] {
        let accepted = accept(&reply);

        assert_eq!(
            accepted.unreadable(),
            Some(&Unreadable::BlockedWithNoReason),
            "a block with nothing to say is not recorded as a block: {reply}"
        );
        assert_eq!(accepted.reported().as_str(), "failed");
    }
}

#[test]
fn a_key_the_contract_does_not_name_does_not_throw_the_answer_away() {
    let accepted =
        done(r#"{"status": "done", "summary": "Finished.", "blocked_reason": null, "turns": 14}"#);

    assert_eq!(accepted.reported(), &Reported::Done);
}

#[test]
fn a_reason_beside_a_done_is_dropped_rather_than_refused() {
    let accepted = done(&stub_reply("done", SUMMARY, Some("left over from an edit")));

    assert_eq!(accepted.reported(), &Reported::Done);
    assert_eq!(accepted.reported().reason(), None);
}

#[test]
fn every_status_and_key_the_contract_names_is_in_the_prompt() {
    for named in [
        "\"status\"",
        "\"summary\"",
        "\"blocked_reason\"",
        "\"done\"",
        "\"blocked\"",
        "\"failed\"",
    ] {
        assert!(
            RESULT_PROMPT.contains(named),
            "the prompt states the shape it asks for: {named} is missing"
        );
    }
}

// The prompt's own example object is read by the parser it describes, so the
// prose and the reader cannot drift into two different contracts.
#[test]
fn the_shape_the_prompt_states_is_the_shape_the_parser_reads() {
    let stated = objects(RESULT_PROMPT)
        .first()
        .map(|object| {
            object
                .replace("\"done\" | \"blocked\" | \"failed\"", "\"done\"")
                .replace("null | \"...\"", "null")
        })
        .expect("the prompt shows the object it asks for");

    let accepted = done(&stated);

    assert_eq!(accepted.reported(), &Reported::Done);
}

#[test]
fn a_reported_prints_its_status_and_reason() {
    assert_eq!(Reported::Done.to_string(), "done");
    assert_eq!(
        Reported::Blocked(REASON.to_owned()).to_string(),
        format!("blocked: {REASON}")
    );
    assert_eq!(
        Reported::Failed("the tests did not run".to_owned()).to_string(),
        "failed: the tests did not run"
    );
}
