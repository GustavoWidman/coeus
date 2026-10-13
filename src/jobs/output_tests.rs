//! Real pipes/private durable ledgers; shell printf fixtures only, never nh.
use super::{
    DeploymentJobs,
    process_tests::{ShellFixture, finished, output},
    tests::request,
};
use crate::common::proto::{DeployPhase, LogSource};
use eyre::Result;
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

fn fixture(script: &'static str) -> Arc<ShellFixture> {
    Arc::new(ShellFixture {
        script,
        deadline: Duration::from_secs(2),
    })
}

fn assert_same_output(actual: &[(LogSource, String)], expected: &[(LogSource, String)]) {
    assert_eq!(actual.len(), expected.len());
    for ((source, text), (expected_source, expected_text)) in actual.iter().zip(expected) {
        assert_eq!(
            std::mem::discriminant(source),
            std::mem::discriminant(expected_source)
        );
        assert_eq!(text, expected_text);
    }
}

#[tokio::test]
async fn pipe_progress_both_sources_and_eof_tail_survive_reattach_restart() -> Result<()> {
    timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir()?;
        let state = directory.path().join("state");
        let executor = fixture(r"printf '\033[32mBuilding\033[0m\r\033[2KDone\r\n'; printf '\033]0;hidden title\007warning\r\n' >&2; printf '\033Pprivate string\033\\tail é🦀'");
        let jobs = DeploymentJobs::open(executor.clone(), state.clone(), "output-fixture".into()).await?;
        let original = request("output");
        let id = jobs.accept(original.clone()).await?;
        assert_eq!(finished(&jobs, id).await?, DeployPhase::Succeeded);
        let before = output(&jobs, id).await?;
        let stdout: Vec<_> = before.iter().filter(|(s, _)| matches!(s, LogSource::Stdout)).map(|(_, t)| t.as_str()).collect();
        let stderr: Vec<_> = before.iter().filter(|(s, _)| matches!(s, LogSource::Stderr)).map(|(_, t)| t.as_str()).collect();
        assert_eq!(stdout, ["Building", "Done", "tail é🦀"]);
        assert_eq!(stderr, ["warning"]);
        assert!(before.iter().all(|(_, t)| !t.contains('\x1b') && !t.contains('\r') && t.len() <= 1024));
        jobs.shutdown().await;
        drop(jobs);
        let recovered = DeploymentJobs::open(executor, state, "output-fixture".into()).await?;
        assert_eq!(recovered.accept(original).await?, id);
        assert_same_output(&output(&recovered, id).await?, &before);
        let page = recovered.logs(id, 0).await?;
        assert!(page.finished);
        assert_eq!(recovered.logs(id, page.next_cursor).await?.entries.len(), 0);
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    }).await??;
    Ok(())
}

#[tokio::test]
async fn cancellation_during_incomplete_escape_does_not_block_drain_or_terminal_log() -> Result<()>
{
    timeout(Duration::from_secs(5), async {
        let directory = tempfile::tempdir()?;
        let state = directory.path().join("state");
        let executor = fixture(r"printf 'ready\r\033]unterminated'; sleep 30");
        let jobs =
            DeploymentJobs::open(executor.clone(), state.clone(), "output-cancel".into()).await?;
        let id = jobs.accept(request("cancel-output")).await?;
        loop {
            if output(&jobs, id)
                .await?
                .iter()
                .any(|(s, t)| matches!(s, LogSource::Stdout) && t == "ready")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        jobs.cancel(id).await?;
        assert_eq!(finished(&jobs, id).await?, DeployPhase::Cancelled);
        let before = output(&jobs, id).await?;
        assert!(
            before
                .iter()
                .all(|(_, t)| !t.contains("unterminated") && !t.contains('\x1b'))
        );
        assert!(before.last().unwrap().1.starts_with("Cancelled — "));
        jobs.shutdown().await;
        drop(jobs);
        let recovered = DeploymentJobs::open(executor, state, "output-cancel".into()).await?;
        assert_same_output(&output(&recovered, id).await?, &before);
        assert_eq!(
            recovered.status(Some(id)).await?[0].phase,
            DeployPhase::Cancelled
        );
        recovered.shutdown().await;
        Ok::<_, eyre::Report>(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn normalized_progress_still_obeys_retained_chunk_and_page_bounds() -> Result<()> {
    let jobs = DeploymentJobs::new(fixture(
        r#"i=0; while [ $i -lt 300 ]; do printf '\033[32mupdate%s\033[0m\r' "$i"; i=$((i+1)); done"#,
    ));
    let id = jobs.accept(request("bounded-progress")).await?;
    assert_eq!(finished(&jobs, id).await?, DeployPhase::Succeeded);
    let page = jobs.logs(id, 0).await?;
    assert!(page.gap);
    assert_eq!(page.entries.len(), 32);
    let all = output(&jobs, id).await?;
    assert_eq!(all.len(), 256);
    assert!(
        all.iter()
            .all(|(_, text)| !text.contains("[32m") && !text.contains('\r'))
    );
    assert!(all.iter().any(|(_, text)| text == "update299"));
    jobs.shutdown().await;
    Ok(())
}
