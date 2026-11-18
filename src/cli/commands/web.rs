use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
};

use axum::Router;
use clap::Args;
use eyre::{Result, WrapErr, bail};
use log::info;
use tokio::{
    net::UnixListener,
    signal::unix::{SignalKind, signal},
};

use crate::{
    cli::{exit::Exit, local_hostname},
    client::{Fleet, Targets},
    common::model::Hostname,
    config::WebSettings,
    web::{
        site::{Access, Settings, Site},
        source::Source,
    },
};

#[derive(Args, Debug)]
pub struct WebArgs {
    /// The built web app, served for every path outside /api
    #[arg(long, value_name = "DIR", env = "COEUS_WEB_DIR")]
    pub web_dir: Option<PathBuf>,
}

impl WebArgs {
    /// serves the fleet named in `config`, or in /etc/coeus/web.toml, on its
    /// origin socket
    pub async fn run(self, config: Option<&Path>) -> Result<Exit> {
        let settings = WebSettings::load(config.unwrap_or(Path::new(WebSettings::PATH)))?;
        let key = settings.client.key.load()?;
        let targets = Targets::select(&["@all".to_owned()], &settings.client)?;
        if targets.hosts().is_empty() {
            bail!("no machines in the inventory to show");
        }

        let fleet = Fleet::new(settings.client.clone(), key, targets);
        let source = Source::Live(fleet.clone());
        let site = Site::start(
            fleet.hosts().to_vec(),
            source.clone(),
            Settings {
                web_dir: self.web_dir,
                served_by: local_hostname()
                    .parse::<Hostname>()
                    .wrap_err("this machine's hostname is not a valid name")?,
                access: Access::Closed,
            },
        );
        source.feed(site.inputs.clone());

        let socket = &settings.web.origin_socket;
        let listener = bind(socket)?;
        info!("serving on {}", socket.display());
        serve(listener, site.router).await?;
        let _ = std::fs::remove_file(socket);
        Ok(Exit::Success)
    }
}

/// binds `socket` for its owner alone, replacing a socket a previous run left
fn bind(socket: &Path) -> Result<UnixListener> {
    if std::fs::symlink_metadata(socket).is_ok_and(|meta| meta.file_type().is_socket()) {
        std::fs::remove_file(socket)
            .wrap_err_with(|| format!("failed to remove {}", socket.display()))?;
    }
    let listener = UnixListener::bind(socket)
        .wrap_err_with(|| format!("failed to listen on {}", socket.display()))?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))
        .wrap_err_with(|| format!("failed to restrict {}", socket.display()))?;
    Ok(listener)
}

/// serves until SIGTERM or SIGINT
async fn serve<L>(listener: L, router: Router) -> Result<()>
where
    L: axum::serve::Listener,
    L::Addr: std::fmt::Debug,
{
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::select! {
        served = axum::serve(listener, router) => served.wrap_err("the server stopped"),
        _ = terminate.recv() => Ok(()),
        _ = interrupt.recv() => Ok(()),
    }
}
