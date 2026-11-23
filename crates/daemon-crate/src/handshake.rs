// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Server-Handshake mit einem Director: Hello prüfen, CRAM-MD5, OK-Hello.
//!
//! Zweck: Ruft ein Director einen Daemon (FD oder SD) an, schickt er zuerst
//! `Hello Director <Name> calling`. Der Daemon sucht den Director in seiner
//! Konfiguration, beide prüfen sich gegenseitig mit CRAM-MD5, und der Daemon
//! bestätigt mit seiner OK-Hello-Zeile (`2000 OK Hello 54` beim FD,
//! `3000 OK Hello` beim SD). Misslingt CRAM-MD5, schickt er die
//! Ablehnungszeile und schließt.
//!
//! Datenfluss: [`crate::accept::open_bnet`] → erste Zeile →
//! [`authenticate_director`] → Name des Directors → Sitzung des Daemons.
//!
//! Bareos-Gegenstück: `core/src/filed/authenticate.cc`
//! (`AuthenticateDirector`), `stored/authenticate.cc`
//! (`AuthenticateDirector`), `lib/util.cc`
//! (`GetNameAndResourceTypeAndVersionFromHello`), `lib/cram_md5.cc`.
//!
//! Herkunft: `ofd/src/accept.rs` (`handle_session`), am 20.11.2026 ohne
//! Verhaltensänderung herausgelöst. Die Log-Meldungen schreibt weiter der
//! Daemon, weil sie seine Sitzungsnummer tragen.

use std::io::{Read, Write};

use bareos_bnet::{BnetStream, CramMd5Auth};

/// Why a Director was not let in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectorAuthError {
    /// The first line is not a valid `Hello Director <name> calling`.
    BadHello,
    /// No Director resource of that name.
    UnknownDirector(String),
    /// CRAM-MD5 failed; the refusal line was sent.
    AuthFailed(String),
    /// The OK-Hello line could not be sent.
    SendFailed(String),
}

/// The Director's name from its Hello (`Hello Director %s calling`,
/// 25 to 500 bytes long, blanks bashed as `0x01`), `None` for anything else.
pub fn director_name_from_hello(hello: &str) -> Option<String> {
    match hello.strip_prefix("Hello Director ").and_then(|r| r.split_whitespace().next()) {
        Some(name) if hello.len() >= 25 && hello.len() <= 500 => Some(bareos_format::bash::unbash_spaces(name)),
        _ => None,
    }
}

/// The lines a daemon answers a Director's Hello with.
#[derive(Debug, Clone, Copy)]
pub struct HelloReplies<'a> {
    /// Sent after a successful CRAM-MD5, e.g. `"2000 OK Hello 54\n"`.
    pub ok: &'a str,
    /// Sent when CRAM-MD5 fails, e.g. `"2999 Authentication failed.\n"`.
    pub sorry: &'a str,
}

/// Authenticates the Director that sent `hello` on `bnet` (server side of
/// CRAM-MD5, TLS policy 2 as Bareos with TLS-PSK) and answers with
/// `replies.ok`. `password_of(name)` gives the configured password of a
/// Director, `own_qualified_name` is e.g. `R_CLIENT::<fd-name>`. Returns
/// the Director's name.
pub fn authenticate_director<S, P>(
    bnet: &mut BnetStream<S>,
    hello: &str,
    own_qualified_name: &str,
    replies: HelloReplies<'_>,
    password_of: P,
) -> Result<String, DirectorAuthError>
where
    S: Read + Write,
    P: FnOnce(&str) -> Option<String>,
{
    let name = director_name_from_hello(hello).ok_or(DirectorAuthError::BadHello)?;
    let Some(password) = password_of(&name) else {
        return Err(DirectorAuthError::UnknownDirector(name));
    };
    let auth = CramMd5Auth::new(&password, own_qualified_name, 2);
    match auth.two_way_authenticate(bnet, true) {
        Ok(result) if result.success => {}
        _ => {
            let _ = bnet.send_str(replies.sorry);
            return Err(DirectorAuthError::AuthFailed(name));
        }
    }
    bnet.send_str(replies.ok).map_err(|_| DirectorAuthError::SendFailed(name.clone()))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    #[test]
    fn hello_names_like_ofd() {
        assert_eq!(director_name_from_hello("Hello Director PDC-dir calling\n").as_deref(), Some("PDC-dir"));
        assert_eq!(director_name_from_hello("Hello Director My\u{1}Dir calling\n").as_deref(), Some("My Dir"));
        assert_eq!(director_name_from_hello("Hello Director x\n"), None); // shorter than 25
        assert_eq!(director_name_from_hello("Hello Storage calling Start Job x\n"), None);
    }

    /// One handshake against a Director client over loopback.
    fn run(server_password: Option<&'static str>, client_password: &'static str) -> (Result<String, DirectorAuthError>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::thread::spawn(move || {
            let mut bnet = BnetStream::new(TcpStream::connect(addr).unwrap());
            bnet.send_str("Hello Director test-dir calling\n").unwrap();
            let auth = CramMd5Auth::new(client_password, "test-dir", 2);
            let _ = auth.two_way_authenticate(&mut bnet, false);
            bnet.recv_str().unwrap_or_default()
        });
        let (s, _) = listener.accept().unwrap();
        let mut bnet = BnetStream::new(s);
        let hello = bnet.recv_str().unwrap();
        let replies = HelloReplies { ok: "2000 OK Hello 54\n", sorry: "2999 Authentication failed.\n" };
        let r = authenticate_director(&mut bnet, &hello, "R_CLIENT::test-fd", replies, |name| {
            assert_eq!(name, "test-dir");
            server_password.map(str::to_string)
        });
        if r.is_err() {
            drop(bnet);
        }
        (r, client.join().unwrap())
    }

    #[test]
    fn right_password_gets_ok_hello() {
        let (r, answer) = run(Some("secret"), "secret");
        assert_eq!(r, Ok("test-dir".to_string()));
        assert_eq!(answer, "2000 OK Hello 54\n");
    }

    #[test]
    fn wrong_password_is_refused() {
        let (r, _) = run(Some("secret"), "wrong");
        assert_eq!(r, Err(DirectorAuthError::AuthFailed("test-dir".into())));
    }
}
