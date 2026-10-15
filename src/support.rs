use anyhow::Result;

pub(crate) fn dependency_status(binary: &str) -> &'static str {
    match std::process::Command::new(binary).arg("--help").output() {
        Ok(_) => "present",
        Err(_) => "missing",
    }
}

pub(crate) fn print_payload<T>(json: bool, payload: &T) -> Result<()>
where
    T: serde::Serialize,
{
    if json {
        println!("{}", serde_json::to_string_pretty(payload)?);
    } else {
        println!("{}", serde_json::to_string(payload)?);
    }
    Ok(())
}

/// Drive one of the semantic module's futures to completion.
///
/// With the `semantic` feature a tokio current-thread runtime runs it, as before.
/// A slim build carries no tokio: there only the deterministic test embedder can
/// get this far, and those futures await nothing that can suspend, so one poll
/// finishes them. A future that does suspend is reported, never spun on.
#[cfg(feature = "semantic")]
pub(crate) fn block_on<F: std::future::Future>(future: F) -> Result<F::Output> {
    use anyhow::Context;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create semantic runtime")?;
    Ok(runtime.block_on(future))
}

#[cfg(not(feature = "semantic"))]
pub(crate) fn block_on<F: std::future::Future>(future: F) -> Result<F::Output> {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => Ok(output),
        Poll::Pending => anyhow::bail!(
            "semantic work suspended, which a build without the `semantic` feature cannot drive"
        ),
    }
}
