//! PAM login of a console (`Use Pam Authentication = yes`), the dialog of
//! `dird/auth_pam.cc` and `console/auth_pam.cc` (docs/STUBS.md S-43).
//!
//! The password of the Console resource is checked first (CRAM-MD5, as for
//! every console); then the Director asks for a PAM login on the same
//! connection. The messages below were observed with the installed original
//! `bconsole` 25.1.3 against a stand-in Director
//! (`crates/onesimus-rig/tests/pam_capture.rs`):
//!
//! ```text
//! Director                          Console
//!   "1001 PAM required"     ────►
//!                           ◄────   "4002" RS user RS password   (credentials file, `bconsole -p`)
//!                              or   "4001" RS                    (asks the user)
//!   interactive only, per PAM prompt:
//!   one frame  [type]       ────►   1 = answer is not echoed, 2 = echoed
//!   one frame  prompt text  ────►
//!                           ◄────   one frame with the answer
//!   interactive only, at the end:
//!   frame [0x00], frame [result] ─► (the console ignores the second byte)
//!   "1000 OK: …"            ────►   the usual banner follows (success)
//!   "1999 …" or close       ────►   the console ends without a message (failure)
//! ```
//!
//! RS is the byte 0x1e. The Director refuses a console older than 18.2
//! ("PAM authentication using outdated Bareos console denied").
//!
//! What PAM decides is separated from the dialog: [`Backend`] is the system
//! PAM ([`SystemPam`], service `bareos` like the original) in production and
//! a stand-in in tests, because a PAM service cannot be set up without root.

use std::io::{Read, Write};
use std::sync::Arc;

use bareos_bnet::wire::{BnetMessage, BnetStream};
use bareos_config::raw::ConfResource;

/// Field separator inside the console's answer.
const RS: char = '\x1e';
/// The console wants to be asked (interactive).
const CHOICE_INTERACTIVE: &str = "4001";
/// The console sends user and password at once.
const CHOICE_CREDENTIALS: &str = "4002";
/// The PAM service name of the original (`pam_start("bareos", …)`).
pub const PAM_SERVICE: &str = "bareos";

/// What a PAM module asks the user.
pub trait Conversation {
    /// A question; `echo` tells whether the answer may be shown (user name)
    /// or not (password). `Err` ends the login.
    fn prompt(&mut self, echo: bool, text: &str) -> Result<String, String>;
}

/// The PAM implementation behind the dialog.
pub trait Backend: Send + Sync {
    /// Runs `pam_authenticate` and `pam_acct_mgmt` for `user` (None: PAM asks
    /// for the name) and returns the name PAM settled on.
    fn authenticate(&self, user: Option<&str>, conversation: &mut dyn Conversation) -> Result<String, String>;
}

/// Answers every question with the credentials the console sent.
struct Credentials<'a> {
    /// User name.
    user: &'a str,
    /// Password.
    password: &'a str,
}

impl Conversation for Credentials<'_> {
    fn prompt(&mut self, echo: bool, _text: &str) -> Result<String, String> {
        Ok(if echo { self.user } else { self.password }.to_string())
    }
}

/// Forwards every question to the console and reads its answer.
struct Interactive<'a, S: Read + Write> {
    /// The console connection.
    bnet: &'a mut BnetStream<S>,
}

impl<S: Read + Write> Conversation for Interactive<'_, S> {
    fn prompt(&mut self, echo: bool, text: &str) -> Result<String, String> {
        let io = |e: bareos_bnet::wire::BnetError| e.to_string();
        // two frames: the type byte (1 not echoed, 2 echoed), then the text
        self.bnet.send(&[if echo { 2 } else { 1 }]).map_err(io)?;
        self.bnet.send(text.as_bytes()).map_err(io)?;
        match self.bnet.recv().map_err(io)? {
            BnetMessage::Data(data) => Ok(String::from_utf8_lossy(&data).trim_end_matches('\0').to_string()),
            BnetMessage::Signal(signal) => Err(format!("the console answered with a signal ({signal:?})")),
        }
    }
}

/// The PAM dialog on `bnet`, after the console passed CRAM-MD5. Returns the
/// PAM user name; on failure the reason (the caller ends the connection).
/// The connection stays open for the banner on success.
pub fn run_dialog<S: Read + Write>(bnet: &mut BnetStream<S>, backend: &dyn Backend) -> Result<String, String> {
    let io = |e: bareos_bnet::wire::BnetError| e.to_string();
    bnet.send_str("1001 PAM required").map_err(io)?;
    let choice = bnet.recv_str().map_err(io)?;
    let choice = choice.trim_end_matches(['\0', '\n']);
    let mut fields = choice.split(RS);
    let result = match fields.next() {
        Some(CHOICE_CREDENTIALS) => {
            let rest: Vec<&str> = fields.collect();
            let [user, password] = rest.as_slice() else {
                return Err("console sent wrong number of credentials".into());
            };
            backend.authenticate(Some(user), &mut Credentials { user, password })
        }
        Some(CHOICE_INTERACTIVE) => {
            let outcome = backend.authenticate(None, &mut Interactive { bnet: &mut *bnet });
            // the end of the dialog: a frame with the byte 0, one with the result
            bnet.send(&[0]).map_err(io)?;
            bnet.send(&[u8::from(outcome.is_ok())]).map_err(io)?;
            outcome
        }
        _ => return Err(format!("unknown answer of the console to the PAM request: {choice:?}")),
    };
    result
}

/// The `User` resource of a PAM user: it carries the ACLs of the session
/// ("No user config found for user %s" in the original).
pub fn user_resource<'a>(raw: &'a [ConfResource], user: &str) -> Option<&'a ConfResource> {
    raw.iter().find(|r| r.norm_type() == "user" && r.name() == Some(user))
}

/// True when the Console resource `name` has `Use Pam Authentication = yes`.
pub fn console_uses_pam(raw: &[ConfResource], name: &str) -> bool {
    raw.iter()
        .filter(|r| r.norm_type() == "console" && r.name().is_some_and(|n| n == name))
        .any(|r| {
            r.directives.iter().rev().find(|d| d.norm_key() == "usepamauthentication")
                .and_then(|d| d.first())
                .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"))
        })
}

/// The system PAM through the crate `pam-client2`.
pub struct SystemPam;

/// Bridges the PAM conversation callbacks to a [`Conversation`].
struct Bridge<'a> {
    /// Where the questions go.
    conversation: &'a mut dyn Conversation,
}

impl pam_client2::ConversationHandler for Bridge<'_> {
    fn prompt_echo_on(&mut self, prompt: &std::ffi::CStr) -> Result<std::ffi::CString, pam_client2::ErrorCode> {
        let answer = self.conversation.prompt(true, &prompt.to_string_lossy()).map_err(|_| pam_client2::ErrorCode::CONV_ERR)?;
        std::ffi::CString::new(answer).map_err(|_| pam_client2::ErrorCode::CONV_ERR)
    }

    fn prompt_echo_off(&mut self, prompt: &std::ffi::CStr) -> Result<std::ffi::CString, pam_client2::ErrorCode> {
        let answer = self.conversation.prompt(false, &prompt.to_string_lossy()).map_err(|_| pam_client2::ErrorCode::CONV_ERR)?;
        std::ffi::CString::new(answer).map_err(|_| pam_client2::ErrorCode::CONV_ERR)
    }

    // The console understands only questions (types 1 and 2); module
    // messages are dropped.
    fn text_info(&mut self, _msg: &std::ffi::CStr) {}

    fn error_msg(&mut self, _msg: &std::ffi::CStr) {}
}

impl Backend for SystemPam {
    fn authenticate(&self, user: Option<&str>, conversation: &mut dyn Conversation) -> Result<String, String> {
        let mut context = pam_client2::Context::new(PAM_SERVICE, user, Bridge { conversation })
            .map_err(|e| format!("pam_start: {e}"))?;
        context.authenticate(pam_client2::Flag::DISALLOW_NULL_AUTHTOK).map_err(|e| format!("pam_authenticate: {e}"))?;
        context.acct_mgmt(pam_client2::Flag::NONE).map_err(|e| format!("pam_acct_mgmt: {e}"))?;
        context.user().map_err(|e| format!("pam_get_item(PAM_USER): {e}"))
    }
}

/// The backend in use: the system PAM, in tests the one set by [`set_test_backend`].
pub fn backend() -> Arc<dyn Backend> {
    #[cfg(test)]
    if let Some(backend) = TEST_BACKEND.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return backend;
    }
    Arc::new(SystemPam)
}

/// Backend of the tests (`None`: the system PAM).
#[cfg(test)]
static TEST_BACKEND: std::sync::Mutex<Option<Arc<dyn Backend>>> = std::sync::Mutex::new(None);

/// Replaces the PAM of the tests.
#[cfg(test)]
pub fn set_test_backend(backend: Option<Arc<dyn Backend>>) {
    *TEST_BACKEND.lock().unwrap_or_else(|e| e.into_inner()) = backend;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    /// Accepts `testuser` / `testpass`; asks for the name only when it has none.
    struct Pam;

    impl Backend for Pam {
        fn authenticate(&self, user: Option<&str>, conversation: &mut dyn Conversation) -> Result<String, String> {
            let user = match user {
                Some(user) => user.to_string(),
                None => conversation.prompt(true, "login: ")?,
            };
            let password = conversation.prompt(false, "Password: ")?;
            if user == "testuser" && password == "testpass" { Ok(user) } else { Err("authentication failure".into()) }
        }
    }

    /// Runs `run_dialog` on one end of a loopback connection and `client` on the other.
    fn dialog(client: impl FnOnce(&mut BnetStream<TcpStream>) + Send + 'static) -> Result<String, String> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let console = std::thread::spawn(move || client(&mut BnetStream::new(TcpStream::connect(address).unwrap())));
        let (stream, _) = listener.accept().unwrap();
        let result = run_dialog(&mut BnetStream::new(stream), &Pam);
        console.join().unwrap();
        result
    }

    /// The frames the original console expects and sends (pam_capture.rs).
    #[test]
    fn credentials_in_one_answer() {
        let ok = dialog(|bnet| {
            assert!(bnet.recv_str().unwrap().starts_with("1001 "));
            bnet.send_str("4002\x1etestuser\x1etestpass").unwrap();
        });
        assert_eq!(ok, Ok("testuser".into()));
        let wrong = dialog(|bnet| {
            bnet.recv_str().unwrap();
            bnet.send_str("4002\x1etestuser\x1enope").unwrap();
        });
        assert!(wrong.is_err());
        let count = dialog(|bnet| {
            bnet.recv_str().unwrap();
            bnet.send_str("4002\x1etestuser").unwrap();
        });
        assert!(count.unwrap_err().contains("wrong number of credentials"));
    }

    /// Two frames per question (type byte, text), the answer in one frame, and
    /// at the end a frame with 0 and one with the result.
    #[test]
    fn interactive_questions_and_end() {
        let answer = |password: &'static str| dialog(move |bnet| {
            bnet.recv_str().unwrap();
            bnet.send_str("4001\x1e").unwrap();
            for (kind, text, reply) in [(2u8, "login: ", "testuser"), (1u8, "Password: ", password)] {
                assert_eq!(bnet.recv().unwrap(), BnetMessage::Data(vec![kind]));
                assert_eq!(bnet.recv().unwrap(), BnetMessage::Data(text.as_bytes().to_vec()));
                bnet.send_str(reply).unwrap();
            }
            assert_eq!(bnet.recv().unwrap(), BnetMessage::Data(vec![0]));
            assert_eq!(bnet.recv().unwrap(), BnetMessage::Data(vec![u8::from(password == "testpass")]));
        });
        assert_eq!(answer("testpass"), Ok("testuser".into()));
        assert!(answer("nope").is_err());
    }

    /// Anything but 4001 and 4002 is refused.
    #[test]
    fn an_unknown_answer_is_refused() {
        let result = dialog(|bnet| {
            bnet.recv_str().unwrap();
            bnet.send_str("hello").unwrap();
        });
        assert!(result.unwrap_err().contains("unknown answer"));
    }

    /// Consoles with PAM and the Users that carry the ACLs.
    #[test]
    fn pam_consoles_and_users_are_found_by_name() {
        let raw = bareos_config::raw::parse_str(
            "Console {\n Name = pam\n Password = x\n Use Pam Authentication = yes\n}\n\
             Console {\n Name = plain\n Password = x\n}\n\
             User {\n Name = anna\n Command ACL = *all*\n}\n", None).unwrap();
        assert!(console_uses_pam(&raw, "pam") && !console_uses_pam(&raw, "plain") && !console_uses_pam(&raw, "none"));
        assert!(user_resource(&raw, "anna").is_some() && user_resource(&raw, "pam").is_none());
    }

    /// The system PAM is called for real: without a PAM service `bareos` the
    /// fallback `other` denies, and a wrong login must end in an error, never
    /// in a user name. (A login that succeeds needs a PAM service and
    /// root to set it up: not tested here.)
    #[test]
    #[ignore = "needs libpam and a host whose PAM service `other` denies: run with --ignored"]
    fn the_system_pam_refuses_an_unknown_login() {
        let mut credentials = Credentials { user: "no-such-user-for-odir-test", password: "x" };
        let result = SystemPam.authenticate(Some("no-such-user-for-odir-test"), &mut credentials);
        assert!(result.is_err(), "{result:?}");
    }
}
