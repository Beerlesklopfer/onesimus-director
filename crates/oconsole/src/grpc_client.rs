//! gRPC client for onesimus-dird: TLS with certificate pinning + console login.
//!
//! The Director uses a self-signed certificate. Instead of CA validation the
//! client pins the SHA-256 fingerprint of the certificate (trust on first use,
//! stored in ~/.config/oconsole/known_directors). Handshake signatures are
//! still verified, so a pinned connection cannot be intercepted.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use hyper_util::rt::TokioIo;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};

use bareos_proto_rs::bareos::director::{
    director_client::DirectorClient, ChallengeRequest, CommandRequest, LoginRequest,
    ResponseFormat,
};

pub const SESSION_HEADER: &str = "x-onesimus-session";

pub struct GrpcSession {
    client: DirectorClient<Channel>,
    channel: Channel,
    token: MetadataValue<tonic::metadata::Ascii>,
    pub director_name: String,
    pub version: String,
}

pub struct CommandOutput {
    pub success: bool,
    pub text: String,
    pub error: String,
}

/// Fetch the server certificate fingerprint without trusting it (for TOFU prompts).
pub async fn probe_fingerprint(host: &str, port: u16) -> anyhow::Result<String> {
    let seen = Arc::new(Mutex::new(None));
    let verifier = PinVerifier { pinned: None, seen: seen.clone() };
    // The handshake is aborted by the verifier when nothing is pinned; we only want the cert.
    let _ = tls_connect(host, port, verifier).await;
    let fp = seen.lock().unwrap().take();
    fp.ok_or_else(|| anyhow::anyhow!("no TLS certificate received from {host}:{port}"))
}

impl GrpcSession {
    /// Connect with a pinned certificate fingerprint and log in.
    pub async fn connect(
        host: &str,
        port: u16,
        fingerprint: &str,
        console_name: &str,
        password: &str,
    ) -> anyhow::Result<Self> {
        let host_owned = host.to_string();
        let pinned = fingerprint.to_ascii_lowercase();
        let connector = tower::service_fn(move |_: Uri| {
            let host = host_owned.clone();
            let verifier = PinVerifier { pinned: Some(pinned.clone()), seen: Arc::new(Mutex::new(None)) };
            async move { tls_connect(&host, port, verifier).await.map(TokioIo::new) }
        });
        // The URI is only used for :authority; the connector does the real dialing.
        let channel = Endpoint::from_shared(format!("http://{host}:{port}"))?
            .connect_with_connector(connector)
            .await
            .map_err(|e| anyhow::anyhow!("gRPC connect to {host}:{port} failed: {}", error_chain(&e)))?;
        let mut client = DirectorClient::new(channel.clone());

        let nonce = client
            .get_challenge(ChallengeRequest { console_name: console_name.to_string() })
            .await?
            .into_inner()
            .nonce;
        let proof = compute_proof(&password_key(password), &nonce);
        let login = client
            .login(LoginRequest { console_name: console_name.to_string(), nonce, proof })
            .await?
            .into_inner();
        if !login.success {
            anyhow::bail!("login failed: {}", login.error_message);
        }

        Ok(Self {
            client,
            channel,
            token: login.session_token.parse()?,
            director_name: login.director_name,
            version: login.version,
        })
    }

    /// Execute a full command line ("list jobs", ".clients", ...).
    pub async fn execute(&mut self, command: &str, format: ResponseFormat) -> anyhow::Result<CommandOutput> {
        let mut req = tonic::Request::new(CommandRequest {
            command_name: command.to_string(),
            response_format: format as i32,
            ..Default::default()
        });
        req.metadata_mut().insert(SESSION_HEADER, self.token.clone());
        let resp = self.client.execute_command(req).await?.into_inner();
        Ok(CommandOutput { success: resp.success, text: resp.text_output, error: resp.error_message })
    }
}

impl GrpcSession {
    /// Call one RPC of the typed `DirectorApi` with an encoded request and
    /// return the encoded response (S-36: the MCP tools encode/decode with
    /// the descriptors of `bareos_proto_rs::typed`).
    pub async fn call_typed(&mut self, rpc: &str, request: Vec<u8>) -> Result<Vec<u8>, tonic::Status> {
        let mut grpc = tonic::client::Grpc::new(self.channel.clone());
        grpc.ready().await.map_err(|e| tonic::Status::unavailable(format!("gRPC channel not ready: {e}")))?;
        let path = format!("/{}/{rpc}", bareos_proto_rs::typed::DIRECTOR_API_SERVICE);
        let path = tonic::codegen::http::uri::PathAndQuery::try_from(path)
            .map_err(|e| tonic::Status::invalid_argument(format!("RPC name {rpc}: {e}")))?;
        let mut req = tonic::Request::new(request);
        req.metadata_mut().insert(SESSION_HEADER, self.token.clone());
        Ok(grpc.unary(req, path, RawCodec).await?.into_inner())
    }
}

/// Passes the protobuf wire bytes through unchanged.
#[derive(Clone, Copy, Default)]
struct RawCodec;

impl tonic::codec::Codec for RawCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = RawCodec;
    type Decoder = RawCodec;
    fn encoder(&mut self) -> Self::Encoder {
        RawCodec
    }
    fn decoder(&mut self) -> Self::Decoder {
        RawCodec
    }
}

impl tonic::codec::Encoder for RawCodec {
    type Item = Vec<u8>;
    type Error = tonic::Status;
    fn encode(&mut self, item: Vec<u8>, dst: &mut tonic::codec::EncodeBuf<'_>) -> Result<(), tonic::Status> {
        use bytes::BufMut;
        dst.put_slice(&item);
        Ok(())
    }
}

impl tonic::codec::Decoder for RawCodec {
    type Item = Vec<u8>;
    type Error = tonic::Status;
    fn decode(&mut self, src: &mut tonic::codec::DecodeBuf<'_>) -> Result<Option<Vec<u8>>, tonic::Status> {
        use bytes::Buf;
        let mut out = vec![0u8; src.remaining()];
        src.copy_to_slice(&mut out);
        Ok(Some(out))
    }
}

async fn tls_connect(
    host: &str,
    port: u16,
    verifier: PinVerifier,
) -> std::io::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec()];

    let tcp = tokio::net::TcpStream::connect((host, port)).await?;
    // SNI value is irrelevant for pinning; IP literals are not valid DNS names.
    let name = ServerName::try_from(host.to_string())
        .unwrap_or_else(|_| ServerName::try_from("localhost").unwrap());
    tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await
}

#[derive(Debug)]
struct PinVerifier {
    pinned: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = hex::encode(Sha256::digest(end_entity.as_ref()));
        *self.seen.lock().unwrap() = Some(fp.clone());
        match &self.pinned {
            Some(p) if *p == fp => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::General(format!(
                "certificate fingerprint mismatch (server presented {fp})"
            ))),
            None => Err(rustls::Error::General("certificate not pinned".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message, cert, dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// MD5 hex of the password; "[md5]<hex>" values are used as is (same rule as the Director).
pub fn password_key(password: &str) -> String {
    match password.strip_prefix("[md5]") {
        Some(hex) => hex.to_ascii_lowercase(),
        None => bareos_bnet::cram_md5::md5_hash_password(password),
    }
}

pub fn compute_proof(key: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(nonce.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let part = s.to_string();
        // tonic/hyper wrap errors that already repeat their source text
        if !msg.contains(&part) {
            msg.push_str(": ");
            msg.push_str(&part);
        }
        src = s.source();
    }
    msg
}

// ---------------------------------------------------------------------------
// Known Directors (trust on first use)
// ---------------------------------------------------------------------------

/// ~/.config/oconsole/known_directors — one "host:port sha256hex" per line.
pub struct KnownDirectors {
    path: PathBuf,
    entries: BTreeMap<String, String>,
}

impl KnownDirectors {
    pub fn load() -> Self {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        Self::load_from(base.join("oconsole").join("known_directors"))
    }

    pub fn load_from(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.to_string(), it.next()?.to_ascii_lowercase()))
            })
            .collect();
        Self { path, entries }
    }

    pub fn get(&self, host: &str, port: u16) -> Option<&str> {
        self.entries.get(&format!("{host}:{port}")).map(String::as_str)
    }

    pub fn remember(&mut self, host: &str, port: u16, fingerprint: &str) -> std::io::Result<()> {
        self.entries.insert(format!("{host}:{port}"), fingerprint.to_ascii_lowercase());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let content: String = self.entries.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
        std::fs::write(&self.path, content)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_matches_director_rule() {
        // HMAC-SHA256(key = md5hex("secret"), "abc")
        let key = password_key("secret");
        assert_eq!(key, "5ebe2294ecd0e0f08eab7690d2a6ee69");
        assert_eq!(password_key("[md5]5EBE2294ECD0E0F08EAB7690D2A6EE69"), key);
        assert_eq!(compute_proof(&key, "abc").len(), 64);
    }

    #[test]
    fn known_directors_roundtrip() {
        let path = std::env::temp_dir().join(format!("oconsole-known-{}", std::process::id()));
        let mut kd = KnownDirectors::load_from(path.clone());
        assert!(kd.get("dir", 10101).is_none());
        kd.remember("dir", 10101, "ABCDEF").unwrap();
        let kd2 = KnownDirectors::load_from(path.clone());
        assert_eq!(kd2.get("dir", 10101), Some("abcdef"));
        std::fs::remove_file(path).unwrap();
    }
}
