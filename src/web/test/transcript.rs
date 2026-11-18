use super::ORDERED;
use crate::{
    common::model::{
        AttachFrame, ClosureDiff, Failure, JobEvent, JobId, Outcome, PackageChange, Phase, Stamped,
    },
    web::{
        transcript::Transcript,
        view::{LineKind, LogLine},
    },
};

const BUILD: &str = include_str!("../../nix/fixtures/build.jsonl");
const DOWNLOAD: &str = include_str!("../../nix/fixtures/download.jsonl");
const FAILING: &str = include_str!("../../nix/fixtures/failing.jsonl");

fn read(log: &str) -> Vec<LogLine> {
    Transcript::default().lines(&[AttachFrame::Nix(log.as_bytes().to_vec())])
}

fn kinds(lines: &[LogLine]) -> Vec<(LineKind, Option<&str>, &str)> {
    lines
        .iter()
        .map(|line| (line.kind, line.prefix.as_deref(), line.text.as_str()))
        .collect()
}

#[test]
fn a_download_reads_as_what_nix_fetches() {
    assert_eq!(
        kinds(&read(DOWNLOAD)),
        [
            (
                LineKind::Note,
                None,
                "these 2 paths will be fetched (1.3 MiB download, 43.8 MiB unpacked):"
            ),
            (
                LineKind::Note,
                None,
                "  /nix/store/fv6rgzamfwj334kf1xv8ms3y6x8nvirn-hello-2.12.3"
            ),
            (
                LineKind::Note,
                None,
                "  /nix/store/jaixg82zxmqjbn2s6grpd31jq0yygzc1-libiconv-115.100.1"
            ),
            (
                LineKind::Fetch,
                None,
                "copying path '/nix/store/jaixg82zxmqjbn2s6grpd31jq0yygzc1-libiconv-115.100.1' from 'https://cache.nixos.org'"
            ),
            (
                LineKind::Fetch,
                None,
                "copying path '/nix/store/fv6rgzamfwj334kf1xv8ms3y6x8nvirn-hello-2.12.3' from 'https://cache.nixos.org'"
            ),
        ]
    );
}

#[test]
fn a_build_names_the_builder_of_each_output_line() {
    assert_eq!(
        kinds(&read(BUILD)),
        [
            (LineKind::Note, None, "this derivation will be built:"),
            (
                LineKind::Note,
                None,
                "  /nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-coeus-fixture-1794443502.drv"
            ),
            (
                LineKind::Error,
                None,
                "ssh: connect to host builder port 22: Operation timed out"
            ),
            (LineKind::Error, None, "SSH master stdout first line:"),
            (
                LineKind::Error,
                None,
                "cannot build on 'ssh-ng://build@builder': error: failed to start SSH master connection to 'builder'"
            ),
            (
                LineKind::Build,
                None,
                "building '/nix/store/9rfvhi1nqszqmny102jqxp2vmb6n0mk7-coeus-fixture-1794443502.drv'"
            ),
            (LineKind::Output, Some("coeus-fixture-1794443502"), "step 1"),
            (LineKind::Output, Some("coeus-fixture-1794443502"), "step 2"),
            (LineKind::Output, Some("coeus-fixture-1794443502"), "step 3"),
        ]
    );
}

#[test]
fn a_failed_build_ends_in_nix_s_error_without_its_colors() {
    let lines = read(FAILING);

    let builders: Vec<&str> = lines
        .iter()
        .filter(|line| line.kind == LineKind::Output)
        .filter_map(|line| line.prefix.as_deref())
        .collect();
    assert!(builders.contains(&"coeus-count-1794500001"));
    assert!(builders.contains(&"coeus-greet-1794500001"));
    assert!(builders.contains(&"coeus-broken-1794500001"));

    let errors: Vec<&str> = lines
        .iter()
        .filter(|line| line.kind == LineKind::Error)
        .map(|line| line.text.as_str())
        .collect();
    assert!(errors.contains(
        &"error: Cannot build '/nix/store/g1w6lisiqps834vb8k4nkvlkihfsrr4b-coeus-broken-1794500001.drv'."
    ));
    assert!(errors.contains(&"       Reason: builder failed with exit code 1."));

    for line in &lines {
        assert!(!line.text.contains('\u{1b}'), "{line:?}");
        assert!(!line.text.starts_with("@nix"), "{line:?}");
    }
}

#[test]
fn a_log_read_in_batches_reads_as_a_whole() {
    let frames: Vec<AttachFrame> = FAILING
        .lines()
        .map(|line| AttachFrame::Nix(format!("{line}\n").into_bytes()))
        .collect();
    let mut transcript = Transcript::default();
    let batched: Vec<LogLine> = frames
        .chunks(7)
        .flat_map(|batch| transcript.lines(batch))
        .collect();

    assert_eq!(batched, read(FAILING));
}

#[test]
fn a_log_resumed_mid_build_leaves_its_builder_unnamed() {
    let resumed: String = FAILING
        .lines()
        .skip_while(|line| !line.contains("broken line 1\""))
        .map(|line| format!("{line}\n"))
        .collect();

    let first = &read(&resumed)[0];

    assert_eq!(
        (first.kind, first.prefix.as_deref(), first.text.as_str()),
        (LineKind::Output, None, "broken line 1")
    );
}

#[test]
fn coeus_events_read_as_its_own_lines() {
    let at = ORDERED;
    let failure = Failure::new(Phase::Building, "builder for 'zfs.drv' failed");
    let diff = ClosureDiff {
        changes: vec![PackageChange {
            name: "firefox".into(),
            before: vec![],
            after: vec!["121.0".into()],
            size_delta: 0,
        }],
    };
    let cases = [
        (
            JobEvent::Started {
                attempt: 2,
                pid: 4242,
                version: "0.6.0".into(),
            },
            Some((LineKind::Coeus, "runner started (coeus 0.6.0, attempt 2)")),
        ),
        (
            JobEvent::Phase(Phase::Evaluating),
            Some((LineKind::Phase, "evaluating")),
        ),
        (
            JobEvent::Evaluated {
                drv: "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11.drv"
                    .parse()
                    .unwrap(),
                toplevel: "/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-nixos-system-lab-26.11"
                    .parse()
                    .unwrap(),
            },
            Some((LineKind::Coeus, "system nixos-system-lab-26.11")),
        ),
        (JobEvent::Progress(Default::default()), None),
        (
            JobEvent::Diff(ClosureDiff::default()),
            Some((LineKind::Coeus, "no package changes")),
        ),
        (
            JobEvent::Diff(diff),
            Some((LineKind::Coeus, "1 added · 0 B")),
        ),
        (
            JobEvent::Note("the system already runs this generation".into()),
            Some((LineKind::Note, "the system already runs this generation")),
        ),
        (
            JobEvent::Finished(Outcome::Superseded { by: JobId(9) }),
            Some((LineKind::Done, "superseded by #9")),
        ),
        (
            JobEvent::Finished(Outcome::Failed(failure)),
            Some((
                LineKind::Error,
                "failed while building: builder for 'zfs.drv' failed",
            )),
        ),
    ];

    for (event, expected) in cases {
        let frame = AttachFrame::Event(Stamped {
            at,
            event: event.clone(),
        });
        let lines = Transcript::default().lines(&[frame]);
        let expected: Vec<LogLine> = expected
            .into_iter()
            .map(|(kind, text)| LogLine {
                at: Some(at),
                kind,
                text: text.into(),
                prefix: None,
            })
            .collect();
        assert_eq!(lines, expected, "{event:?}");
    }
}
