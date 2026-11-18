use std::{
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
};

use axum::Router;
use clap::Args;
use eyre::{Result, WrapErr, bail};
use log::info;
use tokio::{
    net::{TcpListener, UnixListener},
    signal::unix::{SignalKind, signal},
};

use crate::{
    cli::{exit::Exit, local_hostname},
    client::{Fleet, Targets},
    common::model::Hostname,
    config::WebSettings,
    web::{
        fixture::{Fixture, Scenario},
        site::{Access, Settings, Site},
        source::Source,
    },
};

#[derive(Args, Debug)]
pub struct WebArgs {
    /// The built web app, served for every path outside /api
    #[arg(long, value_name = "DIR", env = "COEUS_WEB_DIR")]
    pub web_dir: Option<PathBuf>,

    /// Serve a simulated fleet, open to anyone, instead of the real one; POST
    /// /fixture/seek?t=<seconds>[&speed=<x>] moves its clock
    #[arg(long, value_name = "SCENARIO")]
    pub fixture: Option<Scenario>,

    /// Where a simulated fleet is served
    #[arg(
        long,
        value_name = "ADDRESS",
        default_value = "127.0.0.1:7979",
        requires = "fixture"
    )]
    pub listen: SocketAddr,

    /// How fast a simulated fleet's clock runs; 0 holds it
    #[arg(long, default_value_t = 1.0, requires = "fixture")]
    pub speed: f64,
}

impl WebArgs {
    /// serves the fleet named in `config`, or in /etc/coeus/web.toml, on its
    /// origin socket, or a simulated one on a local port
    pub async fn run(self, config: Option<&Path>) -> Result<Exit> {
        if let Some(scenario) = self.fixture {
            return self.simulate(scenario).await;
        }

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

    /// serves a simulated fleet, which never dials a coeusd
    async fn simulate(self, scenario: Scenario) -> Result<Exit> {
        let fixture = Fixture::new(scenario, self.speed);
        let source = Source::Fixture(fixture.clone());
        let site = Site::start(
            fixture.hosts(),
            source.clone(),
            Settings {
                web_dir: self.web_dir,
                served_by: "lab".parse()?,
                access: Access::Open(Fixture::viewer()),
            },
        );
        source.feed(site.inputs.clone());

        let listener = TcpListener::bind(self.listen)
            .await
            .wrap_err_with(|| format!("failed to listen on {}", self.listen))?;
        info!("serving the {scenario:?} fixture on http://{}", self.listen);
        serve(listener, site.router.merge(fixture.routes())).await?;
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
