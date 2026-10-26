//! CatReq relay loop: SD ↔ Director catalog request proxy.
//!
//! The SD sends CatReq messages to the Director (via the worker pipe)
//! and the Director responds with catalog data. This thread runs
//! alongside the main FD communication thread.

use std::io::{BufRead, BufReader, Write};

use bareos_bnet::{BnetStream, BnetMessage, BnetSignal};
use bareos_bnet::tls_psk::PskStream;
use tracing::{debug, warn, error};

use serde::{Serialize, Deserialize};

/// Pipe message for CatReq relay (serialized as JSON over the worker pipe).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CatReqMessage {
    #[serde(rename = "cat_req")]
    Request { job_id: u32, request: String },
    #[serde(rename = "cat_resp")]
    Response { response: String },
}

/// Run the SD CatReq relay loop.
///
/// Reads messages from SD, forwards CatReqs to the Director via pipe,
/// reads responses from pipe, sends them back to SD.
///
/// Terminates on: SD disconnect, "3099 Job end", or pipe EOF.
pub fn sd_catrec_loop(
    sd: &mut BnetStream<PskStream>,
    job_id: u32,
    to_dir: &mut dyn Write,
    from_dir: &mut dyn std::io::Read,
) {
    let mut from_dir = BufReader::new(from_dir);

    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).trim().to_string();

                if msg.starts_with("CatReq") {
                    debug!("SD CatReq [{job_id}]: {msg}");

                    // Forward to Director
                    let pipe_msg = CatReqMessage::Request { job_id, request: msg };
                    if let Ok(json) = serde_json::to_string(&pipe_msg) {
                        let _ = writeln!(to_dir, "{json}");
                    }

                    // Read CatResp from Director
                    let mut buf = String::new();
                    match from_dir.read_line(&mut buf) {
                        Ok(n) if n > 0 => {
                            if let Ok(CatReqMessage::Response { response }) =
                                serde_json::from_str::<CatReqMessage>(&buf)
                            {
                                debug!("SD CatResp [{job_id}]: {}", &response[..response.len().min(60)]);
                                let _ = sd.send_str(&response);
                            } else {
                                warn!("unexpected pipe msg [{job_id}]: {}", buf.trim());
                                let _ = sd.send_str("1000 OK\n");
                            }
                        }
                        Ok(_) => {
                            warn!("pipe EOF [{job_id}] while waiting for CatResp");
                            let _ = sd.send_str("1000 OK\n");
                        }
                        Err(e) => {
                            error!("pipe read error [{job_id}]: {e}");
                            let _ = sd.send_str("1000 OK\n");
                        }
                    }
                } else if msg.starts_with("3099 Job") {
                    debug!("SD job end [{job_id}]");
                    break;
                } else if msg.starts_with("3010 Job") {
                    debug!("SD job start [{job_id}]");
                } else if msg.starts_with("Jmsg") {
                    debug!("SD Jmsg [{job_id}]: {msg}");
                } else {
                    debug!("SD [{job_id}]: {}", &msg[..msg.len().min(80)]);
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(sig)) => debug!("SD signal [{job_id}]: {sig:?}"),
            Err(e) => { debug!("SD recv [{job_id}]: {e}"); break; }
        }
    }

    let _ = sd.send_signal(BnetSignal::Terminate);
    debug!("SD CatReq loop ended [{job_id}]");
}
