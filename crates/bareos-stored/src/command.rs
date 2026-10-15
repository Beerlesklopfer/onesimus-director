//! Send a command to SD and read the response.
//!
//! Connects via TLS-PSK, authenticates, sends command, reads until EOD.

use bareos_bnet::{BnetMessage, BnetSignal};

/// Connect to SD, send a single command, read response, disconnect.
pub fn sd_command(
    address: &str,
    port: u16,
    password: &str,
    director_name: &str,
    command: &str,
) -> Result<String, String> {
    let mut sd = crate::connect::connect_sd(address, port, password, director_name)?;

    sd.send_str(command).map_err(|e| format!("send: {e}"))?;

    let mut response = String::new();
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                response.push_str(&String::from_utf8_lossy(&data));
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod | BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        }
    }
    let _ = sd.send_signal(BnetSignal::Terminate);
    Ok(response)
}

/// Control commands such as setdebug return one message rather than an EOD
/// terminated listing. Do not wait for another command's response or mask EOF.
pub fn sd_control_command(
    address: &str, port: u16, password: &str, director_name: &str, command: &str,
) -> Result<String, String> {
    let mut sd = crate::connect::connect_sd(address, port, password, director_name)?;
    sd.send_str(command).map_err(|error| format!("send: {error}"))?;
    let result = read_control_reply(&mut sd);
    let _ = sd.send_signal(BnetSignal::Terminate);
    result
}

fn read_control_reply<S: std::io::Read + std::io::Write>(sd: &mut bareos_bnet::BnetStream<S>) -> Result<String, String> {
    loop {
        match sd.recv().map_err(|error| format!("SD response: {error}"))? {
            BnetMessage::Data(bytes) => return String::from_utf8(bytes).map_err(|error| error.to_string()),
            BnetMessage::Signal(BnetSignal::Heartbeat) => {
                sd.send_signal(BnetSignal::HeartbeatResponse).map_err(|error| error.to_string())?;
            }
            BnetMessage::Signal(BnetSignal::Eod | BnetSignal::Terminate) => return Err("SD ended command without a response".into()),
            BnetMessage::Signal(_) => {},
        }
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
    struct Stream { input: Cursor<Vec<u8>>, output: Vec<u8> }
    impl Read for Stream { fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { self.input.read(b) } }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.output.extend_from_slice(b); Ok(b.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    #[test]
    fn single_response_needs_no_eod_and_reports_connection_loss() {
        let reply = b"3000 OK setdebug\r\n";
        let mut input = (reply.len() as i32).to_be_bytes().to_vec();
        input.extend_from_slice(reply);
        let mut sd = bareos_bnet::BnetStream::new(Stream { input: Cursor::new(input), output: vec![] });
        assert_eq!(read_control_reply(&mut sd).unwrap().as_bytes(), reply);
        let mut sd = bareos_bnet::BnetStream::new(Stream { input: Cursor::new(vec![]), output: vec![] });
        assert!(read_control_reply(&mut sd).is_err());
    }
}
