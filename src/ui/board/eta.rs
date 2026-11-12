use std::time::Duration;

use crate::common::model::{JobState, JobView, MachineView, Outcome, Phase, Timestamp};

/// how long `job` probably still runs: the median of the machine's recent
/// successful jobs of the same kind, else an extrapolation of the build's
/// progress
pub fn eta(view: &MachineView, job: &JobView, now: Timestamp) -> Option<Duration> {
    let started = job.summary.started_at?;
    let elapsed = started.until(now);

    match typical(view, job) {
        Some(typical) => typical
            .checked_sub(elapsed)
            .filter(|left| left.as_secs() > 0),
        None => extrapolate(job, now),
    }
}

fn typical(view: &MachineView, job: &JobView) -> Option<Duration> {
    let mut durations: Vec<Duration> = view
        .recent
        .iter()
        .filter(|recent| {
            recent.state == JobState::Finished(Outcome::Succeeded)
                && recent.action == job.summary.action
                && recent.source.builds() == job.summary.source.builds()
        })
        .filter_map(|recent| Some(recent.started_at?.until(recent.finished_at?)))
        .collect();

    if durations.is_empty() {
        return None;
    }

    durations.sort_unstable();
    Some(durations[durations.len() / 2])
}

/// building time so far, scaled by what is left; only once enough is done
/// for the rate to mean something
pub fn extrapolate(job: &JobView, now: Timestamp) -> Option<Duration> {
    if job.summary.state.phase() != Some(Phase::Building) {
        return None;
    }

    let fraction = job
        .progress
        .fraction()
        .filter(|fraction| *fraction >= 0.05)?;
    let building = job.phase_started_at?.until(now);
    Some(building.mul_f64((1.0 - fraction) / fraction))
}
