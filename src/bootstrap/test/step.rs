use crate::bootstrap::Step;

#[test]
fn steps_run_in_the_checklist_order() {
    let titles: Vec<&str> = Step::ALL.iter().map(|step| step.title()).collect();
    assert_eq!(
        titles,
        [
            "reach", "nix", "enroll", "rekey", "push", "switch", "answer", "reboot", "review",
            "cleanup"
        ]
    );
    assert!(Step::ALL.is_sorted());
    assert_eq!(Step::Cleanup.to_string(), "cleanup");
}
