use std::collections::HashMap;
use std::io::ErrorKind;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

struct AvfDaemonGatewayProxy {
    gateway_addr: String,
    backend_addr: String,
    handle: tokio::task::JoinHandle<()>,
}

static AVF_DAEMON_GATEWAY_PROXIES: OnceLock<StdMutex<HashMap<u16, AvfDaemonGatewayProxy>>> =
    OnceLock::new();

fn avf_daemon_gateway_proxies() -> &'static StdMutex<HashMap<u16, AvfDaemonGatewayProxy>> {
    AVF_DAEMON_GATEWAY_PROXIES.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn ensure_avf_guest_gateway_proxy(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    let mut replaced_existing_proxy = false;
    let existing_handle_to_abort = {
        let mut proxies = avf_daemon_gateway_proxies()
            .lock()
            .map_err(|_| anyhow!("AVF daemon gateway proxy mutex poisoned"))?;
        proxies.retain(|_, proxy| !proxy.handle.is_finished());
        if let Some(existing) = proxies.get(&port) {
            if existing.gateway_addr == gateway_addr && existing.backend_addr == backend_addr {
                return Ok(());
            }
        }
        let removed = proxies.remove(&port).map(|proxy| proxy.handle);
        if removed.is_some() {
            replaced_existing_proxy = true;
        }
        removed
    };

    if let Some(handle) = existing_handle_to_abort {
        handle.abort();
        tokio::task::yield_now().await;
    }

    let listener = loop {
        match tokio::net::TcpListener::bind(gateway_addr).await {
            Ok(listener) => break listener,
            Err(err)
                if replaced_existing_proxy
                    && matches!(
                        err.kind(),
                        ErrorKind::AddrInUse | ErrorKind::AddrNotAvailable
                    ) =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::AddrInUse | ErrorKind::AddrNotAvailable
                ) =>
            {
                tracing::debug!(
                    gateway_addr,
                    backend_addr,
                    "AVF daemon gateway proxy bind is unavailable; assuming a guest-reachable listener already exists"
                );
                return Ok(());
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("binding AVF guest gateway proxy at {gateway_addr} for {backend_addr}")
                });
            }
        }
    };

    let gateway_addr = gateway_addr.to_string();
    let backend_addr = backend_addr.to_string();
    let gateway_addr_for_task = gateway_addr.clone();
    let backend_addr_for_task = backend_addr.clone();
    let handle = tokio::spawn(async move {
        loop {
            let (mut inbound, peer_addr) = match listener.accept().await {
                Ok(parts) => parts,
                Err(err) => {
                    tracing::warn!(
                        gateway_addr = gateway_addr_for_task,
                        backend_addr = backend_addr_for_task,
                        "AVF daemon gateway proxy accept failed: {err}"
                    );
                    break;
                }
            };
            let backend_addr = backend_addr_for_task.clone();
            let gateway_addr = gateway_addr_for_task.clone();
            tokio::spawn(async move {
                match tokio::net::TcpStream::connect(&backend_addr).await {
                    Ok(mut outbound) => {
                        if let Err(err) =
                            tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await
                        {
                            tracing::debug!(
                                gateway_addr,
                                backend_addr,
                                %peer_addr,
                                "AVF daemon gateway proxy relay closed with error: {err}"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            gateway_addr,
                            backend_addr,
                            %peer_addr,
                            "AVF daemon gateway proxy could not connect to backend: {err}"
                        );
                    }
                }
            });
        }
    });
    let mut proxies = avf_daemon_gateway_proxies()
        .lock()
        .map_err(|_| anyhow!("AVF daemon gateway proxy mutex poisoned"))?;
    if let Some(existing) = proxies.get(&port) {
        if !existing.handle.is_finished() {
            handle.abort();
            return Ok(());
        }
    }
    proxies.insert(
        port,
        AvfDaemonGatewayProxy {
            gateway_addr: gateway_addr.clone(),
            backend_addr: backend_addr.clone(),
            handle,
        },
    );
    tracing::info!(
        gateway_addr,
        backend_addr,
        "started AVF daemon gateway proxy"
    );
    Ok(())
}

pub(crate) async fn ensure_avf_guest_gateway_proxy_for_test(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    ensure_avf_guest_gateway_proxy(gateway_addr, backend_addr, port).await
}
