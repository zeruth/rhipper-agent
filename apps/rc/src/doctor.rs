//! `rc doctor`: the checks a run needs as a table, then the local route tried for real. The
//! table's `local server` line says a process lists a model at the URL; the `local route` line
//! under it says whether that model answers a one-token request, which one and how fast, or
//! why it refused, so a person sees the difference between a server that is up and one that
//! serves the roles.

use std::path::Path;
use std::time::Duration;

use ra_console::agent::Check;
use ra_providers::local::{self, Probe};

/// The name of the probe's line, padded as the table pads its checks.
const LINE_NAME: &str = "local route";

/// Prints the table and the probe's line; a required check failing is the error, so the exit
/// code says it. The probe blocks nothing: a missing server degrades the roles to no call.
pub(crate) async fn run(root: &Path) -> anyhow::Result<()> {
    let checks = ra_console::agent::doctor(root).await;
    for line in Check::table(&checks) {
        println!("{line}");
    }
    println!("{}", local_line(&local::probe().await));
    // the machine's manifest probed afresh, the one every project's runs read today
    let dir = root.to_path_buf();
    let manifest = tokio::task::spawn_blocking(move || ra_console::hub::manifest::refresh(&dir)).await?;
    println!();
    println!("machine manifest of {} ({}):", manifest.day, ra_console::hub::dir().join(ra_console::hub::manifest::MANIFEST_FILE).display());
    for line in manifest.lines() {
        println!("   {line}");
    }
    let blocking = Check::blocking(&checks);
    anyhow::ensure!(blocking.is_empty(), "a run needs: {}", blocking.join(", "));
    Ok(())
}

/// The probe's line in the table's shape: `answering: <model> in <latency>` under a tick, or
/// `refused: <reason>` under a dash, as a failed optional check gets.
pub(crate) fn local_line(outcome: &Result<Probe, String>) -> String {
    match outcome {
        Ok(probe) => format!("✓  {LINE_NAME:<15} answering: {} in {}", probe.model, latency(probe.latency)),
        Err(reason) => format!("–  {LINE_NAME:<15} refused: {reason}"),
    }
}

/// A latency as a person reads it: milliseconds under a second, else seconds to a tenth.
fn latency(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) { format!("{}ms", elapsed.as_millis()) } else { format!("{:.1}s", elapsed.as_secs_f64()) }
}
