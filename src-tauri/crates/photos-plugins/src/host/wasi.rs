//! Lumen's WASI p2 (spec §2): no preopens, no env, no args, TCP, UDP and name lookup off and a
//! socket-address check that refuses everything; clocks and random work. stdout and stderr
//! become log lines.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamResult};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder};

use super::ctx::LogSink;

/// A guest's longest line; a longer run without a newline is logged in pieces.
const MAX_LINE: usize = 4096;

pub(crate) fn ctx(plugin: &str, log: &LogSink) -> WasiCtx {
    WasiCtxBuilder::new()
        .stdout(LogStream::new(plugin, log.clone()))
        .stderr(LogStream::new(plugin, log.clone()))
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false)
        .socket_addr_check(|_, _| Box::pin(async { false }))
        .build()
}

/// Splits a guest's output into lines and hands each to the plugin's log sink.
#[derive(Clone)]
struct LogStream {
    prefix: Arc<str>,
    sink: LogSink,
    pending: Arc<Mutex<Vec<u8>>>,
}

impl LogStream {
    fn new(plugin: &str, sink: LogSink) -> LogStream {
        LogStream { prefix: format!("plugin={plugin} ").into(), sink, pending: Arc::default() }
    }

    fn emit(&self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        let text = text.trim_end_matches('\r');
        if !text.is_empty() {
            (self.sink)(&format!("{}{text}", self.prefix));
        }
    }

    fn take(&self, bytes: &[u8]) {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.extend_from_slice(bytes);
        while let Some(i) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=i).collect();
            self.emit(&line[..line.len() - 1]);
        }
        if pending.len() > MAX_LINE {
            let line: Vec<u8> = std::mem::take(&mut *pending);
            self.emit(&line);
        }
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        // The last clone flushes an unfinished line.
        if Arc::strong_count(&self.pending) == 1 {
            let rest = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|p| p.into_inner()));
            self.emit(&rest);
        }
    }
}

impl IsTerminal for LogStream {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogStream {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for LogStream {
    async fn ready(&mut self) {}
}

impl OutputStream for LogStream {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.take(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(64 * 1024)
    }
}

impl tokio::io::AsyncWrite for LogStream {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        self.take(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_and_flush_on_drop() {
        let got: Arc<Mutex<Vec<String>>> = Arc::default();
        let g = got.clone();
        let s = LogStream::new("p", Arc::new(move |l: &str| g.lock().unwrap().push(l.into())));
        s.take(b"one\r\ntw");
        s.take(b"o\nthree");
        drop(s);
        assert_eq!(*got.lock().unwrap(), vec!["plugin=p one", "plugin=p two", "plugin=p three"]);
    }
}
