use colored::Colorize;

use crate::{
    common::model::{Hostname, JobId},
    ui::Theme,
};

/// something `top` asks of one machine, once confirmed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub host: Hostname,
    pub intent: Intent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// stop the running job, or withdraw the queued one
    Cancel {
        job: JobId,
    },
    /// switch back to the generation before `from`, the running one
    Rollback {
        from: Option<u32>,
    },
    Reboot,
    /// order `main`, as `coeus deploy` does
    Deploy,
}

impl Request {
    /// the branch `d` deploys
    pub const DEPLOY_REF: &'static str = "main";

    /// what the confirmation asks
    pub fn question(&self) -> String {
        let host = Theme::host(self.host.as_str());
        match &self.intent {
            Intent::Cancel { job } => format!("cancel job {} on {host}?", job.to_string().bold()),
            Intent::Rollback { from: Some(from) } => format!(
                "roll {host} back to the generation before {}?",
                from.to_string().bold()
            ),
            Intent::Rollback { from: None } => {
                format!("roll {host} back to its previous generation?")
            }
            Intent::Reboot => format!("reboot {host} now?"),
            Intent::Deploy => format!(
                "deploy {} to {host} and switch to it?",
                Self::DEPLOY_REF.cyan().bold()
            ),
        }
    }

    /// what the status line says while the machine answers
    pub fn pending(&self) -> String {
        let host = Theme::host(self.host.as_str());
        match &self.intent {
            Intent::Cancel { job } => format!("cancelling job {job} on {host}…"),
            Intent::Rollback { .. } => format!("rolling {host} back…"),
            Intent::Reboot => format!("asking {host} to reboot…"),
            Intent::Deploy => format!("ordering {} on {host}…", Self::DEPLOY_REF),
        }
    }

    /// the actions that cannot be taken back get a louder confirmation
    pub const fn is_drastic(&self) -> bool {
        matches!(self.intent, Intent::Reboot | Intent::Rollback { .. })
    }
}
