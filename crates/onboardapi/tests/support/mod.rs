//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::{fmt::Debug, time::Duration};

use onboardapi::{ClientSide, Endpoint, Inbox, Message, Role};
use tokio::time::timeout;

pub const WAIT: Duration = Duration::from_secs(15);

pub fn domain() -> i32 {
    std::env::var("ONBOARDAPI_TEST_DOMAIN")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(42)
}

/// onboardapi service names should only use `[a-zA-Z0-9_]`.
pub fn unique(name: &str) -> String {
    format!("rs_test_{name}_{}", std::process::id())
}

pub async fn connect<R: ClientSide>(client: &Endpoint<R>) -> anyhow::Result<()> {
    timeout(WAIT, client.connected()).await??;
    Ok(())
}

pub async fn next<M: Message>(inbox: &mut Inbox<M>) -> M {
    timeout(WAIT, inbox.recv())
        .await
        .expect("timed out waiting for a message")
        .expect("inbox closed")
}

/// Sends `expected` from `sender` and waits until all of them arrived at `inbox`, in any order.
pub async fn exchange<R, M>(
    sender: &Endpoint<R>,
    inbox: &mut Inbox<M>,
    expected: Vec<M>,
) -> anyhow::Result<()>
where
    R: Role<Outgoing = M>,
    M: Message + Debug + PartialEq,
{
    for msg in &expected {
        sender.send(msg)?;
    }
    let mut missing = expected;
    while !missing.is_empty() {
        let got = match timeout(WAIT, inbox.recv()).await {
            Ok(Some(m)) => m,
            _ => anyhow::bail!(
                "{}: {} operations never arrived: {:?}",
                M::INTERFACE,
                missing.len(),
                missing.iter().map(Message::op_name).collect::<Vec<_>>()
            ),
        };
        match missing.iter().position(|m| *m == got) {
            Some(i) => {
                missing.swap_remove(i);
            }
            // Late-join data (e.g. removal reports) may show up as well; ignore it.
            None => tracing::debug!(op = got.op_name(), "unexpected extra message"),
        }
    }
    Ok(())
}
