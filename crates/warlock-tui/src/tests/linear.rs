use std::error::Error as _;
use std::io;

use serde_json::{Value, json};

use super::{
    Assignee, BACKLOG, BLOCKERS_PAGE, Blocker, Board, CUT_NOTE, Client, ENDPOINT, Error,
    LABELS_PAGE, Linear, NamedIssue, NewIssue, NewProject, Posts, Priority, QUEUE_PAGE,
    QueuedIssue, REQUEST_TIMEOUT, SKIP_NOTE, StateType, answer, authorization, backlog_state,
    backlog_status, comment_on_issue, comment_on_project, create_issue, create_project,
    create_relation, fetch_project, issue_label_id, label_id, move_issue, move_project,
    named_issue, planned_projects, project_named, project_status, scope_queue, team_id, viewer,
    workflow_state,
};

use crate::queue::IN_PROGRESS;
use crate::stubs::Posting;

const KEY: &str = "lin_api_a_key_nobody_holds_8f3a1c";

fn variants() -> Vec<Error> {
    vec![
        Error::Key,
        Error::Transport {
            source: ureq::Error::Io(io::Error::other("connection reset by peer")),
        },
        Error::Status { code: 401 },
        Error::Malformed {
            detail: "it was not JSON".to_owned(),
        },
        Error::Refused {
            message: "Entity not found".to_owned(),
        },
    ]
}

#[test]
fn one_timeout_of_thirty_seconds_covers_the_whole_call() {
    assert_eq!(
        REQUEST_TIMEOUT.as_secs(),
        30,
        "thirty seconds, per Linear call"
    );
    // The global timeout, which is the one that covers resolve, connect,
    // handshake, send and body rather than any single phase of them.
    assert_eq!(Client::new(KEY).timeout(), Some(REQUEST_TIMEOUT));
}

#[test]
fn the_endpoint_is_linears_one_graphql_url() {
    assert_eq!(ENDPOINT, "https://api.linear.app/graphql");
}

#[test]
fn the_key_is_a_parameter_and_the_client_never_prints_it() {
    let client = Client::new(KEY);

    let printed = format!("{client:?}");

    assert!(
        !printed.contains(KEY),
        "the key reached `Debug` output: {printed}"
    );
}

#[test]
fn no_error_variant_prints_the_key() {
    for error in variants() {
        let (shown, printed) = (error.to_string(), format!("{error:?}"));

        assert!(!shown.contains(KEY), "the key reached `Display`: {shown}");
        assert!(!printed.contains(KEY), "the key reached `Debug`: {printed}");
    }
}

#[test]
fn only_a_transport_failure_has_a_source_under_it() {
    for error in variants() {
        let expected = matches!(error, Error::Transport { .. });

        assert_eq!(error.source().is_some(), expected, "{error:?}");
    }
}

#[test]
fn the_authorization_value_is_the_bare_key() {
    let value = authorization(KEY).expect("an ordinary key is a header value");

    let sent = value.to_str().expect("an ASCII key is a readable value");

    // Not `Bearer <key>`: that is what an OAuth token wants, and it is a 401
    // for a personal API key.
    assert_eq!(sent, KEY);
    assert!(!sent.contains("Bearer"));
    // And marked sensitive, so a header map that gets printed prints
    // `Sensitive` where the key is.
    assert!(value.is_sensitive());
    assert!(!format!("{value:?}").contains(KEY));
}

#[test]
fn a_key_that_cannot_be_a_header_value_is_refused_without_quoting_it() {
    let error = authorization(&format!("{KEY}\nX-Injected: yes"))
        .expect_err("a newline cannot go in a header value");

    assert!(matches!(error, Error::Key));
    assert!(!error.to_string().contains(KEY));
}

#[test]
fn the_data_object_is_what_comes_back() {
    let body = json!({ "data": { "team": { "id": "team-1" } } });

    let data = answer(body).expect("a `data` object is an answer");

    assert_eq!(data, json!({ "team": { "id": "team-1" } }));
}

#[test]
fn a_graphql_errors_array_is_a_refusal_in_linears_words() {
    let body = json!({
        "data": null,
        "errors": [{ "message": "Entity not found" }, { "message": "and another" }],
    });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert!(
        matches!(&error, Error::Refused { message } if message == "Entity not found"),
        "{error:?}"
    );
}

#[test]
fn a_validation_refusal_carries_the_reason_linear_gives_a_person() {
    let body = json!({
        "data": null,
        "errors": [{
            "message": "Argument Validation Error",
            "extensions": {
                "userPresentableMessage": "name must be shorter than or equal to 80 characters.",
            },
        }],
    });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert_eq!(
        error.to_string(),
        "Linear refused the request: Argument Validation Error: name must be shorter than or \
         equal to 80 characters."
    );
}

#[test]
fn a_presentable_message_that_repeats_the_message_is_not_said_twice() {
    let body = json!({
        "data": null,
        "errors": [{
            "message": "Entity not found",
            "extensions": { "userPresentableMessage": "Entity not found" },
        }],
    });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert!(
        matches!(&error, Error::Refused { message } if message == "Entity not found"),
        "{error:?}"
    );
}

#[test]
fn an_errors_entry_with_no_message_still_refuses() {
    let body = json!({ "data": null, "errors": [{ "extensions": {} }] });

    let error = answer(body).expect_err("an `errors` array is a refusal");

    assert!(matches!(&error, Error::Refused { .. }), "{error:?}");
}

#[test]
fn an_answer_with_no_data_is_malformed() {
    for body in [json!({}), json!({ "data": null }), json!("not an object")] {
        let error = answer(body.clone()).expect_err("no `data` is no answer");

        assert!(
            matches!(&error, Error::Malformed { .. }),
            "{body}: {error:?}"
        );
    }
}

#[test]
fn the_seam_drives_an_operation_with_no_socket() {
    let linear = Posting::answering([Ok(json!({ "teams": { "nodes": [] } }))]);

    let data = linear
        .post(
            "query Teams($key: String!) { teams { nodes { id } } }",
            json!({ "key": "WAR" }),
        )
        .expect("the stand-in answered");

    assert_eq!(data, json!({ "teams": { "nodes": [] } }));
    assert_eq!(
        linear.documents(),
        ["query Teams($key: String!) { teams { nodes { id } } }"]
    );
    assert_eq!(linear.variables(), [json!({ "key": "WAR" })]);
}

#[test]
fn the_seam_carries_a_refusal_as_well_as_an_answer() {
    let linear = Posting::answering([Err(Error::Status { code: 401 })]);

    let error = linear
        .post("query { viewer { id } }", json!({}))
        .expect_err("the stand-in refused");

    assert!(matches!(error, Error::Status { code: 401 }), "{error:?}");
}

const SLUG: &str = "1a2b3c4d5e6f";

fn project_on_the_board() -> Value {
    json!({
        "project": {
            "id": "65fcabef-373b-4c2e-82bc-3e98fe7accbe",
            "name": "Push a brief to the board",
            "content": "# Push a brief to the board\n\n## Scope\n",
            "url": "https://linear.app/acme/project/a-brief-1a2b3c4d5e6f",
            "status": { "name": "Planned" },
            "comments": { "nodes": [
                { "body": "Warlock cut slice `One` into `WAR-1`." },
                { "body": "Warlock skipped slice `Two`." },
            ] },
        },
    })
}

#[test]
fn a_project_is_read_back_by_slug_in_one_request_with_its_notes() {
    let linear = Posting::answering([Ok(project_on_the_board())]);

    let project = fetch_project(&linear, SLUG)
        .expect("the stand-in answered")
        .expect("the stand-in knows the project");

    // Linear's own id, and not the slug it was asked by: issues and comments
    // are written against this.
    assert_eq!(project.id(), "65fcabef-373b-4c2e-82bc-3e98fe7accbe");
    assert_eq!(project.name(), "Push a brief to the board");
    assert_eq!(
        project.content(),
        "# Push a brief to the board\n\n## Scope\n"
    );
    assert_eq!(
        project.url(),
        "https://linear.app/acme/project/a-brief-1a2b3c4d5e6f"
    );
    assert_eq!(project.status(), Some("Planned"));
    assert_eq!(
        project.notes(),
        [
            "Warlock cut slice `One` into `WAR-1`.".to_owned(),
            "Warlock skipped slice `Two`.".to_owned(),
        ]
    );
    assert_eq!(
        linear.variables(),
        [json!({ "id": SLUG, "cut": CUT_NOTE, "skipped": SKIP_NOTE })]
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn only_warlocks_notes_are_asked_for_and_no_project_is_listed() {
    let linear = Posting::answering([Ok(project_on_the_board())]);

    fetch_project(&linear, SLUG).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    assert!(asked.contains("project(id: $id)"), "{asked}");
    assert!(asked.contains("startsWith: $cut"), "{asked}");
    assert!(asked.contains("startsWith: $skipped"), "{asked}");
    assert!(!asked.contains("projects("), "{asked}");
}

#[test]
fn a_project_with_no_notes_comes_back_with_none() {
    let mut answer = project_on_the_board();
    answer["project"]["comments"]["nodes"] = json!([]);
    let linear = Posting::answering([Ok(answer)]);

    let project = fetch_project(&linear, SLUG)
        .expect("a project nothing was cut from is an ordinary answer")
        .expect("the stand-in knows the project");

    assert!(project.notes().is_empty());
}

#[test]
fn a_slug_the_api_does_not_know_is_a_none_rather_than_an_error() {
    // Both shapes an unknown slug can arrive as: Linear's own refusal, and the
    // null node a nullable field would give.
    let answers = [
        Err(Error::Refused {
            message: "Entity not found - could not find referenced Project.".to_owned(),
        }),
        Ok(json!({ "project": null })),
    ];

    for answer in answers {
        let linear = Posting::answering([answer]);

        let project = fetch_project(&linear, SLUG).expect("an unknown slug is an ordinary answer");

        assert_eq!(project, None, "the caller names the slug");
    }
}

#[test]
fn any_other_refusal_is_still_linears_to_word() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Access denied".to_owned(),
    })]);

    let error = fetch_project(&linear, SLUG).expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
}

#[test]
fn a_project_with_no_status_comes_back_without_one() {
    let mut answer = project_on_the_board();
    answer["project"]["status"] = Value::Null;
    let linear = Posting::answering([Ok(answer)]);

    let project = fetch_project(&linear, SLUG)
        .expect("a project with no status is an ordinary answer")
        .expect("the stand-in knows the project");

    // A workspace with no `Backlog` takes the project with no status at all, so
    // this is a project warlock itself can have filed.
    assert_eq!(project.status(), None);
}

#[test]
fn a_project_whose_description_was_emptied_comes_back_empty() {
    let mut answer = project_on_the_board();
    answer["project"]["content"] = Value::Null;
    let linear = Posting::answering([Ok(answer)]);

    let project = fetch_project(&linear, SLUG)
        .expect("an emptied description is an ordinary answer")
        .expect("the stand-in knows the project");

    assert_eq!(project.content(), "");
}

#[test]
fn a_project_answer_missing_a_field_is_malformed() {
    for field in ["id", "name", "content", "url", "status", "comments"] {
        let mut answer = project_on_the_board();
        answer["project"]
            .as_object_mut()
            .expect("the fixture is an object")
            .remove(field);

        let linear = Posting::answering([Ok(answer)]);

        let error =
            fetch_project(&linear, SLUG).expect_err("a field that was asked for is answered");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{field}: {error:?}"
        );
    }
}

#[test]
fn a_status_with_no_name_a_note_with_no_body_and_no_project_are_malformed() {
    let mut nameless = project_on_the_board();
    nameless["project"]["status"] = json!({});
    let mut bodiless = project_on_the_board();
    bodiless["project"]["comments"]["nodes"] = json!([{}]);

    for answer in [nameless, bodiless, json!({ "projects": { "nodes": [] } })] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = fetch_project(&linear, SLUG).expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
}

fn planned(nodes: &Value, more: bool) -> Value {
    json!({ "projects": { "pageInfo": { "hasNextPage": more }, "nodes": nodes } })
}

#[test]
fn planned_projects_are_the_teams_planned_and_labelled_ones_by_slug_and_name() {
    let linear = Posting::answering([Ok(planned(
        &json!([
            { "slugId": "9e41c07a2b13", "name": "Draft from the board" },
            { "slugId": "d1cb3521be71", "name": "Give the CLI a voice" },
        ]),
        false,
    ))]);

    let listing = planned_projects(&linear, "WAR", "warlock").expect("the stand-in answered");

    assert_eq!(
        listing.projects(),
        [
            ("9e41c07a2b13".to_owned(), "Draft from the board".to_owned()),
            ("d1cb3521be71".to_owned(), "Give the CLI a voice".to_owned()),
        ]
    );
    assert!(!listing.capped());
    assert_eq!(
        linear.variables(),
        [json!({ "team": "WAR", "label": "warlock" })]
    );
    let asked = linear.documents().pop().expect("one request was made");
    assert!(asked.contains("key: { eq: $team }"), "{asked}");
    assert!(asked.contains(r#"eqIgnoreCase: "Planned""#), "{asked}");
    assert!(asked.contains("name: { eq: $label }"), "{asked}");
    assert!(asked.contains("hasNextPage"), "{asked}");
}

#[test]
fn a_planned_page_with_more_behind_it_is_capped_and_an_empty_one_is_ordinary() {
    let linear = Posting::answering([
        Ok(planned(
            &json!([{ "slugId": "9e41c07a2b13", "name": "Draft" }]),
            true,
        )),
        Ok(planned(&json!([]), false)),
    ]);

    assert!(
        planned_projects(&linear, "WAR", "warlock")
            .expect("the stand-in answered")
            .capped()
    );
    let empty = planned_projects(&linear, "WAR", "warlock").expect("the stand-in answered");
    assert!(empty.projects().is_empty());
    assert!(!empty.capped());
}

#[test]
fn a_planned_answer_missing_its_slug_or_its_page_is_malformed() {
    for answer in [
        planned(&json!([{ "name": "Draft" }]), false),
        json!({ "projects": { "nodes": [] } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error =
            planned_projects(&linear, "WAR", "warlock").expect_err("not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
}

#[test]
fn a_project_of_the_same_name_is_answered_by_its_url_and_none_is_none() {
    let linear = Posting::answering([
        Ok(json!({ "projects": { "nodes": [
            { "url": "https://linear.app/acme/project/a-brief-1a2b3c4d5e6f" },
        ] } })),
        Ok(json!({ "projects": { "nodes": [] } })),
    ]);

    assert_eq!(
        project_named(&linear, "WAR", "A brief").expect("the stand-in answered"),
        Some("https://linear.app/acme/project/a-brief-1a2b3c4d5e6f".to_owned())
    );
    assert_eq!(
        project_named(&linear, "WAR", "A brief").expect("the stand-in answered"),
        None
    );
    assert_eq!(
        linear.variables(),
        [
            json!({ "team": "WAR", "name": "A brief" }),
            json!({ "team": "WAR", "name": "A brief" }),
        ]
    );
    let asked = linear.documents().pop().expect("a request was made");
    assert!(asked.contains("eqIgnoreCase: $name"), "{asked}");
    assert!(asked.contains("first: 1"), "{asked}");
}

// The three ids a scope's record and this machine's key resolve to, which is
// the whole of what the queue is asked in terms of.
const TEAM: &str = "team-1";
const LABEL: &str = "warlock";
const ME: &str = "user-viewer";

fn a_queue_of(issues: &[Value], more: bool) -> Value {
    json!({ "issues": { "pageInfo": { "hasNextPage": more }, "nodes": issues } })
}

fn an_issue() -> Value {
    json!({
        "id": "1b9a5d2e-6c47-4f0a-9d31-0e7b2c4a8f55",
        "identifier": "WAR-133",
        "title": "Read a scope's ticket queue from Linear",
        "priority": 2,
        "state": { "name": "Todo", "type": "unstarted" },
        "inverseRelations": { "nodes": [blocking("WAR-129", Some("Ada"), "started")] },
    })
}

// One `blocks` edge as it arrives on an issue's `inverseRelations`, where the
// far side is the issue doing the blocking.
fn blocking(identifier: &str, assignee: Option<&str>, state: &str) -> Value {
    json!({
        "type": "blocks",
        "issue": {
            "identifier": identifier,
            "state": { "type": state },
            "assignee": assignee.map(|name| json!({ "name": name })),
        },
    })
}

fn only_issue(queue: &Value) -> QueuedIssue {
    let linear = Posting::answering([Ok(queue.clone())]);

    scope_queue(&linear, TEAM, LABEL, ME)
        .expect("the stand-in answered")
        .issues()
        .first()
        .expect("the stand-in answered with one issue")
        .clone()
}

// The field at that JSON pointer taken out of the object holding it.
fn without(pointer: &str) -> Value {
    dropping(a_queue_of(&[an_issue()], false), pointer)
}

// The same, out of whichever answer is handed in.
fn dropping(mut answer: Value, pointer: &str) -> Value {
    let (parent, field) = pointer
        .rsplit_once('/')
        .expect("a pointer to a field under an object");

    answer
        .pointer_mut(parent)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("the fixture has no `{parent}`"))
        .remove(field)
        .unwrap_or_else(|| panic!("the fixture has no `{pointer}`"));

    answer
}

#[test]
fn a_scopes_whole_queue_is_read_in_one_request() {
    let linear = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);

    let queue = scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    let issue = &queue.issues()[0];

    assert_eq!(issue.id(), "1b9a5d2e-6c47-4f0a-9d31-0e7b2c4a8f55");
    assert_eq!(issue.identifier(), "WAR-133");
    assert_eq!(issue.title(), "Read a scope's ticket queue from Linear");
    // The team's own name for the state, and Linear's type for it: a record
    // names states the first way and readiness is decided the second.
    assert_eq!(issue.state(), "Todo");
    assert_eq!(issue.state_type(), &StateType::new("unstarted"));
    assert_eq!(issue.priority(), Priority::High);
    assert_eq!(
        issue.blockers(),
        [Blocker::new(
            "WAR-129",
            Some("Ada"),
            StateType::new("started")
        )]
    );
    assert!(!queue.capped());
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn the_queue_is_the_teams_labelled_work_assigned_to_the_key_holder_and_not_finished() {
    let linear = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);

    scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    assert!(asked.contains("issues("), "{asked}");
    // On the key, because the scope record holds `WAR` rather than a team id:
    // an `id` filter here is refused by Linear with "eq must be a UUID".
    assert!(asked.contains("$team: String!"), "{asked}");
    assert!(asked.contains("team: { key: { eq: $team } }"), "{asked}");
    assert!(
        asked.contains("labels: { name: { eq: $label } }"),
        "{asked}"
    );
    assert!(
        asked.contains("assignee: { id: { eq: $assignee } }"),
        "{asked}"
    );
    // On the state's *type* and never its name: a team calls its finished state
    // whatever it likes, and a queue that filtered by name would work tickets
    // that shipped last week.
    assert!(
        asked.contains(r#"state: { type: { nin: ["completed", "canceled"] } }"#),
        "{asked}"
    );
    assert_eq!(
        linear.variables(),
        [json!({
            "team": TEAM,
            "label": LABEL,
            "assignee": ME,
            "first": QUEUE_PAGE,
            "blockers": BLOCKERS_PAGE,
        })]
    );
}

#[test]
fn the_page_asked_for_is_the_cap_the_constant_holds() {
    let linear = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);

    scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    assert!(asked.contains("first: $first"), "{asked}");
    assert_eq!(linear.variables()[0]["first"], json!(QUEUE_PAGE));
    // And the nested connection has its own cap rather than whatever default
    // Linear applies to a relation list nobody bounded.
    assert!(
        asked.contains("inverseRelations(first: $blockers)"),
        "{asked}"
    );
    assert_eq!(linear.variables()[0]["blockers"], json!(BLOCKERS_PAGE));
}

#[test]
fn a_blocker_is_an_issue_blocking_this_one_and_never_one_it_blocks() {
    // The direction is the whole risk: reading `relations` instead of
    // `inverseRelations` compiles, answers the same shape, and inverts every
    // dependency in the queue. `WAR-140` waits on this issue and must not read
    // as holding it up.
    let mut waiting = an_issue();
    waiting["relations"] = json!({
        "nodes": [blocking("WAR-140", Some("Ada"), "unstarted")],
    });

    let issue = only_issue(&a_queue_of(&[waiting], false));

    assert_eq!(
        issue
            .blockers()
            .iter()
            .map(Blocker::identifier)
            .collect::<Vec<_>>(),
        ["WAR-129"]
    );
}

#[test]
fn the_only_relations_read_are_the_inverse_ones() {
    let linear = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);

    scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    assert!(asked.contains("inverseRelations"), "{asked}");
    // With the one field taken out of the text, no other `relations` may be
    // left in it: asking for both and reading the wrong one is the inversion
    // above.
    assert!(!asked.replace("inverseRelations", "").contains("relations"));
}

#[test]
fn a_relation_that_blocks_nothing_is_read_and_dropped() {
    let mut issue = an_issue();
    issue["inverseRelations"]["nodes"] = json!([
        { "type": "related", "issue": { "identifier": "WAR-1", "state": { "type": "started" }, "assignee": null } },
        blocking("WAR-129", Some("Ada"), "started"),
        { "type": "duplicate", "issue": { "identifier": "WAR-2", "state": { "type": "started" }, "assignee": null } },
    ]);

    let issue = only_issue(&a_queue_of(&[issue], false));

    assert_eq!(issue.blockers().len(), 1, "only `blocks` holds work up");
    assert_eq!(issue.blockers()[0].identifier(), "WAR-129");
}

#[test]
fn a_blocker_is_carried_whoever_owns_it_and_whatever_state_it_is_in() {
    let mut issue = an_issue();
    issue["inverseRelations"]["nodes"] = json!([
        blocking("WAR-9", Some("Someone Else"), "started"),
        blocking("WAR-10", None, "backlog"),
        blocking("WAR-11", Some("Ada"), "completed"),
    ]);

    let issue = only_issue(&a_queue_of(&[issue], false));

    // The queue's own filters are not the blockers': an issue is held up by
    // whatever blocks it, on anybody's plate and under any label, and an
    // unassigned one still holds it up.
    assert_eq!(
        issue.blockers(),
        [
            Blocker::new("WAR-9", Some("Someone Else"), StateType::new("started")),
            Blocker::new("WAR-10", None, StateType::new("backlog")),
            Blocker::new("WAR-11", Some("Ada"), StateType::new("completed")),
        ]
    );
    assert!(!issue.blockers()[0].state_type().settled());
    assert!(!issue.blockers()[1].state_type().settled());
    assert!(issue.blockers()[2].state_type().settled(), "out of the way");
}

#[test]
fn the_blockers_asked_for_carry_no_filter_of_their_own() {
    let linear = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);

    scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");
    let (_, relations) = asked
        .split_once("inverseRelations")
        .expect("the document reads the inverse relations");

    // A blocker filtered by assignee or label is a blocker that disappears, and
    // an issue held up by somebody else's ticket that reads as ready.
    assert!(!relations.contains("filter"), "{relations}");
    assert!(!relations.contains("assignee: {"), "{relations}");
    assert!(!relations.contains("labels"), "{relations}");
}

#[test]
fn only_completed_and_canceled_are_out_of_the_way() {
    for settled in ["completed", "canceled"] {
        assert!(StateType::new(settled).settled(), "{settled}");
    }
    for open in ["triage", "backlog", "unstarted", "started", "invented"] {
        assert!(!StateType::new(open).settled(), "{open}");
    }
}

#[test]
fn linears_priority_numbers_are_read_as_the_order_work_is_taken() {
    let ranks = [
        (1, Priority::Urgent),
        (2, Priority::High),
        (3, Priority::Medium),
        (4, Priority::Low),
        (0, Priority::None),
    ];

    for (number, rank) in ranks {
        let mut issue = an_issue();
        issue["priority"] = json!(number);

        assert_eq!(
            only_issue(&a_queue_of(&[issue], false)).priority(),
            rank,
            "priority {number}"
        );
    }

    // The reason the number does not leave the module: `0` is Linear's "no
    // priority" and would sort ahead of urgent, so the order has to be the
    // type's and not the integer's.
    let mut ordered = ranks.map(|(_, rank)| rank);
    ordered.sort_unstable();
    assert_eq!(
        ordered,
        [
            Priority::Urgent,
            Priority::High,
            Priority::Medium,
            Priority::Low,
            Priority::None,
        ]
    );
}

#[test]
fn a_priority_number_no_version_of_linear_sends_is_no_priority() {
    for number in [json!(5), json!(-1), json!(2.0)] {
        let mut issue = an_issue();
        issue["priority"] = number.clone();

        let read = only_issue(&a_queue_of(&[issue], false)).priority();

        // `2.0` is the `Float!` the schema promises spelled the other way, and
        // still high; the two outside the five are not a reason to work
        // something first.
        let expected = if number == json!(2.0) {
            Priority::High
        } else {
            Priority::None
        };

        assert_eq!(read, expected, "{number}");
    }
}

#[test]
fn a_page_that_came_back_full_says_so_and_a_short_one_does_not() {
    let short = Posting::answering([Ok(a_queue_of(&[an_issue()], false))]);
    let more = Posting::answering([Ok(a_queue_of(&[an_issue()], true))]);

    assert!(
        !scope_queue(&short, TEAM, LABEL, ME)
            .expect("the stand-in answered")
            .capped()
    );
    // `hasNextPage`, which is Linear saying there is another page, is the plain
    // case.
    assert!(
        scope_queue(&more, TEAM, LABEL, ME)
            .expect("the stand-in answered")
            .capped()
    );

    // And a page filled exactly to the cap, which an API that answers
    // `hasNextPage: false` on the boundary would otherwise hide.
    let full = Posting::answering([Ok(a_queue_of(&vec![an_issue(); QUEUE_PAGE], false))]);

    let queue = scope_queue(&full, TEAM, LABEL, ME).expect("the stand-in answered");

    assert_eq!(queue.issues().len(), QUEUE_PAGE);
    assert!(queue.capped());
}

#[test]
fn an_issue_whose_relations_filled_their_page_caps_the_queue_too() {
    let mut issue = an_issue();
    issue["inverseRelations"]["nodes"] =
        Value::Array(vec![blocking("WAR-9", None, "started"); BLOCKERS_PAGE]);

    let linear = Posting::answering([Ok(a_queue_of(&[issue], false))]);

    let queue = scope_queue(&linear, TEAM, LABEL, ME).expect("the stand-in answered");

    // A blocker list cut off would make a held-up issue read as ready, so the
    // flag covers it: what it says is that there is more of this queue on the
    // board than came back.
    assert!(queue.capped());
}

#[test]
fn a_queue_with_nothing_on_it_is_an_ordinary_answer() {
    let linear = Posting::answering([Ok(a_queue_of(&[], false))]);

    let queue = scope_queue(&linear, TEAM, LABEL, ME).expect("an empty queue is an answer");

    assert_eq!(queue.issues(), []);
    assert!(!queue.capped());
}

#[test]
fn a_queue_answer_that_is_not_the_one_asked_for_is_malformed() {
    let mut answers = vec![
        json!({ "issues": {} }),
        json!({ "projects": { "nodes": [] } }),
        // Asked for and not answered, so the page is not one this side can say
        // anything about.
        json!({ "issues": { "nodes": [] } }),
        a_queue_of(&[json!({})], false),
    ];

    for pointer in [
        "/issues/pageInfo/hasNextPage",
        "/issues/nodes/0/id",
        "/issues/nodes/0/identifier",
        "/issues/nodes/0/title",
        "/issues/nodes/0/priority",
        "/issues/nodes/0/state",
        "/issues/nodes/0/state/name",
        "/issues/nodes/0/state/type",
        "/issues/nodes/0/inverseRelations",
        "/issues/nodes/0/inverseRelations/nodes/0/type",
        "/issues/nodes/0/inverseRelations/nodes/0/issue",
        "/issues/nodes/0/inverseRelations/nodes/0/issue/identifier",
        "/issues/nodes/0/inverseRelations/nodes/0/issue/state",
        "/issues/nodes/0/inverseRelations/nodes/0/issue/state/type",
        "/issues/nodes/0/inverseRelations/nodes/0/issue/assignee",
    ] {
        answers.push(without(pointer));
    }

    for answer in answers {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error =
            scope_queue(&linear, TEAM, LABEL, ME).expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no second page");
    }
}

#[test]
fn an_unassigned_issue_can_still_be_holding_one_up() {
    let mut issue = an_issue();
    issue["inverseRelations"]["nodes"] = json!([blocking("WAR-9", None, "started")]);

    let issue = only_issue(&a_queue_of(&[issue], false));

    assert_eq!(issue.blockers()[0].assignee(), None);
}

#[test]
fn the_board_hands_the_team_label_and_assignee_to_the_wire_in_that_order() {
    // Three `&str` in a row: a swap between the trait and the operation under it
    // compiles, and asks the board for somebody else's work.
    let board = Linear::new(Posting::answering([Ok(a_queue_of(&[], false))]));

    board
        .scope_queue(TEAM, LABEL, ME)
        .expect("the stand-in answered");

    assert_eq!(
        board.posts.variables().pop().expect("one request was made"),
        json!({
            "team": TEAM,
            "label": LABEL,
            "assignee": ME,
            "first": QUEUE_PAGE,
            "blockers": BLOCKERS_PAGE,
        })
    );
}

// One named ticket as the board answers it: the queue's own node, plus the three
// facts the queue answered with a filter instead.
fn a_named_ticket(node: &Value) -> Value {
    json!({ "issues": { "nodes": [node] } })
}

fn named_node() -> Value {
    let mut node = an_issue();

    node["team"] = json!({ "key": "WAR" });
    node["labels"] = json!({ "nodes": [{ "name": "warlock" }, { "name": "area/tui" }] });
    node["assignee"] = json!({ "id": ME, "name": "Cole" });
    node
}

fn a_ticket(node: &Value) -> NamedIssue {
    let linear = Posting::answering([Ok(a_named_ticket(node))]);

    named_issue(&linear, "WAR", 133)
        .expect("the stand-in answered")
        .expect("the stand-in answered with the ticket")
}

#[test]
fn a_named_ticket_is_read_by_the_two_facts_its_identifier_is_made_of() {
    let linear = Posting::answering([Ok(a_named_ticket(&named_node()))]);

    let found = named_issue(&linear, "WAR", 133)
        .expect("the stand-in answered")
        .expect("the stand-in answered with the ticket");

    // The identifier itself is not a filter Linear has: it is a display name made
    // of the team's key and the number, and those two are what can be asked.
    let asked = linear.documents().pop().expect("one request was made");

    assert!(
        asked.contains("filter: { team: { key: { eq: $team } }, number: { eq: $number } }"),
        "{asked}"
    );
    assert_eq!(found.issue().identifier(), "WAR-133");
    assert_eq!(linear.documents().len(), 1, "one request per question");
}

#[test]
fn the_named_read_carries_none_of_the_queues_filters() {
    let linear = Posting::answering([Ok(a_named_ticket(&named_node()))]);

    named_issue(&linear, "WAR", 133).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");
    let (filter, _) = asked
        .split_once('\n')
        .and_then(|(_, rest)| rest.split_once("first: 1"))
        .expect("the document filters one page of one");

    // Label, assignee and state are read as fields and never filtered on, which
    // is the whole point of this read: a ticket dropped by a filter cannot say
    // which filter dropped it, so the three arrive as facts to be checked.
    assert!(!filter.contains("labels: {"), "{filter}");
    assert!(!filter.contains("assignee: {"), "{filter}");
    assert!(!filter.contains("state: {"), "{filter}");
    // And the selection asks for all three.
    assert!(asked.contains("team { key }"), "{asked}");
    assert!(
        asked.contains("labels(first: $labels) { nodes { name } }"),
        "{asked}"
    );
    assert!(asked.contains("assignee { id name }"), "{asked}");
}

#[test]
fn the_named_read_asks_for_the_caps_the_constants_hold() {
    let linear = Posting::answering([Ok(a_named_ticket(&named_node()))]);

    named_issue(&linear, "war", 133).expect("the stand-in answered");

    assert_eq!(
        linear.variables(),
        [json!({
            // Upper cased on the way out, because Linear's team keys are and
            // `war-133` is a thing a person types.
            "team": "WAR",
            "number": 133,
            "labels": LABELS_PAGE,
            "blockers": BLOCKERS_PAGE,
        })]
    );
}

#[test]
fn a_named_ticket_is_the_same_value_a_queue_would_have_given() {
    // Parsed by the queue's own function, so a named ticket and a chosen one are
    // one type and the rules over them cannot drift.
    let found = a_ticket(&named_node());

    assert_eq!(
        found.issue(),
        &only_issue(&a_queue_of(&[an_issue()], false))
    );
}

#[test]
fn a_named_ticket_carries_the_team_labels_and_assignee_the_queue_filtered_on() {
    let found = a_ticket(&named_node());

    // The key and not the id: a scope record names a team `WAR`, so the two are
    // comparable without resolving either.
    assert_eq!(found.team(), "WAR");
    assert_eq!(found.labels(), ["warlock", "area/tui"]);
    assert_eq!(found.assignee(), Some(&Assignee::new(ME, "Cole")));
    // The id is what says whether a ticket is yours, and the name is what a
    // refusal prints: two people in a workspace can share a display name.
    assert_eq!(found.assignee().map(Assignee::id), Some(ME));
    assert_eq!(found.assignee().map(Assignee::name), Some("Cole"));
}

#[test]
fn a_ticket_with_no_labels_and_nobody_on_it_is_an_ordinary_answer() {
    let mut node = named_node();
    node["labels"] = json!({ "nodes": [] });
    node["assignee"] = Value::Null;

    let found = a_ticket(&node);

    // Neither is malformed: both are refusals the caller words, not answers this
    // module cannot read.
    assert_eq!(found.labels(), [] as [String; 0]);
    assert_eq!(found.assignee(), None);
}

#[test]
fn a_ticket_that_shipped_is_still_answered() {
    let mut node = named_node();
    node["state"] = json!({ "name": "Done", "type": "completed" });

    let found = a_ticket(&node);

    // The state filter is left off this read on purpose: a finished ticket is
    // absent from the queue too, and a refusal calling that "not on your team"
    // would be a lie about the board.
    assert_eq!(found.issue().state(), "Done");
    assert!(found.issue().state_type().settled());
}

#[test]
fn a_number_no_ticket_has_is_an_absence_rather_than_an_error() {
    let linear = Posting::answering([Ok(json!({ "issues": { "nodes": [] } }))]);

    let found = named_issue(&linear, "WAR", 9_999).expect("an empty answer is an answer");

    assert_eq!(found, None);
}

#[test]
fn a_named_answer_that_is_not_the_one_asked_for_is_malformed() {
    let mut answers = vec![
        json!({ "issues": {} }),
        json!({ "projects": { "nodes": [] } }),
        a_named_ticket(&json!({})),
    ];

    for pointer in [
        // The three the queue answered with a filter: a missing one is an answer
        // no gate can be decided from, so it is never read as an absence.
        "/issues/nodes/0/team",
        "/issues/nodes/0/team/key",
        "/issues/nodes/0/labels",
        "/issues/nodes/0/labels/nodes/0/name",
        "/issues/nodes/0/assignee",
        "/issues/nodes/0/assignee/id",
        "/issues/nodes/0/assignee/name",
        // And the queue's own fields, since the ticket is parsed by its function.
        "/issues/nodes/0/identifier",
        "/issues/nodes/0/state/type",
        "/issues/nodes/0/inverseRelations",
    ] {
        answers.push(dropping(a_named_ticket(&named_node()), pointer));
    }

    for answer in answers {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = named_issue(&linear, "WAR", 133).expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no second page");
    }
}

#[test]
fn the_board_hands_the_team_key_and_the_number_to_the_wire_in_that_order() {
    let board = Linear::new(Posting::answering([Ok(a_named_ticket(&named_node()))]));

    let found = board
        .named_issue("WAR", 133)
        .expect("the stand-in answered")
        .expect("the stand-in answered with the ticket");

    assert_eq!(found.issue().identifier(), "WAR-133");
    assert_eq!(
        board.posts.variables().pop().expect("one request was made")["number"],
        json!(133)
    );
}

fn issue_label_found() -> Value {
    json!({ "issueLabels": { "nodes": [{ "id": "issue-label-held" }] } })
}

fn no_issue_label() -> Value {
    json!({ "issueLabels": { "nodes": [] } })
}

fn issue_label_created() -> Value {
    json!({ "issueLabelCreate": { "issueLabel": { "id": "issue-label-made" } } })
}

#[test]
fn an_issue_label_the_team_already_has_is_reused_by_id() {
    let linear = Posting::answering([Ok(issue_label_found())]);

    let label = issue_label_id(&linear, "warlock", "team-1").expect("the stand-in answered");

    assert_eq!(label, "issue-label-held");
    assert_eq!(
        linear.documents().len(),
        1,
        "a label that exists is not created again"
    );
    assert_eq!(
        linear.variables(),
        [json!({ "name": "warlock", "team": "team-1" })]
    );
}

#[test]
fn an_issue_label_the_team_lacks_is_created_on_that_team() {
    let linear = Posting::answering([Ok(no_issue_label()), Ok(issue_label_created())]);

    let label = issue_label_id(&linear, "warlock", "team-1").expect("the stand-in answered");

    assert_eq!(label, "issue-label-made");

    let asked = linear.documents();

    assert_eq!(asked.len(), 2, "one request per thing asked");
    assert!(asked[0].contains("issueLabels("), "{asked:?}");
    assert!(asked[1].contains("issueLabelCreate("), "{asked:?}");
    assert_eq!(
        linear.variables()[1],
        json!({ "input": { "name": "warlock", "teamId": "team-1" } })
    );
}

#[test]
fn an_issue_label_never_reaches_the_project_label_queries() {
    let linear = Posting::answering([Ok(no_issue_label()), Ok(issue_label_created())]);

    issue_label_id(&linear, "warlock", "team-1").expect("the stand-in answered");

    // The two are different types in Linear's schema, and an id from one is not
    // usable by the other, so neither resolver may drift onto the other's
    // queries.
    for asked in linear.documents() {
        assert!(!asked.contains("projectLabel"), "{asked}");
    }
}

#[test]
fn a_project_label_never_reaches_the_issue_label_queries() {
    let linear = Posting::answering([Ok(no_label()), Ok(label_created())]);

    label_id(&linear, "warlock").expect("the stand-in answered");

    for asked in linear.documents() {
        assert!(!asked.contains("issueLabel"), "{asked}");
    }
}

#[test]
fn an_issue_label_answer_that_is_not_the_one_asked_for_is_malformed() {
    for answers in [
        vec![Ok(json!({ "issueLabels": {} }))],
        vec![Ok(
            json!({ "issueLabels": { "nodes": [{ "name": "warlock" }] } }),
        )],
        vec![
            Ok(no_issue_label()),
            Ok(json!({ "issueLabelCreate": { "issueLabel": null } })),
        ],
        vec![
            Ok(no_issue_label()),
            Ok(json!({ "issueLabelCreate": { "success": true } })),
        ],
    ] {
        let asked = answers.len();
        let linear = Posting::answering(answers);

        let error = issue_label_id(&linear, "warlock", "team-1")
            .expect_err("that is not the answer asked for");

        assert!(matches!(error, Error::Malformed { .. }), "{error:?}");
        assert_eq!(linear.documents().len(), asked, "no retry and no backoff");
    }
}

fn label_found() -> Value {
    json!({ "projectLabels": { "nodes": [{ "id": "label-held" }] } })
}

fn no_label() -> Value {
    json!({ "projectLabels": { "nodes": [] } })
}

fn label_created() -> Value {
    json!({ "projectLabelCreate": { "projectLabel": { "id": "label-made" } } })
}

fn project_created() -> Value {
    json!({
        "projectCreate": {
            "project": {
                "id": "project-1",
                "url": "https://linear.app/acme/project/a-brief-1a2b3c",
            },
        },
    })
}

fn brief<'a>() -> NewProject<'a> {
    NewProject::new(
        "Repair the answer",
        "# Repair the answer\n\nBody.\n",
        "team-1",
        "warlock",
    )
}

// The `input` of the last thing the stand-in was asked, which is the create
// under test in every test that gets this far — the resolvers that run before
// one send their own variables, and the create is always last.
fn last_input(linear: &Posting) -> Value {
    linear
        .variables()
        .pop()
        .expect("the stand-in was asked at least once")["input"]
        .clone()
}

#[test]
fn the_viewer_is_the_key_holders_id_in_one_request() {
    let linear = Posting::answering([Ok(json!({ "viewer": { "id": "user-1" } }))]);

    let user = viewer(&linear).expect("the stand-in answered");

    assert_eq!(user, "user-1");
    assert_eq!(linear.variables(), [json!({})]);

    let asked = linear.documents();

    assert_eq!(asked.len(), 1, "one request per operation");
    assert!(asked[0].contains("viewer { id }"), "{asked:?}");
}

#[test]
fn a_viewer_answer_that_is_not_the_one_asked_for_is_malformed() {
    for answer in [
        json!({}),
        json!({ "viewer": null }),
        json!({ "viewer": { "name": "Ada" } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = viewer(&linear).expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
}

#[test]
fn a_team_key_resolves_to_its_id_in_one_request() {
    let linear = Posting::answering([Ok(json!({ "teams": { "nodes": [{ "id": "team-1" }] } }))]);

    let team = team_id(&linear, "WAR").expect("the stand-in answered");

    assert_eq!(team.as_deref(), Some("team-1"));
    assert_eq!(linear.variables(), [json!({ "key": "WAR" })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_team_key_the_api_does_not_know_is_a_none_rather_than_an_error() {
    let linear = Posting::answering([Ok(json!({ "teams": { "nodes": [] } }))]);

    let team = team_id(&linear, "NOPE").expect("an unknown key is an ordinary answer");

    assert_eq!(team, None, "the caller turns this into its own refusal");
}

#[test]
fn a_team_answer_with_no_nodes_is_malformed() {
    let linear = Posting::answering([Ok(json!({ "teams": {} }))]);

    let error = team_id(&linear, "WAR").expect_err("a connection with no nodes is no answer");

    assert!(matches!(error, Error::Malformed { .. }), "{error:?}");
}

#[test]
fn a_team_node_with_no_id_is_malformed() {
    let linear = Posting::answering([Ok(json!({ "teams": { "nodes": [{ "key": "WAR" }] } }))]);

    let error = team_id(&linear, "WAR").expect_err("a node with no id is no answer");

    assert!(matches!(error, Error::Malformed { .. }), "{error:?}");
}

#[test]
fn the_backlog_status_is_found_by_name_among_the_others() {
    let linear = Posting::answering([Ok(json!({
        "projectStatuses": {
            "nodes": [
                { "id": "status-planned", "name": "Planned" },
                { "id": "status-backlog", "name": BACKLOG },
                { "id": "status-done", "name": "Completed" },
            ],
        },
    }))]);

    let status = backlog_status(&linear).expect("the stand-in answered");

    assert_eq!(status.as_deref(), Some("status-backlog"));
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_workspace_with_no_backlog_status_is_a_none_rather_than_an_error() {
    let linear = Posting::answering([Ok(json!({
        "projectStatuses": { "nodes": [{ "id": "status-now", "name": "In Progress" }] },
    }))]);

    let status = backlog_status(&linear).expect("no `Backlog` is an ordinary answer");

    assert_eq!(status, None, "the caller creates with no status");
}

#[test]
fn a_project_status_is_found_by_name_ignoring_case_and_spaces() {
    let linear = Posting::answering([Ok(json!({
        "projectStatuses": {
            "nodes": [
                { "id": "status-planned", "name": "Planned" },
                { "id": "status-doing", "name": " in progress " },
            ],
        },
    }))]);

    let status = project_status(&linear, IN_PROGRESS).expect("the stand-in answered");

    assert_eq!(status.as_deref(), Some("status-doing"));
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_workspace_with_no_such_project_status_is_a_none_rather_than_an_error() {
    let linear = Posting::answering([Ok(json!({
        "projectStatuses": { "nodes": [{ "id": "status-planned", "name": "Planned" }] },
    }))]);

    let status = project_status(&linear, IN_PROGRESS).expect("an ordinary answer");

    assert_eq!(status, None);
}

#[test]
fn a_project_move_writes_the_status_and_nothing_else() {
    let linear = Posting::answering([Ok(json!({
        "projectUpdate": { "project": { "id": "project-1" } },
    }))]);

    let moved = move_project(&linear, "project-1", "status-doing").expect("the stand-in answered");

    assert_eq!(moved, "project-1");
    let asked = linear.documents().pop().expect("one request was made");
    assert!(asked.contains("projectUpdate("), "{asked}");
    assert_eq!(
        linear.variables(),
        [json!({ "id": "project-1", "input": { "statusId": "status-doing" } })]
    );
}

fn workflow_states() -> Value {
    json!({
        "workflowStates": {
            "nodes": [
                { "id": "state-todo", "name": "Todo" },
                { "id": "state-backlog", "name": BACKLOG },
                { "id": "state-doing", "name": "In Progress" },
            ],
        },
    })
}

#[test]
fn the_backlog_workflow_state_is_found_by_name_among_the_teams_others() {
    let linear = Posting::answering([Ok(workflow_states())]);

    let state = backlog_state(&linear, "team-1").expect("the stand-in answered");

    assert_eq!(state.as_deref(), Some("state-backlog"));
    assert_eq!(linear.variables(), [json!({ "team": "team-1" })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn the_workflow_states_asked_for_are_the_resolved_teams_own() {
    let linear = Posting::answering([Ok(workflow_states())]);

    backlog_state(&linear, "team-1").expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    // Workflow states belong to a team, so a workspace-wide list would find
    // another team's `Backlog` and file the issue somewhere nobody asked for.
    assert!(asked.contains("workflowStates("), "{asked}");
    assert!(asked.contains("team: { id: { eq: $team } }"), "{asked}");
}

#[test]
fn a_team_with_no_backlog_workflow_state_is_a_none_rather_than_an_error() {
    let linear = Posting::answering([Ok(json!({
        "workflowStates": { "nodes": [{ "id": "state-doing", "name": "In Progress" }] },
    }))]);

    let state = backlog_state(&linear, "team-1").expect("no `Backlog` is an ordinary answer");

    assert_eq!(state, None, "the caller refuses naming the team");
}

#[test]
fn a_workflow_state_answer_that_is_not_the_one_asked_for_is_malformed() {
    for answer in [
        json!({ "workflowStates": {} }),
        json!({ "workflowStates": { "nodes": [{ "name": BACKLOG }] } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = backlog_state(&linear, "team-1").expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
}

#[test]
fn a_workflow_state_is_resolved_by_name_among_the_teams_others() {
    let linear = Posting::answering([Ok(workflow_states())]);

    let state = workflow_state(&linear, "team-1", IN_PROGRESS).expect("the stand-in answered");

    assert_eq!(state.as_deref(), Some("state-doing"));
    assert_eq!(linear.variables(), [json!({ "team": "team-1" })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_state_name_spelled_differently_is_still_the_same_column() {
    // A team that renamed the case, and a record or a constant with a stray
    // space in it: the same column to everyone except a string comparison.
    for name in ["in progress", "  In Progress ", "IN PROGRESS"] {
        let linear = Posting::answering([Ok(workflow_states())]);

        let state = workflow_state(&linear, "team-1", name).expect("the stand-in answered");

        assert_eq!(state.as_deref(), Some("state-doing"), "{name}");
        assert_eq!(linear.documents().len(), 1, "one request per operation");
    }
}

#[test]
fn a_team_with_no_state_by_that_name_is_a_none_rather_than_an_error() {
    let linear = Posting::answering([Ok(workflow_states())]);

    // The caller prints this as a line and works the ticket anyway: a column
    // the team does not have is not a reason to stop.
    let state = workflow_state(&linear, "team-1", "Doing").expect("no such column is an answer");

    assert_eq!(state, None);
}

#[test]
fn a_named_state_answer_that_is_not_the_one_asked_for_is_malformed() {
    for answer in [
        json!({ "workflowStates": {} }),
        json!({ "workflowStates": { "nodes": [{ "name": IN_PROGRESS }] } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = workflow_state(&linear, "team-1", IN_PROGRESS)
            .expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

#[test]
fn a_state_lookup_the_api_refuses_comes_back_once_in_linears_words() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Entity not found".to_owned(),
    })]);

    let error = workflow_state(&linear, "team-1", IN_PROGRESS).expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
    assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
}

#[test]
fn a_label_the_workspace_already_has_is_reused_by_id() {
    let linear = Posting::answering([Ok(label_found())]);

    let label = label_id(&linear, "warlock").expect("the stand-in answered");

    assert_eq!(label, "label-held");
    assert_eq!(
        linear.documents().len(),
        1,
        "a label that exists is not created again"
    );
    assert_eq!(linear.variables(), [json!({ "name": "warlock" })]);
}

#[test]
fn a_label_the_workspace_lacks_is_created_under_that_name() {
    let linear = Posting::answering([Ok(no_label()), Ok(label_created())]);

    let label = label_id(&linear, "warlock").expect("the stand-in answered");

    assert_eq!(label, "label-made");

    let asked = linear.documents();

    assert!(asked[0].contains("projectLabels"), "{asked:?}");
    assert!(asked[1].contains("projectLabelCreate"), "{asked:?}");
    assert_eq!(
        linear.variables()[1],
        json!({ "input": { "name": "warlock" } })
    );
}

#[test]
fn a_label_create_that_answers_nothing_is_malformed() {
    let linear = Posting::answering([
        Ok(no_label()),
        Ok(json!({ "projectLabelCreate": { "projectLabel": null } })),
    ]);

    let error = label_id(&linear, "warlock").expect_err("a payload with no label is no answer");

    assert!(matches!(error, Error::Malformed { .. }), "{error:?}");
}

#[test]
fn the_label_is_resolved_before_the_project_is_created() {
    let linear = Posting::answering([Ok(label_found()), Ok(project_created())]);

    create_project(&linear, &brief().with_status(Some("status-backlog")))
        .expect("the stand-in answered");

    let asked = linear.documents();

    assert_eq!(asked.len(), 2);
    assert!(asked[0].contains("projectLabels"), "{asked:?}");
    assert!(
        asked[1].contains("projectCreate"),
        "the project was created before its label was resolved: {asked:?}"
    );
}

#[test]
fn a_label_that_had_to_be_created_still_precedes_the_project() {
    let linear = Posting::answering([Ok(no_label()), Ok(label_created()), Ok(project_created())]);

    create_project(&linear, &brief()).expect("the stand-in answered");

    let asked = linear.documents();

    assert_eq!(asked.len(), 3);
    assert!(asked[0].contains("projectLabels"), "{asked:?}");
    assert!(asked[1].contains("projectLabelCreate"), "{asked:?}");
    assert!(
        asked[2].contains("projectCreate("),
        "the project was created before its label existed: {asked:?}"
    );
    assert_eq!(last_input(&linear)["labelIds"], json!(["label-made"]));
}

#[test]
fn a_label_that_cannot_be_resolved_stops_before_the_project_is_created() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Entity not found".to_owned(),
    })]);

    let error = create_project(&linear, &brief()).expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
    assert_eq!(
        linear.documents().len(),
        1,
        "nothing is created once the label has failed"
    );
}

#[test]
fn the_create_carries_the_name_content_team_status_and_label() {
    let linear = Posting::answering([Ok(label_found()), Ok(project_created())]);

    create_project(&linear, &brief().with_status(Some("status-backlog")))
        .expect("the stand-in answered");

    assert_eq!(
        last_input(&linear),
        json!({
            "name": "Repair the answer",
            "content": "# Repair the answer\n\nBody.\n",
            "teamIds": ["team-1"],
            "labelIds": ["label-held"],
            "statusId": "status-backlog",
        }),
        "no field warlock would have to invent"
    );
}

#[test]
fn a_create_with_no_status_leaves_the_field_out_entirely() {
    let linear = Posting::answering([Ok(label_found()), Ok(project_created())]);

    create_project(&linear, &brief()).expect("the stand-in answered");

    let input = last_input(&linear);

    assert!(
        input.get("statusId").is_none(),
        "a workspace with no `Backlog` sends no status at all: {input}"
    );
}

#[test]
fn a_created_project_comes_back_with_its_id_and_url() {
    let linear = Posting::answering([Ok(label_found()), Ok(project_created())]);

    let project = create_project(&linear, &brief()).expect("the stand-in answered");

    assert_eq!(project.id(), "project-1");
    assert_eq!(
        project.url(),
        "https://linear.app/acme/project/a-brief-1a2b3c"
    );
}

#[test]
fn a_create_that_answers_no_project_is_malformed() {
    for answer in [
        json!({ "projectCreate": { "project": null } }),
        json!({ "projectCreate": { "success": true } }),
        json!({ "projectCreate": { "project": { "id": "project-1" } } }),
    ] {
        let linear = Posting::answering([Ok(label_found()), Ok(answer.clone())]);

        let error = create_project(&linear, &brief()).expect_err("no project is no answer");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
}

fn issue_created() -> Value {
    json!({
        "issueCreate": {
            "issue": {
                "id": "1b9a5d2e-6c47-4f0a-9d31-0e7b2c4a8f55",
                "identifier": "WAR-125",
                "url": "https://linear.app/acme/issue/WAR-125/repair-the-answer",
            },
        },
    })
}

fn draft<'a>() -> NewIssue<'a> {
    NewIssue::new(
        "Repair the answer",
        "## Problem\n\nThe answer is wrong.\n",
        "team-1",
        "project-1",
        "issue-label-held",
        "state-backlog",
        "user-viewer",
    )
}

#[test]
fn a_created_issue_comes_back_with_its_id_identifier_and_url() {
    let linear = Posting::answering([Ok(issue_created())]);

    let issue = create_issue(&linear, &draft()).expect("the stand-in answered");

    // The id is what a relation is written with and the identifier is what the
    // cut record stores, so one create has to answer both.
    assert_eq!(issue.id(), "1b9a5d2e-6c47-4f0a-9d31-0e7b2c4a8f55");
    assert_eq!(issue.identifier(), "WAR-125");
    assert_eq!(
        issue.url(),
        "https://linear.app/acme/issue/WAR-125/repair-the-answer"
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn the_issue_create_carries_the_title_body_team_project_label_state_and_assignee_and_nothing_else()
{
    let linear = Posting::answering([Ok(issue_created())]);

    create_issue(&linear, &draft()).expect("the stand-in answered");

    assert_eq!(
        last_input(&linear),
        json!({
            "title": "Repair the answer",
            "description": "## Problem\n\nThe answer is wrong.\n",
            "teamId": "team-1",
            "projectId": "project-1",
            "labelIds": ["issue-label-held"],
            "stateId": "state-backlog",
            "assigneeId": "user-viewer",
        }),
        "no field warlock would have to invent"
    );
}

#[test]
fn an_issue_is_created_assigned_to_the_user_the_caller_resolved() {
    let linear = Posting::answering([Ok(issue_created())]);

    create_issue(&linear, &draft()).expect("the stand-in answered");

    // The claim `warlock pull` reads: an issue filed with no assignee is one it
    // can never select, so the id the caller resolved has to reach the wire
    // under the name Linear knows it by.
    assert_eq!(last_input(&linear)["assigneeId"], json!("user-viewer"));
}

#[test]
fn no_issue_create_invents_a_status_priority_estimate_cycle_or_milestone() {
    let linear = Posting::answering([Ok(issue_created())]);

    create_issue(&linear, &draft()).expect("the stand-in answered");

    let input = last_input(&linear);

    for invented in [
        "statusId",
        "priority",
        "priorityLabel",
        "estimate",
        "cycleId",
        "projectMilestoneId",
    ] {
        assert!(
            input.get(invented).is_none(),
            "the create invented `{invented}`: {input}"
        );
    }
    // The workflow state the issue is filed in is a team's, resolved by the
    // caller; a project status is not a thing an issue has and nothing on this
    // path moves one.
    let asked = linear.documents().pop().expect("one request was made");

    assert!(!asked.contains("projectStatus"), "{asked}");
    assert!(!asked.contains("projectUpdate"), "{asked}");
}

#[test]
fn an_issue_create_that_answers_nothing_usable_is_malformed() {
    for answer in [
        json!({ "issueCreate": { "issue": null } }),
        json!({ "issueCreate": { "success": true } }),
        json!({ "issueCreate": {} }),
        json!({ "issueCreate": { "issue": { "identifier": "WAR-125", "url": "u" } } }),
        json!({ "issueCreate": { "issue": { "id": "issue-1", "url": "u" } } }),
        json!({ "issueCreate": { "issue": { "id": "issue-1", "identifier": "WAR-125" } } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = create_issue(&linear, &draft()).expect_err("no issue is no answer");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

fn relation_created() -> Value {
    json!({ "issueRelationCreate": { "issueRelation": { "id": "relation-1" } } })
}

#[test]
fn a_relation_is_one_blocking_edge_written_in_one_request() {
    let linear = Posting::answering([Ok(relation_created())]);

    let relation =
        create_relation(&linear, "issue-first", "issue-second").expect("the stand-in answered");

    assert_eq!(relation, "relation-1");
    assert_eq!(
        last_input(&linear),
        json!({
            // `issueId` blocks `relatedIssueId`, so the blocker is the first
            // argument and a swap here inverts the slice's dependencies.
            "issueId": "issue-first",
            "relatedIssueId": "issue-second",
            "type": "blocks",
        })
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
    assert!(
        linear.documents()[0].contains("issueRelationCreate("),
        "{:?}",
        linear.documents()
    );
}

#[test]
fn the_board_hands_the_blocker_to_the_wire_first() {
    // The two ids are both `&str`, so a swap between the trait and the
    // operation under it compiles and inverts every dependency a cut writes.
    let board = Linear::new(Posting::answering([Ok(relation_created())]));

    board
        .create_relation("issue-first", "issue-second")
        .expect("the stand-in answered");

    assert_eq!(last_input(&board.posts)["issueId"], json!("issue-first"));
    assert_eq!(
        last_input(&board.posts)["relatedIssueId"],
        json!("issue-second")
    );
}

#[test]
fn a_relation_that_answers_nothing_usable_is_malformed() {
    for answer in [
        json!({ "issueRelationCreate": { "issueRelation": null } }),
        json!({ "issueRelationCreate": { "success": true } }),
        json!({ "issueRelationCreate": {} }),
        json!({ "issueRelationCreate": { "issueRelation": { "type": "blocks" } } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = create_relation(&linear, "issue-first", "issue-second")
            .expect_err("no relation is no answer");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

#[test]
fn a_relation_the_api_refuses_comes_back_in_linears_words() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Entity not found".to_owned(),
    })]);

    let error =
        create_relation(&linear, "issue-first", "issue-second").expect_err("the stand-in refused");

    // The caller turns this into one reported line rather than a failed slice,
    // which it can only do if the refusal arrives as Linear worded it.
    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
}

fn issue_updated() -> Value {
    json!({ "issueUpdate": { "issue": { "id": "issue-1" } } })
}

#[test]
fn an_issue_is_moved_into_a_state_by_id_in_one_request() {
    let linear = Posting::answering([Ok(issue_updated())]);

    let moved = move_issue(&linear, "issue-1", "state-doing").expect("the stand-in answered");

    assert_eq!(moved, "issue-1");
    assert_eq!(
        linear.variables(),
        [json!({ "id": "issue-1", "input": { "stateId": "state-doing" } })]
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_move_writes_the_state_and_nothing_else_about_the_issue() {
    let linear = Posting::answering([Ok(issue_updated())]);

    move_issue(&linear, "issue-1", "state-doing").expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");
    let input = last_input(&linear);

    assert!(asked.contains("issueUpdate("), "{asked}");
    // Whose ticket it is stays a human's decision, and a project's status is a
    // different type on a different object that no issue move may touch.
    assert!(input.get("assigneeId").is_none(), "{input}");
    assert!(input.get("statusId").is_none(), "{input}");
    assert_eq!(
        input.as_object().map(serde_json::Map::len),
        Some(1),
        "the state is the whole of what a move writes: {input}"
    );
    assert!(!asked.contains("projectUpdate"), "{asked}");
}

#[test]
fn a_move_that_answers_nothing_usable_is_malformed() {
    for answer in [
        json!({ "issueUpdate": { "issue": null } }),
        json!({ "issueUpdate": { "success": true } }),
        json!({ "issueUpdate": {} }),
        json!({ "issueUpdate": { "issue": { "identifier": "WAR-134" } } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = move_issue(&linear, "issue-1", "state-doing").expect_err("no issue");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

#[test]
fn a_move_the_api_refuses_comes_back_once_in_linears_words() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Entity not found".to_owned(),
    })]);

    // A retried move is how a board ends up disagreeing with itself, and a
    // column nobody could set is one reported line rather than a failed run.
    let error = move_issue(&linear, "issue-1", "state-doing").expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
    assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
}

fn comment_created() -> Value {
    json!({ "commentCreate": { "comment": { "id": "comment-1" } } })
}

#[test]
fn a_comment_is_written_on_the_project_by_id_in_one_request() {
    let linear = Posting::answering([Ok(comment_created())]);

    let comment = comment_on_project(&linear, "project-1", "Filed WAR-125. Status not moved.")
        .expect("the stand-in answered");

    assert_eq!(comment, "comment-1");
    assert_eq!(
        last_input(&linear),
        json!({
            "projectId": "project-1",
            "body": "Filed WAR-125. Status not moved.",
        })
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn a_project_comment_names_no_issue_and_moves_no_status() {
    let linear = Posting::answering([Ok(comment_created())]);

    comment_on_project(&linear, "project-1", "Filed WAR-125.").expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    // One comment mutation serves issues and projects both, told apart by the
    // id in the input, and a comment is the whole of what this operation does.
    assert!(asked.contains("commentCreate("), "{asked}");
    assert!(last_input(&linear).get("issueId").is_none());
    assert!(last_input(&linear).get("statusId").is_none());
    assert!(!asked.contains("projectUpdate"), "{asked}");
}

#[test]
fn a_comment_that_answers_nothing_usable_is_malformed() {
    for answer in [
        json!({ "commentCreate": { "comment": null } }),
        json!({ "commentCreate": { "success": true } }),
        json!({ "commentCreate": {} }),
        json!({ "commentCreate": { "comment": { "body": "Filed WAR-125." } } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error =
            comment_on_project(&linear, "project-1", "Filed WAR-125.").expect_err("no comment");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

#[test]
fn a_comment_is_written_on_the_issue_by_id_in_one_request() {
    let linear = Posting::answering([Ok(comment_created())]);

    let comment = comment_on_issue(&linear, "issue-1", "Halted: the tree was dirty.")
        .expect("the stand-in answered");

    assert_eq!(comment, "comment-1");
    assert_eq!(
        last_input(&linear),
        json!({
            "issueId": "issue-1",
            "body": "Halted: the tree was dirty.",
        })
    );
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn an_issue_comment_names_no_project_and_moves_no_state() {
    let linear = Posting::answering([Ok(comment_created())]);

    comment_on_issue(&linear, "issue-1", "Halted.").expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");
    let input = last_input(&linear);

    // One comment mutation serves issues and projects both, told apart by the id
    // in the input: a `projectId` here would comment on the wrong object in a
    // request that succeeds.
    assert!(asked.contains("commentCreate("), "{asked}");
    assert!(input.get("projectId").is_none(), "{input}");
    assert!(input.get("stateId").is_none(), "{input}");
    assert!(!asked.contains("issueUpdate"), "{asked}");
}

#[test]
fn an_issue_comment_that_answers_nothing_usable_is_malformed() {
    for answer in [
        json!({ "commentCreate": { "comment": null } }),
        json!({ "commentCreate": { "success": true } }),
        json!({ "commentCreate": {} }),
        json!({ "commentCreate": { "comment": { "body": "Halted." } } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = comment_on_issue(&linear, "issue-1", "Halted.").expect_err("no comment");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
        assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
    }
}

#[test]
fn an_issue_comment_the_api_refuses_comes_back_once_in_linears_words() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Entity not found".to_owned(),
    })]);

    // A create is not idempotent: a retry here is how one halt gets explained
    // twice on the same ticket.
    let error = comment_on_issue(&linear, "issue-1", "Halted.").expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
    assert_eq!(linear.documents().len(), 1, "no retry and no backoff");
}
