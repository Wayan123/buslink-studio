//! Accept loop: TLS, HTTP/1.1 with upgrades, a connection limit, and a shutdown
//! that drops every connection task.

use super::limits::{ConnLimits, ConnTracker};
use super::routes::{ClientAddr, ConnHandle, Ctx, router};
use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BUF: usize = 16 * 1024;

pub(crate) async fn run(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    ctx: Arc<Ctx>,
    conn_limits: ConnLimits,
    mut shutdown: watch::Receiver<bool>,
) {
    let app: Router = router(Arc::clone(&ctx));
    let tracker = ConnTracker::new(conn_limits);
    let mut conns: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            // Reap finished tasks so the set does not grow.
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
            accepted = listener.accept() => {
                let Ok((sock, peer)) = accepted else { continue };
                let conn = match tracker.admit(peer.ip()) {
                    Ok(c) => c,
                    Err(_) => {
                        ctx.stats.refused_connections.fetch_add(1, Ordering::Relaxed);
                        drop(sock);
                        continue;
                    }
                };
                let acceptor = acceptor.clone();
                let app = app
                    .clone()
                    .layer(axum::Extension(ClientAddr(peer)))
                    .layer(axum::Extension(ConnHandle(Arc::clone(&conn))));
                conns.spawn(async move {
                    let _conn = conn;
                    let _ = sock.set_nodelay(true);
                    let Ok(Ok(tls)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(sock)).await else {
                        return;
                    };
                    let service = TowerToHyperService::new(app);
                    let _ = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(HEADER_TIMEOUT)
                        .max_buf_size(MAX_BUF)
                        .serve_connection(TokioIo::new(tls), service)
                        .with_upgrades()
                        .await;
                });
            }
        }
    }
    // Dropping the set aborts every connection; the listener closes with this function.
    drop(conns);
    drop(listener);
}
