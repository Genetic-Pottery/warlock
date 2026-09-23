use std::collections::VecDeque;
use std::error::Error as _;
use std::io;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::{
    BACKLOG, Board, Client, ENDPOINT, Error, Linear, NewIssue, NewProject, Posts, REQUEST_TIMEOUT,
    answer, authorization, backlog_state, backlog_status, comment_on_project, create_issue,
    create_project, create_relation, fetch_project, issue_label_id, label_id, team_id,
};

const KEY: &str = "lin_api_a_key_nobody_holds_8f3a1c";

// A Linear that answers from memory: it hands back what it was given, in the
// order it was given, and keeps every document it was asked so that a test can
// say which call came first. Answers are popped rather than cloned because
// `Error` is not `Clone` — carrying a refusal that could only happen once is the
// point, not a limitation.
#[derive(Debug, Default)]
struct Posting {
    answers: Mutex<VecDeque<Result<Value, Error>>>,
    asked: Mutex<Vec<(String, Value)>>,
}

impl Posting {
    fn answering(answers: impl IntoIterator<Item = Result<Value, Error>>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn documents(&self) -> Vec<String> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .iter()
            .map(|(document, _)| document.clone())
            .collect()
    }

    fn variables(&self) -> Vec<Value> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .iter()
            .map(|(_, variables)| variables.clone())
            .collect()
    }
}

impl Posts for Posting {
    fn post(&self, document: &str, variables: Value) -> Result<Value, Error> {
        self.asked
            .lock()
            .expect("the stand-in was not used across a panic")
            .push((document.to_owned(), variables));

        self.answers
            .lock()
            .expect("the stand-in was not used across a panic")
            .pop_front()
            .unwrap_or_else(|| panic!("the stand-in was asked more times than it was answered"))
    }
}

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

const PROJECT: &str = "65fcabef-373b-4c2e-82bc-3e98fe7accbe";

fn project_on_the_board() -> Value {
    json!({
        "project": {
            "name": "Push a brief to the board",
            "content": "# Push a brief to the board\n\n## Scope\n",
            "url": "https://linear.app/acme/project/a-brief-1a2b3c",
            "status": { "name": "Planned" },
        },
    })
}

#[test]
fn a_project_is_read_back_by_id_in_one_request() {
    let linear = Posting::answering([Ok(project_on_the_board())]);

    let project = fetch_project(&linear, PROJECT)
        .expect("the stand-in answered")
        .expect("the stand-in knows the project");

    assert_eq!(project.name(), "Push a brief to the board");
    assert_eq!(
        project.content(),
        "# Push a brief to the board\n\n## Scope\n"
    );
    assert_eq!(
        project.url(),
        "https://linear.app/acme/project/a-brief-1a2b3c"
    );
    assert_eq!(project.status(), Some("Planned"));
    assert_eq!(linear.variables(), [json!({ "id": PROJECT })]);
    assert_eq!(linear.documents().len(), 1, "one request per operation");
}

#[test]
fn the_id_is_the_only_selector_and_nothing_is_listed() {
    let linear = Posting::answering([Ok(project_on_the_board())]);

    fetch_project(&linear, PROJECT).expect("the stand-in answered");

    let asked = linear.documents().pop().expect("one request was made");

    assert!(asked.contains("project(id: $id)"), "{asked}");
    // A workspace walk and a name match are what this operation exists not to
    // be: brief 22 is on this repository's board twice under one title.
    assert!(!asked.contains("projects("), "{asked}");
    assert!(!asked.contains("filter"), "{asked}");
    assert!(!asked.contains("first:"), "{asked}");
}

#[test]
fn a_project_id_the_api_does_not_know_is_a_none_rather_than_an_error() {
    // Both shapes an unknown id can arrive as: Linear's own refusal, and the
    // null node a nullable field would give.
    let answers = [
        Err(Error::Refused {
            message: "Entity not found - could not find referenced Project.".to_owned(),
        }),
        Ok(json!({ "project": null })),
    ];

    for answer in answers {
        let linear = Posting::answering([answer]);

        let project = fetch_project(&linear, PROJECT).expect("an unknown id is an ordinary answer");

        assert_eq!(project, None, "the caller names the id and the file");
    }
}

#[test]
fn any_other_refusal_is_still_linears_to_word() {
    let linear = Posting::answering([Err(Error::Refused {
        message: "Access denied".to_owned(),
    })]);

    let error = fetch_project(&linear, PROJECT).expect_err("the stand-in refused");

    assert!(matches!(error, Error::Refused { .. }), "{error:?}");
}

#[test]
fn a_project_with_no_status_comes_back_without_one() {
    let mut answer = project_on_the_board();
    answer["project"]["status"] = Value::Null;
    let linear = Posting::answering([Ok(answer)]);

    let project = fetch_project(&linear, PROJECT)
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

    let project = fetch_project(&linear, PROJECT)
        .expect("an emptied description is an ordinary answer")
        .expect("the stand-in knows the project");

    assert_eq!(project.content(), "");
}

#[test]
fn a_project_answer_missing_a_field_is_malformed() {
    for field in ["name", "content", "url", "status"] {
        let mut answer = project_on_the_board();
        answer["project"]
            .as_object_mut()
            .expect("the fixture is an object")
            .remove(field);

        let linear = Posting::answering([Ok(answer)]);

        let error =
            fetch_project(&linear, PROJECT).expect_err("a field that was asked for is answered");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{field}: {error:?}"
        );
    }
}

#[test]
fn a_status_with_no_name_and_an_answer_with_no_project_are_malformed() {
    for answer in [
        json!({ "project": { "name": "A", "content": "", "url": "u", "status": {} } }),
        json!({ "projects": { "nodes": [] } }),
    ] {
        let linear = Posting::answering([Ok(answer.clone())]);

        let error = fetch_project(&linear, PROJECT).expect_err("that is not the answer asked for");

        assert!(
            matches!(error, Error::Malformed { .. }),
            "{answer}: {error:?}"
        );
    }
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
fn the_issue_create_carries_the_title_body_team_project_label_and_state_and_nothing_else() {
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
        }),
        "no field warlock would have to invent"
    );
}

#[test]
fn no_issue_create_invents_a_status_assignee_priority_estimate_cycle_or_milestone() {
    let linear = Posting::answering([Ok(issue_created())]);

    create_issue(&linear, &draft()).expect("the stand-in answered");

    let input = last_input(&linear);

    for invented in [
        "statusId",
        "assigneeId",
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
