//! Zero-copy IPC via iceoryx2 shared memory.
//!
//! Replaces JSON-over-pipe for CatReq/CatResp between fork() worker and Director.
//! Latency: ~1-10μs vs ~5-40ms with pipes.
//!
//! Architecture:
//! ```text
//! Worker (fork child)              Director (parent)
//!   SD → CatReq                     CatReq handler (PgPool)
//!     → shm publish(request)          ← shm subscribe(request)
//!     ← shm subscribe(response)       → DB query
//!     → SD response                   → shm publish(response)
//! ```
//!
//! Services are per-job: "onesimus/job/{job_id}/req" and ".../resp"
//! Created by Director before fork(), used by both sides after fork().

use iceoryx2::prelude::*;
use tracing::{debug, error, warn};

// ---------------------------------------------------------------------------
// Fixed-size messages for shared memory (no heap, zero-copy)
// ---------------------------------------------------------------------------

/// CatReq message: Worker → Director
#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct ShmCatReq {
    pub job_id: u32,
    pub seq: u32,
    pub request_len: u32,
    pub request: [u8; 1024],
}

/// CatResp message: Director → Worker
#[derive(Debug, Clone, Copy, ZeroCopySend)]
#[repr(C)]
pub struct ShmCatResp {
    pub job_id: u32,
    pub seq: u32,
    pub response_len: u32,
    pub response: [u8; 2048],
}

impl ShmCatReq {
    pub fn new(job_id: u32, seq: u32, request: &str) -> Self {
        let mut msg = Self { job_id, seq, request_len: 0, request: [0; 1024] };
        let bytes = request.as_bytes();
        let len = bytes.len().min(1024);
        msg.request[..len].copy_from_slice(&bytes[..len]);
        msg.request_len = len as u32;
        msg
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.request[..self.request_len as usize]).unwrap_or("")
    }
}

impl ShmCatResp {
    pub fn new(job_id: u32, seq: u32, response: &str) -> Self {
        let mut msg = Self { job_id, seq, response_len: 0, response: [0; 2048] };
        let bytes = response.as_bytes();
        let len = bytes.len().min(2048);
        msg.response[..len].copy_from_slice(&bytes[..len]);
        msg.response_len = len as u32;
        msg
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.response[..self.response_len as usize]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Director-side: creates services, subscribes to requests, publishes responses
// ---------------------------------------------------------------------------

/// Director-side CatReq handler via shared memory.
pub struct ShmDirectorEnd {
    pub job_id: u32,
    req_subscriber: iceoryx2::port::subscriber::Subscriber<ipc::Service, ShmCatReq, ()>,
    resp_publisher: iceoryx2::port::publisher::Publisher<ipc::Service, ShmCatResp, ()>,
}

// SAFETY: ShmDirectorEnd is only used from the monitor_loop thread.
// iceoryx2 uses Rc internally but we guarantee single-thread access
// via the JobManager Mutex (only one thread calls poll_all at a time).
unsafe impl Send for ShmDirectorEnd {}

/// Worker-side CatReq sender via shared memory.
pub struct ShmWorkerEnd {
    pub job_id: u32,
    req_publisher: iceoryx2::port::publisher::Publisher<ipc::Service, ShmCatReq, ()>,
    resp_subscriber: iceoryx2::port::subscriber::Subscriber<ipc::Service, ShmCatResp, ()>,
    seq: u32,
}

// SAFETY: ShmWorkerEnd is only used from the sd-cat thread in the forked worker.
unsafe impl Send for ShmWorkerEnd {}

/// Create both ends of the SHM channel BEFORE fork().
/// Returns (director_end, worker_end) — each side uses its own end after fork.
pub fn create_shm_channel(job_id: u32) -> Result<(ShmDirectorEnd, ShmWorkerEnd), String> {
    let node = NodeBuilder::new()
        .create::<ipc::Service>()
        .map_err(|e| format!("iceoryx node: {e}"))?;

    // Request channel: Worker publishes, Director subscribes
    let req_service_name = ServiceName::new(&format!("onesimus/job/{job_id}/req"))
        .map_err(|e| format!("service name req: {e}"))?;
    let req_service = node.service_builder(&req_service_name)
        .publish_subscribe::<ShmCatReq>()
        .open_or_create()
        .map_err(|e| format!("req service: {e}"))?;

    let req_publisher = req_service.publisher_builder()
        .create()
        .map_err(|e| format!("req publisher: {e}"))?;
    let req_subscriber = req_service.subscriber_builder()
        .create()
        .map_err(|e| format!("req subscriber: {e}"))?;

    // Response channel: Director publishes, Worker subscribes
    let resp_service_name = ServiceName::new(&format!("onesimus/job/{job_id}/resp"))
        .map_err(|e| format!("service name resp: {e}"))?;
    let resp_service = node.service_builder(&resp_service_name)
        .publish_subscribe::<ShmCatResp>()
        .open_or_create()
        .map_err(|e| format!("resp service: {e}"))?;

    let resp_publisher = resp_service.publisher_builder()
        .create()
        .map_err(|e| format!("resp publisher: {e}"))?;
    let resp_subscriber = resp_service.subscriber_builder()
        .create()
        .map_err(|e| format!("resp subscriber: {e}"))?;

    debug!("SHM channel created for job {job_id}");

    Ok((
        ShmDirectorEnd { job_id, req_subscriber, resp_publisher },
        ShmWorkerEnd { job_id, req_publisher, resp_subscriber, seq: 0 },
    ))
}

impl ShmDirectorEnd {
    /// Try to receive a CatReq (non-blocking). Returns None if no request pending.
    pub fn try_recv_request(&self) -> Option<ShmCatReq> {
        match self.req_subscriber.receive() {
            Ok(Some(sample)) => Some(*sample),
            Ok(None) => None,
            Err(e) => { warn!("shm recv req: {e}"); None }
        }
    }

    /// Send a CatResp back to the worker.
    pub fn send_response(&self, resp: ShmCatResp) -> Result<(), String> {
        let sample = self.resp_publisher.loan_uninit()
            .map_err(|e| format!("shm loan resp: {e}"))?;
        let sample = sample.write_payload(resp);
        sample.send()
            .map_err(|e| format!("shm send resp: {e}"))?;
        Ok(())
    }
}

impl ShmWorkerEnd {
    /// Send a CatReq to the Director and wait for response (blocking).
    pub fn request(&mut self, request: &str) -> Result<String, String> {
        self.seq += 1;
        let req = ShmCatReq::new(self.job_id, self.seq, request);

        // Publish request
        let sample = self.req_publisher.loan_uninit()
            .map_err(|e| format!("shm loan req: {e}"))?;
        let sample = sample.write_payload(req);
        sample.send()
            .map_err(|e| format!("shm send req: {e}"))?;

        // Wait for matching response (spin with yield)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match self.resp_subscriber.receive() {
                Ok(Some(sample)) => {
                    if sample.seq == self.seq {
                        return Ok(sample.as_str().to_string());
                    }
                    // Wrong seq — stale response, skip
                }
                Ok(None) => {}
                Err(e) => return Err(format!("shm recv resp: {e}")),
            }
            if std::time::Instant::now() > deadline {
                return Err("shm CatReq timeout (30s)".into());
            }
            std::thread::yield_now();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shm_catrec_roundtrip() {
        let req = ShmCatReq::new(42, 1, "CatReq FindMedia pool_name=Full");
        assert_eq!(req.job_id, 42);
        assert_eq!(req.seq, 1);
        assert_eq!(req.as_str(), "CatReq FindMedia pool_name=Full");

        let resp = ShmCatResp::new(42, 1, "1000 OK VolName=Full-0001 VolJobs=3");
        assert_eq!(resp.as_str(), "1000 OK VolName=Full-0001 VolJobs=3");
    }

    #[test]
    fn shm_message_sizes() {
        assert!(std::mem::size_of::<ShmCatReq>() < 2048);
        assert!(std::mem::size_of::<ShmCatResp>() < 4096);
    }
}
