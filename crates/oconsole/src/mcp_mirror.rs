//! Unfiltered MCP traffic mirror. stdout remains reserved for the protocol.
use http_body_util::BodyExt;
use serde_json::json;
use std::{
    io::{self, Write},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[derive(Clone)]
pub struct Mirror {
    sink: Arc<Mutex<Box<dyn Write + Send>>>,
    sequence: Arc<AtomicU64>,
}

impl Mirror {
    pub fn stderr() -> Self {
        Self {
            sink: Arc::new(Mutex::new(Box::new(io::stderr()))),
            sequence: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn access(&self, transport: &'static str) -> Access {
        Access {
            mirror: self.clone(),
            transport,
            id: self.sequence.fetch_add(1, Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub struct Access {
    mirror: Mirror,
    transport: &'static str,
    id: u64,
}

impl Access {
    pub fn event(&self, direction: &str, data: &[u8]) {
        // JSON encoding makes terminal control characters visible instead of executing them.
        // No truncation or log-level filter: every received/sent byte is represented.
        let body = serde_json::from_slice::<serde_json::Value>(data).unwrap_or_else(|_| {
            match std::str::from_utf8(data) {
                Ok(text) => json!({"text": text}),
                Err(_) => json!({"hex": hex::encode(data)}),
            }
        });
        let record = json!({
            "mcp": self.transport, "access": self.id, "direction": direction,
            "time_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "body": body,
        });
        // One locked write per record keeps concurrent HTTP requests distinguishable.
        // A closed stderr must not panic or corrupt the MCP response.
        if let Ok(mut sink) = self.mirror.sink.lock() {
            let line = format!("MCP {record}\n");
            let _ = sink.write_all(line.as_bytes());
            let _ = sink.flush();
        }
    }

    pub fn body(&self, body: axum::body::Body, direction: &'static str) -> axum::body::Body {
        let access = self.clone();
        // Log each HTTP data frame immediately, including streaming SSE replies;
        // waiting for EOF would hide responses on a persistent event stream.
        let errors = self.clone();
        axum::body::Body::new(
            body.map_frame(move |frame| {
                if let Some(data) = frame.data_ref() {
                    access.event(direction, data);
                }
                frame
            })
            .map_err(move |error| {
                errors.event("transport-error", error.to_string().as_bytes());
                error
            }),
        )
    }
}

/// Mirror accepted stdio bytes without changing framing or backpressure.
pub struct MirrorIo<T> {
    inner: T,
    access: Access,
    direction: &'static str,
    pending: Vec<u8>,
}

impl<T> MirrorIo<T> {
    pub fn new(inner: T, access: Access, direction: &'static str) -> Self {
        Self {
            inner,
            access,
            direction,
            pending: Vec::new(),
        }
    }
    fn record(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            self.access.event(self.direction, &self.pending[..end + 1]);
            self.pending.drain(..end + 1);
        }
    }
    fn finish(&mut self) {
        if !self.pending.is_empty() {
            self.access.event(self.direction, &self.pending);
            self.pending.clear();
        }
    }
}
impl<T> Drop for MirrorIo<T> {
    fn drop(&mut self) {
        self.finish();
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for MirrorIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) {
            let data = &buf.filled()[before..];
            if data.is_empty() {
                this.finish();
            } else {
                this.record(data);
            }
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for MirrorIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(size)) = result {
            this.record(&buf[..size]);
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.finish();
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[derive(Clone, Default)]
    pub struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        pub fn mirror(&self) -> Mirror {
            Mirror {
                sink: Arc::new(Mutex::new(Box::new(self.clone()))),
                sequence: Arc::new(AtomicU64::new(1)),
            }
        }
        pub fn records(&self) -> Vec<serde_json::Value> {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line.strip_prefix("MCP ").unwrap()).unwrap())
                .collect()
        }
    }
    #[tokio::test]
    async fn fragmented_stdio_is_mirrored_without_changing_protocol_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let capture = Capture::default();
        let access = capture.mirror().access("stdio");
        let mut writer = MirrorIo::new(Vec::new(), access.clone(), "response");
        writer.write_all(b"{\"id\":").await.unwrap();
        writer.write_all(b"7,\"result\":{}}\n").await.unwrap();
        assert_eq!(writer.inner, b"{\"id\":7,\"result\":{}}\n");
        let input = b"{\"method\":\"ping\",\"id\":8}\ninvalid\x1b[2J";
        let mut reader = MirrorIo::new(&input[..], access, "request");
        let mut received = vec![];
        reader.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, input);
        let records = capture.records();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["body"]["id"], 7);
        assert_eq!(records[1]["body"]["method"], "ping");
        assert_eq!(records[2]["body"]["text"], "invalid\x1b[2J");
        assert!(!capture.0.lock().unwrap().contains(&0x1b));
    }
    #[tokio::test]
    async fn sse_frames_are_visible_before_eof_and_are_forwarded_verbatim() {
        let capture = Capture::default();
        let access = capture.mirror().access("http");
        let bytes = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let mut body = access.body(axum::body::Body::from(bytes), "response-body");
        let frame = body.frame().await.unwrap().unwrap();
        assert_eq!(frame.data_ref().unwrap().as_ref(), bytes.as_bytes());
        // No EOF poll or drop yet: persistent SSE streams must mirror immediately.
        let records = capture.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["body"]["text"], bytes);
    }
    #[test]
    fn parallel_accesses_have_distinct_ids_and_complete_unfiltered_data() {
        let capture = Capture::default();
        let mirror = capture.mirror();
        let data = "x".repeat(100_000);
        let first = mirror.access("http");
        let second = mirror.access("http");
        first.event("request", data.as_bytes());
        second.event("response", b"\xff");
        let records = capture.records();
        assert_ne!(records[0]["access"], records[1]["access"]);
        assert_eq!(records[0]["body"]["text"], data);
        assert_eq!(records[1]["body"]["hex"], "ff");
    }
}
