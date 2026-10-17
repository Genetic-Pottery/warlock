use warlock_engine::ScopeRecord;

use super::{Boards, Opener, Opens};

/// Shaped like neither board's, because nothing is sent: what is asserted here
/// is which board was built, and both clients hold whatever line they were
/// given without reading it.
const NOT_A_KEY: &str = "this-is-not-a-key";

#[test]
fn a_record_filing_to_linear_opens_the_linear_board() {
    let record = ScopeRecord::new("data-plane", "WAR", "In Review", "area/data");

    assert!(
        matches!(Opener.open(&record, NOT_A_KEY), Boards::Linear(_)),
        "the record files to Linear",
    );
}

#[test]
fn a_record_filing_to_jira_opens_the_jira_board() {
    let record = ScopeRecord::jira(
        "payments",
        "https://example.atlassian.net",
        "PAY",
        "In Review",
        "area/pay",
    );

    assert!(
        matches!(Opener.open(&record, NOT_A_KEY), Boards::Jira(_)),
        "the record files to Jira",
    );
}
