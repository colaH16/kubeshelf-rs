//! Process entry point (Go `cmd/kubeshelf/main.go`).
//!
//! Startup order matches Go: configuration → store (mounted settings + Git,
//! bounded to 60 s) → Kubernetes source → background tasks → HTTP server.
//! SIGTERM/SIGINT stop accepting connections, give in-flight requests up to
//! 10 s, then abort the background tasks (their Git child processes are
//! killed on drop).

use anyhow::{Context, Result, ensure};
use kubeshelf_rs::{
    auth::{Auth, Directory},
    config::Config,
    http::{self, App, VERSION},
    source::{Source, demo},
    store::Store,
};
use std::{sync::Arc, time::Duration};
use tokio::signal::unix::{SignalKind, signal};

/// Go `Server.RunAccounts`: once a minute, provision personal settings files
/// for new active directory accounts (one commit for the whole batch).
async fn run_accounts(directory: Arc<Directory>, store: Arc<Store>) {
    let mut previous: Option<Vec<String>> = None;
    loop {
        let sync = async {
            let data = directory.load().await?;
            let mut ids: Vec<String> = data.users.iter().map(|u| u.subject.clone()).collect();
            ids.sort();
            if previous.as_ref() == Some(&ids) {
                return Ok::<_, anyhow::Error>(());
            }
            let users: Vec<_> = data.users.iter().map(|u| directory.identity(u)).collect();
            let initialized = store.ensure_accounts(&users).await?;
            tracing::info!(
                "Favorite accounts ready: {} active, {initialized} initialized",
                users.len()
            );
            previous = Some(ids);
            Ok(())
        };
        match tokio::time::timeout(Duration::from_secs(45), sync).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("Favorite account initialization: {e:#}"),
            Err(_) => tracing::warn!("Favorite account initialization: timeout"),
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

/// Go `:8080` listens on every interface, IPv6 and IPv4.
async fn bind(listen: &str) -> Result<tokio::net::TcpListener> {
    if let Some(port) = listen.strip_prefix(':') {
        if let Ok(listener) = tokio::net::TcpListener::bind(format!("[::]:{port}")).await {
            return Ok(listener);
        }
        return Ok(tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?);
    }
    Ok(tokio::net::TcpListener::bind(listen).await?)
}

async fn run() -> Result<()> {
    ensure!(
        std::env::args_os().len() == 1,
        "runtime configuration uses environment variables only"
    );
    let cfg = Config::from_env()?;
    let store = tokio::time::timeout(Duration::from_secs(60), Store::open(cfg.clone(), demo().1))
        .await
        .context("settings store startup timed out")??;
    let source = Source::open(&cfg).await?;
    let directory = Directory::new(cfg.clone())?;
    let auth = Auth::new(cfg.clone(), directory.clone())?;

    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(store.clone().run());
    tasks.spawn(source.clone().run());
    tasks.spawn(source.clone().watch_users(store.clone()));
    if !cfg.demo && !cfg.read_only {
        tasks.spawn(run_accounts(directory.clone(), store.clone()));
    }

    let listener = bind(&cfg.listen).await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tracing::info!(
        "KubeShelf {VERSION} listening on {} (demo={})",
        listener.local_addr()?,
        cfg.demo
    );
    let app = App {
        cfg,
        store,
        source,
        auth,
        directory,
    };
    let shutdown = async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = interrupt.recv() => {}
        }
        tracing::info!("shutting down");
    };
    http::serve(listener, http::router(app), shutdown).await;
    tasks.abort_all();
    Ok(())
}

#[tokio::main]
async fn main() {
    // Plain text when not attached to a terminal (container logs).
    tracing_subscriber::fmt()
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    if let Err(e) = run().await {
        // Error messages never contain secret values (only paths and reasons).
        tracing::error!("KubeShelf failed: {e:#}");
        std::process::exit(1);
    }
}
