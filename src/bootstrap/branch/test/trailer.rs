use super::host;
use crate::bootstrap::branch::{
    git::status_paths,
    trailer::{Trailer, parse_log},
};

const A: &str = "1111111111111111111111111111111111111111";
const B: &str = "2222222222222222222222222222222222222222";
const C: &str = "3333333333333333333333333333333333333333";

#[test]
fn the_log_reads_this_hosts_trailers_only() {
    let log = format!(
        "{A}\x1fchore(nova): rekey\x1frekey nova\x1e\n\
         {B}\x1ffix: unrelated\x1f\x1e\n\
         {C}\x1ffeat(nova): join\x1fenroll vega\x1denroll nova\x1e\n"
    );

    let commits = parse_log(&log, &host()).unwrap();

    assert_eq!(commits.len(), 3);
    assert_eq!(commits[0].commit.to_string(), A);
    assert_eq!(commits[0].subject, "chore(nova): rekey");
    assert_eq!(commits[0].trailers, [Trailer::Rekey]);
    assert_eq!(commits[1].trailers, []);
    assert_eq!(commits[2].trailers, [Trailer::Enroll]);
}

#[test]
fn a_trailer_line_names_the_host() {
    assert_eq!(
        Trailer::Enroll.line(&host()),
        "Coeus-Bootstrap: enroll nova"
    );
    assert_eq!(Trailer::Rekey.line(&host()), "Coeus-Bootstrap: rekey nova");
}

#[test]
fn status_lists_each_changed_path_once() {
    let status = " M readme.md\0R  new.nix\0old.nix\0?? hosts/nova/a b\0";

    assert_eq!(
        status_paths(status),
        ["readme.md", "new.nix", "hosts/nova/a b"]
    );
}
