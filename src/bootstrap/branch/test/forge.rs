use crate::bootstrap::branch::{
    PullState,
    forge::{parse_list, parse_view},
};

// `gh pr list --head coeus/bootstrap/nova --state all --json
// number,url,isDraft,state,body,headRefOid,isCrossRepository`, as gh prints
// it without a terminal
const LISTED: &str = r#"[{"body":"adds nova\n\n<!-- coeus:bootstrap -->\n…\n<!-- /coeus:bootstrap -->\n","headRefOid":"4f1d2a0c9b8e7d6c5b4a39281706f5e4d3c2b1a0","isCrossRepository":false,"isDraft":true,"number":42,"state":"OPEN","url":"https://github.com/example/nix/pull/42"},{"body":"","headRefOid":"0a1b2c3d4e5f60718293a4b5c6d7e8f901234567","isCrossRepository":false,"isDraft":false,"number":17,"state":"CLOSED","url":"https://github.com/example/nix/pull/17"}]
"#;

const VIEWED: &str = r#"{"body":"adds nova\n","headRefOid":"4f1d2a0c9b8e7d6c5b4a39281706f5e4d3c2b1a0","isCrossRepository":false,"isDraft":false,"number":42,"state":"MERGED","url":"https://github.com/example/nix/pull/42"}
"#;

#[test]
fn the_newest_pull_request_of_the_branch_is_found() {
    let found = parse_list(LISTED).unwrap().unwrap();

    assert_eq!(found.number, 42);
    assert_eq!(found.url, "https://github.com/example/nix/pull/42");
    assert!(found.draft);
    assert_eq!(found.state, PullState::Open);
    assert_eq!(
        found.head.to_string(),
        "4f1d2a0c9b8e7d6c5b4a39281706f5e4d3c2b1a0"
    );
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
fn a_pull_request_from_a_fork_with_the_same_branch_name_is_not_ours() {
    let fork = r#"{"body":"","headRefOid":"ffffffffffffffffffffffffffffffffffffffff","isCrossRepository":true,"isDraft":false,"number":99,"state":"OPEN","url":"https://github.com/example/nix/pull/99"}"#;
    let listed = LISTED.replacen('[', &format!("[{fork},"), 1);

    assert_eq!(parse_list(&listed).unwrap().unwrap().number, 42);
    let only_the_fork = format!("[{fork}]");
    assert_eq!(parse_list(&only_the_fork).unwrap(), None);
}

#[test]
fn a_pull_request_closed_without_merging_is_left_alone() {
    let closed = LISTED.replacen("\"OPEN\"", "\"CLOSED\"", 1);
    assert_eq!(parse_list(&closed).unwrap(), None);

    let merged = LISTED.replacen("\"OPEN\"", "\"MERGED\"", 1);
    assert_eq!(parse_list(&merged).unwrap().unwrap().number, 42);
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
