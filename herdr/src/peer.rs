//! Authenticated, encrypted peer sessions.
//!
//! Discovery and synchronization are deliberately outside this module. A
//! caller must provide the expected membership record; a network address or a
//! display name is never treated as authentication. The wire format is
//! length-prefixed, versioned, bounded, encrypted with an ephemeral X25519
//! key agreement, and authenticated with the device's Ed25519 identity.

use chacha20poly1305::{
    aead::{Aead, Payload},
    ChaCha20Poly1305, KeyInit, Nonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use x25519_dalek::{EphemeralSecret, PublicKey};

pub(crate) const PEER_PROTOCOL_VERSION: u16 = 1;
pub(crate) const MAX_PEER_HELLO_BYTES: usize = 8 * 1024;
pub(crate) const MAX_PEER_PAYLOAD_BYTES: usize = 256 * 1024;
const FRAME_HEADER_BYTES: usize = 2 + 8 + 4;
const AEAD_TAG_BYTES: usize = 16;
const MAX_WIRE_FRAME_BYTES: usize = FRAME_HEADER_BYTES + MAX_PEER_PAYLOAD_BYTES + AEAD_TAG_BYTES;
const FRAME_LENGTH_BYTES: usize = 4;
const HANDSHAKE_CONTEXT: &[u8] = b"team-assistant/peer-session/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PeerHello {
    pub protocol_version: u16,
    pub team_id: String,
    pub member_id: String,
    pub device_id: String,
    pub public_key: String,
    pub ephemeral_public_key: [u8; 32],
    pub nonce: [u8; 32],
    pub signature: String,
}

impl PeerHello {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, PeerError> {
        self.validate_shape()?;
        let encoded = serde_json::to_vec(self).map_err(|_| PeerError::MalformedHello)?;
        if encoded.len() > MAX_PEER_HELLO_BYTES {
            return Err(PeerError::HelloTooLarge);
        }
        Ok(encoded)
    }

    pub(crate) fn decode(encoded: &[u8]) -> Result<Self, PeerError> {
        if encoded.len() > MAX_PEER_HELLO_BYTES {
            return Err(PeerError::HelloTooLarge);
        }
        let hello: Self = serde_json::from_slice(encoded).map_err(|_| PeerError::MalformedHello)?;
        hello.validate_shape()?;
        Ok(hello)
    }

    fn signing_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&(
            self.protocol_version,
            &self.team_id,
            &self.member_id,
            &self.device_id,
            &self.public_key,
            self.ephemeral_public_key,
            self.nonce,
        ))
        .expect("peer hello signing fields must be serializable")
    }

    fn validate_shape(&self) -> Result<(), PeerError> {
        if self.protocol_version != PEER_PROTOCOL_VERSION {
            return Err(PeerError::UnsupportedProtocol(self.protocol_version));
        }
        for (field, value) in [
            ("team_id", self.team_id.as_str()),
            ("member_id", self.member_id.as_str()),
            ("device_id", self.device_id.as_str()),
            ("public_key", self.public_key.as_str()),
            ("signature", self.signature.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(PeerError::InvalidHello(field));
            }
        }
        if self.ephemeral_public_key.iter().all(|byte| *byte == 0) {
            return Err(PeerError::InvalidHello("ephemeral public key"));
        }
        if self.nonce.iter().all(|byte| *byte == 0) {
            return Err(PeerError::InvalidHello("nonce"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerMembership {
    pub team_id: String,
    pub member_id: String,
    pub device_id: String,
    pub public_key: String,
    pub active: bool,
}

impl PeerMembership {
    fn validates(&self, hello: &PeerHello) -> Result<(), PeerError> {
        if !self.active
            || self.team_id != hello.team_id
            || self.member_id != hello.member_id
            || self.device_id != hello.device_id
            || self.public_key != hello.public_key
        {
            return Err(PeerError::UnauthorizedPeer);
        }
        DeviceIdentity::verify(&hello.public_key, &hello.signing_bytes(), &hello.signature)
            .map_err(|_| PeerError::InvalidSignature)
    }
}

pub(crate) struct PendingPeerHandshake {
    hello: PeerHello,
    ephemeral_secret: EphemeralSecret,
}

impl PendingPeerHandshake {
    pub(crate) fn begin(
        identity: &DeviceIdentity,
        team_id: impl Into<String>,
        member_id: impl Into<String>,
    ) -> Result<Self, PeerError> {
        let team_id = team_id.into();
        let member_id = member_id.into();
        if team_id.trim().is_empty() || member_id.trim().is_empty() {
            return Err(PeerError::InvalidHello("local team or member"));
        }
        let ephemeral_secret = EphemeralSecret::random_from_rng(OsRng);
        let ephemeral_public_key = PublicKey::from(&ephemeral_secret).to_bytes();
        let mut nonce = [0_u8; 32];
        getrandom_bytes(&mut nonce)?;
        let mut hello = PeerHello {
            protocol_version: PEER_PROTOCOL_VERSION,
            team_id,
            member_id,
            device_id: identity.device_id().to_string(),
            public_key: identity.public_key(),
            ephemeral_public_key,
            nonce,
            signature: String::new(),
        };
        hello.signature = identity.sign(&hello.signing_bytes());
        Ok(Self {
            hello,
            ephemeral_secret,
        })
    }

    pub(crate) fn hello(&self) -> &PeerHello {
        &self.hello
    }

    pub(crate) fn complete(
        self,
        remote: &PeerHello,
        expected_remote: &PeerMembership,
    ) -> Result<SecurePeerSession, PeerError> {
        remote.validate_shape()?;
        expected_remote.validates(remote)?;
        if self.hello.team_id != remote.team_id {
            return Err(PeerError::UnauthorizedPeer);
        }
        let remote_public = PublicKey::from(remote.ephemeral_public_key);
        let shared_secret = self.ephemeral_secret.diffie_hellman(&remote_public);
        if shared_secret.as_bytes().iter().all(|byte| *byte == 0) {
            return Err(PeerError::InvalidHello("low-order ephemeral key"));
        }
        let key = derive_session_key(&self.hello, remote, shared_secret.as_bytes());
        let send_direction = if self.hello.device_id < remote.device_id {
            0
        } else {
            1
        };
        Ok(SecurePeerSession {
            key,
            local_device_id: self.hello.device_id,
            remote_device_id: remote.device_id.clone(),
            send_direction,
            send_sequence: 0,
            receive_direction: 1 - send_direction,
            highest_received: None,
        })
    }
}

pub(crate) struct SecurePeerSession {
    key: [u8; 32],
    local_device_id: String,
    remote_device_id: String,
    send_direction: u8,
    send_sequence: u64,
    receive_direction: u8,
    highest_received: Option<u64>,
}

impl fmt::Debug for SecurePeerSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurePeerSession")
            .field("local_device_id", &self.local_device_id)
            .field("remote_device_id", &self.remote_device_id)
            .field("send_sequence", &self.send_sequence)
            .field("highest_received", &self.highest_received)
            .finish_non_exhaustive()
    }
}

impl SecurePeerSession {
    pub(crate) fn seal(&mut self, payload: &[u8]) -> Result<Vec<u8>, PeerError> {
        if payload.len() > MAX_PEER_PAYLOAD_BYTES {
            return Err(PeerError::PayloadTooLarge {
                actual: payload.len(),
                maximum: MAX_PEER_PAYLOAD_BYTES,
            });
        }
        let sequence = self.send_sequence;
        self.send_sequence = self
            .send_sequence
            .checked_add(1)
            .ok_or(PeerError::SequenceExhausted)?;
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let nonce = frame_nonce(self.send_direction, sequence);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: payload,
                    aad: &frame_aad(sequence),
                },
            )
            .map_err(|_| PeerError::EncryptionFailed)?;
        let ciphertext_len =
            u32::try_from(ciphertext.len()).map_err(|_| PeerError::FrameTooLarge)?;
        let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + ciphertext.len());
        frame.extend_from_slice(&PEER_PROTOCOL_VERSION.to_be_bytes());
        frame.extend_from_slice(&sequence.to_be_bytes());
        frame.extend_from_slice(&ciphertext_len.to_be_bytes());
        frame.extend_from_slice(&ciphertext);
        Ok(frame)
    }

    pub(crate) fn open(&mut self, frame: &[u8]) -> Result<Vec<u8>, PeerError> {
        if frame.len() > MAX_WIRE_FRAME_BYTES {
            return Err(PeerError::FrameTooLarge);
        }
        if frame.len() < FRAME_HEADER_BYTES {
            return Err(PeerError::MalformedFrame);
        }
        let version = u16::from_be_bytes([frame[0], frame[1]]);
        if version != PEER_PROTOCOL_VERSION {
            return Err(PeerError::UnsupportedProtocol(version));
        }
        let sequence = u64::from_be_bytes(frame[2..10].try_into().expect("sequence width"));
        if self
            .highest_received
            .is_some_and(|highest| sequence <= highest)
        {
            return Err(PeerError::Replay(sequence));
        }
        let ciphertext_len =
            u32::from_be_bytes(frame[10..14].try_into().expect("ciphertext length width")) as usize;
        if ciphertext_len < AEAD_TAG_BYTES
            || ciphertext_len > MAX_PEER_PAYLOAD_BYTES + AEAD_TAG_BYTES
            || ciphertext_len != frame.len() - FRAME_HEADER_BYTES
        {
            return Err(PeerError::MalformedFrame);
        }
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let nonce = frame_nonce(self.receive_direction, sequence);
        let payload = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &frame[FRAME_HEADER_BYTES..],
                    aad: &frame_aad(sequence),
                },
            )
            .map_err(|_| PeerError::DecryptionFailed)?;
        self.highest_received = Some(sequence);
        Ok(payload)
    }
}

pub(crate) trait PeerTransport {
    fn send(&mut self, payload: &[u8]) -> Result<(), PeerError>;
    fn receive(&mut self) -> Result<Vec<u8>, PeerError>;
}

pub(crate) struct SecurePeerChannel<S> {
    stream: S,
    session: SecurePeerSession,
}

impl<S> SecurePeerChannel<S> {
    pub(crate) fn new(stream: S, session: SecurePeerSession) -> Self {
        Self { stream, session }
    }
}

impl<S: Read + Write> PeerTransport for SecurePeerChannel<S> {
    fn send(&mut self, payload: &[u8]) -> Result<(), PeerError> {
        let frame = self.session.seal(payload)?;
        let length = u32::try_from(frame.len()).map_err(|_| PeerError::FrameTooLarge)?;
        self.stream.write_all(&length.to_be_bytes())?;
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        Ok(())
    }

    fn receive(&mut self) -> Result<Vec<u8>, PeerError> {
        let mut length_bytes = [0_u8; FRAME_LENGTH_BYTES];
        self.stream.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes) as usize;
        if length > MAX_WIRE_FRAME_BYTES {
            return Err(PeerError::FrameTooLarge);
        }
        let mut frame = vec![0_u8; length];
        self.stream.read_exact(&mut frame)?;
        self.session.open(&frame)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerTransportConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
}

impl Default for PeerTransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(10),
        }
    }
}

pub(crate) fn connect_tcp(
    address: SocketAddr,
    config: PeerTransportConfig,
) -> Result<TcpStream, PeerError> {
    let stream = TcpStream::connect_timeout(&address, config.connect_timeout)?;
    stream.set_read_timeout(Some(config.read_timeout))?;
    stream.set_write_timeout(Some(config.write_timeout))?;
    Ok(stream)
}

pub(crate) fn accept_tcp(
    listener: &std::net::TcpListener,
    config: PeerTransportConfig,
) -> Result<(TcpStream, SocketAddr), PeerError> {
    let (stream, address) = listener.accept()?;
    stream.set_read_timeout(Some(config.read_timeout))?;
    stream.set_write_timeout(Some(config.write_timeout))?;
    Ok((stream, address))
}

fn derive_session_key(local: &PeerHello, remote: &PeerHello, shared_secret: &[u8; 32]) -> [u8; 32] {
    let mut endpoints = [
        (&local.device_id, &local.nonce),
        (&remote.device_id, &remote.nonce),
    ];
    endpoints.sort_by(|left, right| left.0.cmp(right.0));
    let mut digest = Sha256::new();
    digest.update(HANDSHAKE_CONTEXT);
    digest.update(local.team_id.as_bytes());
    for (device_id, nonce) in endpoints {
        digest.update(device_id.as_bytes());
        digest.update(nonce);
    }
    digest.update(shared_secret);
    digest.finalize().into()
}

fn frame_nonce(direction: u8, sequence: u64) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[0] = direction;
    nonce[4..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn frame_aad(sequence: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(2 + 8);
    aad.extend_from_slice(&PEER_PROTOCOL_VERSION.to_be_bytes());
    aad.extend_from_slice(&sequence.to_be_bytes());
    aad
}

fn getrandom_bytes(output: &mut [u8; 32]) -> Result<(), PeerError> {
    OsRng.fill_bytes(output);
    Ok(())
}

#[derive(Debug)]
pub(crate) enum PeerError {
    Io(io::Error),
    InvalidHello(&'static str),
    HelloTooLarge,
    MalformedHello,
    UnauthorizedPeer,
    InvalidSignature,
    UnsupportedProtocol(u16),
    PayloadTooLarge { actual: usize, maximum: usize },
    FrameTooLarge,
    MalformedFrame,
    Replay(u64),
    SequenceExhausted,
    EncryptionFailed,
    DecryptionFailed,
}

impl fmt::Display for PeerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "peer transport I/O error: {error}"),
            Self::InvalidHello(field) => write!(formatter, "invalid peer hello: {field}"),
            Self::HelloTooLarge => formatter.write_str("peer hello is too large"),
            Self::MalformedHello => formatter.write_str("peer hello is malformed"),
            Self::UnauthorizedPeer => formatter.write_str("peer is not an active team member"),
            Self::InvalidSignature => formatter.write_str("peer hello signature is invalid"),
            Self::UnsupportedProtocol(version) => {
                write!(formatter, "unsupported peer protocol version: {version}")
            }
            Self::PayloadTooLarge { actual, maximum } => {
                write!(formatter, "peer payload is too large: {actual} > {maximum}")
            }
            Self::FrameTooLarge => formatter.write_str("peer frame is too large"),
            Self::MalformedFrame => formatter.write_str("peer frame is malformed"),
            Self::Replay(sequence) => write!(formatter, "peer frame replayed: sequence {sequence}"),
            Self::SequenceExhausted => formatter.write_str("peer frame sequence exhausted"),
            Self::EncryptionFailed => formatter.write_str("peer payload encryption failed"),
            Self::DecryptionFailed => formatter.write_str("peer payload decryption failed"),
        }
    }
}

impl std::error::Error for PeerError {}

impl From<io::Error> for PeerError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

use crate::identity::DeviceIdentity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{Event, EventType};
    use serde_json::json;
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-peer-{label}-{}-{nanos}.identity",
            std::process::id()
        ))
    }

    fn membership(identity: &DeviceIdentity, member_id: &str) -> PeerMembership {
        PeerMembership {
            team_id: "team-1".into(),
            member_id: member_id.into(),
            device_id: identity.device_id().into(),
            public_key: identity.public_key(),
            active: true,
        }
    }

    fn establish_pair() -> (
        SecurePeerSession,
        SecurePeerSession,
        DeviceIdentity,
        DeviceIdentity,
    ) {
        let first_path = test_path("first");
        let second_path = test_path("second");
        let first = DeviceIdentity::load_or_create(&first_path).unwrap();
        let second = DeviceIdentity::load_or_create(&second_path).unwrap();
        let first_pending = PendingPeerHandshake::begin(&first, "team-1", "member-1").unwrap();
        let second_pending = PendingPeerHandshake::begin(&second, "team-1", "member-2").unwrap();
        let first_hello = first_pending.hello().clone();
        let second_hello = second_pending.hello().clone();
        let first_session = first_pending
            .complete(&second_hello, &membership(&second, "member-2"))
            .unwrap();
        let second_session = second_pending
            .complete(&first_hello, &membership(&first, "member-1"))
            .unwrap();
        let _ = fs::remove_file(first_path);
        let _ = fs::remove_file(second_path);
        (first_session, second_session, first, second)
    }

    #[test]
    fn authorized_peers_exchange_an_encrypted_signed_event() {
        let (mut first_session, mut second_session, first, _second) = establish_pair();
        let mut event = Event {
            event_id: "event-1".into(),
            team_id: "team-1".into(),
            actor_member_id: "member-1".into(),
            device_id: first.device_id().into(),
            event_type: EventType::TaskCreated,
            payload: json!({"task_id":"task-1"}),
            schema_version: crate::team::SUPPORTED_EVENT_SCHEMA_VERSION,
            logical_time: 1,
            signature: None,
        };
        event.signature = Some(first.sign(&event.signing_bytes()));
        event.validate().unwrap();
        let encoded = serde_json::to_vec(&event).unwrap();
        let frame = first_session.seal(&encoded).unwrap();
        let decoded: Event = serde_json::from_slice(&second_session.open(&frame).unwrap()).unwrap();
        DeviceIdentity::verify(
            &first.public_key(),
            &decoded.signing_bytes(),
            decoded.signature.as_deref().unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn peer_hello_is_bounded_and_round_trips() {
        let path = test_path("hello");
        let identity = DeviceIdentity::load_or_create(&path).unwrap();
        let pending = PendingPeerHandshake::begin(&identity, "team-1", "member-1").unwrap();
        let encoded = pending.hello().encode().unwrap();
        assert_eq!(PeerHello::decode(&encoded).unwrap(), *pending.hello());
        let mut oversized = vec![b' '; MAX_PEER_HELLO_BYTES + 1];
        oversized[0] = b'{';
        assert!(matches!(
            PeerHello::decode(&oversized),
            Err(PeerError::HelloTooLarge)
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unknown_or_inactive_peer_is_rejected_before_session_creation() {
        let first_path = test_path("first-reject");
        let second_path = test_path("second-reject");
        let first = DeviceIdentity::load_or_create(&first_path).unwrap();
        let second = DeviceIdentity::load_or_create(&second_path).unwrap();
        let pending = PendingPeerHandshake::begin(&first, "team-1", "member-1").unwrap();
        let remote = PendingPeerHandshake::begin(&second, "team-1", "member-2").unwrap();
        let mut expected = membership(&second, "member-2");
        expected.active = false;
        assert!(matches!(
            pending.complete(remote.hello(), &expected),
            Err(PeerError::UnauthorizedPeer)
        ));
        let _ = fs::remove_file(first_path);
        let _ = fs::remove_file(second_path);
    }

    #[test]
    fn tampering_replay_and_oversized_payloads_are_rejected() {
        let (mut first_session, mut second_session, _first, _second) = establish_pair();
        let frame = first_session.seal(b"hello").unwrap();
        assert_eq!(second_session.open(&frame).unwrap(), b"hello");
        assert!(matches!(
            second_session.open(&frame),
            Err(PeerError::Replay(0))
        ));

        let mut tampered = first_session.seal(b"tamper").unwrap();
        tampered[20] ^= 1;
        assert!(matches!(
            second_session.open(&tampered),
            Err(PeerError::DecryptionFailed)
        ));
        assert!(matches!(
            first_session.seal(&vec![0_u8; MAX_PEER_PAYLOAD_BYTES + 1]),
            Err(PeerError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn framed_channel_round_trips_over_loopback_tcp() {
        let (first_session, second_session, _first, _second) = establish_pair();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = accept_tcp(&listener, PeerTransportConfig::default()).unwrap();
            let mut channel = SecurePeerChannel::new(stream, second_session);
            channel.receive().unwrap()
        });
        let stream = connect_tcp(address, PeerTransportConfig::default()).unwrap();
        let mut channel = SecurePeerChannel::new(stream, first_session);
        channel.send(b"framed").unwrap();
        assert_eq!(server.join().unwrap(), b"framed");
    }
}
