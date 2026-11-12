use crate::bootstrap::branch::{
    PullState,
    forge::{parse_list, parse_view},
};

// `gh pr list --head coeus/bootstrap/nova --state all --json
// number,url,isDraft,state,body`, as gh prints it without a terminal
const LISTED: &str = r#"[{"body":"adds nova\n\n<!-- coeus:bootstrap -->\n…\n<!-- /coeus:bootstrap -->\n","isDraft":true,"number":42,"state":"OPEN","url":"https://github.com/example/nix/pull/42"},{"body":"","isDraft":false,"number":17,"state":"CLOSED","url":"https://github.com/example/nix/pull/17"}]
"#;

const VIEWED: &str = r#"{"body":"adds nova\n","isDraft":false,"number":42,"state":"MERGED","url":"https://github.com/example/nix/pull/42"}
"#;

#[test]
fn the_newest_pull_request_of_the_branch_is_found() {
    let found = parse_list(LISTED).unwrap().unwrap();

    assert_eq!(found.number, 42);
    assert_eq!(found.url, "https://github.com/example/nix/pull/42");
    assert!(found.draft);
    assert_eq!(found.state, PullState::Open);
    assert!(
        found
            .body
            .starts_with("adds nova\n\n<!-- coeus:bootstrap -->")
    );
}

#[test]
fn a_branch_without_pull_requests_has_none() {
    assert_eq!(parse_list("[]\n").unwrap(), None);
}

#[test]
fn a_viewed_pull_request_reads_its_state() {
    let viewed = parse_view(VIEWED).unwrap();

    assert_eq!(viewed.state, PullState::Merged);
    assert!(!viewed.draft);
}

#[test]
fn an_unknown_state_is_refused() {
    let odd = VIEWED.replace("MERGED", "LOCKED");

    assert!(parse_view(&odd).is_err());
}
