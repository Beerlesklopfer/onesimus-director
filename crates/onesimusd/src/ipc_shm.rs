//! Zero-copy IPC via iceoryx2 shared memory.
//!
//! Replaces JSON-over-pipe for CatReq/CatResp between fork() worker and Director.
//! Latency: ~1-10μs vs ~5-40ms with pipes.
//!
//! Architecture:
//! ```text
//! Worker (fork child)              Director (parent)
//!   SD → CatReq                     catalog_worker (PgPool)
//!     → shm publish(request)          ← shm subscribe(request)
//!     ← shm subscribe(response)       → DB query
//!     → SD response                   → shm publish(response)
//! ```

use iceoryx2::prelude::*;

/// Fixed-size CatReq message for shared memory (no heap allocation).
/// Max 1024 bytes for request/response strings.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ShmCatReq {
    pub job_id: u32,
    pub request_len: u32,
    pub request: [u8; 1024],
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ShmCatResp {
    pub job_id: u32,
    pub response_len: u32,
    pub response: [u8; 2048],  // OK_media is ~500 bytes, 2KB is safe
}

impl ShmCatReq {
    pub fn new(job_id: u32, request: &str) -> Self {
        let mut msg = Self { job_id, request_len: 0, request: [0; 1024] };
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
    pub fn new(job_id: u32, response: &str) -> Self {
        let mut msg = Self { job_id, response_len: 0, response: [0; 2048] };
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

/// Create a unique service name for a job's CatReq channel.
pub fn service_name_req(job_id: u32) -> String {
    format!("onesimus/catrec/{job_id}/req")
}

pub fn service_name_resp(job_id: u32) -> String {
    format!("onesimus/catrec/{job_id}/resp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shm_catrec_roundtrip() {
        let req = ShmCatReq::new(42, "CatReq FindMedia pool_name=Full");
        assert_eq!(req.job_id, 42);
        assert_eq!(req.as_str(), "CatReq FindMedia pool_name=Full");

        let resp = ShmCatResp::new(42, "1000 OK VolName=Full-0001 VolJobs=3");
        assert_eq!(resp.as_str(), "1000 OK VolName=Full-0001 VolJobs=3");
    }

    #[test]
    fn shm_message_sizes() {
        // Verify sizes are reasonable for shared memory
        assert_eq!(std::mem::size_of::<ShmCatReq>(), 1032);   // 4+4+1024
        assert_eq!(std::mem::size_of::<ShmCatResp>(), 2056);  // 4+4+2048
    }
}
