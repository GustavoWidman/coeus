use crate::{common::proto::LogSource, jobs::JobContext};
use eyre::Result;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Bound allocations even for arbitrarily long lines or a child that never emits
/// a newline. Oversized lines become multiple chunks instead of unbounded buffers.
pub(super) async fn drain(
    mut reader: impl AsyncRead + Unpin,
    source: LogSource,
    context: JobContext,
) -> Result<()> {
    let mut input = [0; 1024];
    let mut line = Vec::with_capacity(1024);
    loop {
        let count = reader.read(&mut input).await?;
        if count == 0 {
            break;
        }
        for &byte in &input[..count] {
            if byte == b'\n' {
                context.log(source, &String::from_utf8_lossy(&line)).await;
                line.clear();
            } else {
                line.push(byte);
                if line.len() == 1024 {
                    context.log(source, &String::from_utf8_lossy(&line)).await;
                    line.clear();
                }
            }
        }
    }
    if !line.is_empty() {
        context.log(source, &String::from_utf8_lossy(&line)).await;
    }
    Ok(())
}
