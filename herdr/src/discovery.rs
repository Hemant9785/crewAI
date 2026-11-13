//! mDNS-only peer discovery with a manual address fallback.
//!
//! An advertisement contains enough metadata to start the authenticated peer
//! handshake, but resolving an mDNS record is never proof of team membership.
//! The signed peer hello in `peer.rs` remains the authentication boundary.

use crate::identity::DeviceIdentity;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

pub(crate) const SERVICE_TYPE: &str = "_team-assistant._tcp.local.";
const ADVERTISEMENT_VERSION: &str = "1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveryAdvertisement {
    pub team_id: String,
    pub member_id: String,
    pub device_id: String,
    pub public_key: String,
    pub port: u16,
    pub signature: String,
}

impl DiscoveryAdvertisement {
    pub(crate) fn for_identity(
        identity: &DeviceIdentity,
        team_id: impl Into<String>,
        member_id: impl Into<String>,
        port: u16,
    ) -> Result<Self, DiscoveryError> {
        let team_id = team_id.into();
        let member_id = member_id.into();
        validate_text(&team_id, "team_id")?;
        validate_text(&member_id, "member_id")?;
        let mut advertisement = Self {
            team_id,
            member_id,
            device_id: identity.device_id().into(),
            public_key: identity.public_key(),
            port,
            signature: String::new(),
        };
        advertisement.signature = identity.sign(&advertisement.signing_bytes());
        Ok(advertisement)
    }

    pub(crate) fn verify(&self) -> Result<(), DiscoveryError> {
        validate_text(&self.team_id, "team_id")?;
        validate_text(&self.member_id, "member_id")?;
        validate_text(&self.device_id, "device_id")?;
        validate_text(&self.public_key, "public_key")?;
        validate_text(&self.signature, "signature")?;
        DeviceIdentity::verify(&self.public_key, &self.signing_bytes(), &self.signature)
            .map_err(|_| DiscoveryError::InvalidSignature)
    }

    pub(crate) fn service_info(&self) -> Result<ServiceInfo, DiscoveryError> {
        self.verify()?;
        let instance = format!("{}-{}", self.device_id, self.port);
        let host = format!("{}.local.", self.device_id);
        let properties = [
            ("version", ADVERTISEMENT_VERSION.to_string()),
            ("team_id", self.team_id.clone()),
            ("member_id", self.member_id.clone()),
            ("device_id", self.device_id.clone()),
            ("public_key", self.public_key.clone()),
            ("signature", self.signature.clone()),
        ];
        ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            (),
            self.port,
            &properties[..],
        )
        .map_err(DiscoveryError::Mdns)
    }

    fn signing_bytes(&self) -> Vec<u8> {
        [
            ADVERTISEMENT_VERSION,
            self.team_id.as_str(),
            self.member_id.as_str(),
            self.device_id.as_str(),
            self.public_key.as_str(),
            &self.port.to_string(),
        ]
        .join("\n")
        .into_bytes()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiscoveredPeer {
    pub team_id: String,
    pub member_id: String,
    pub device_id: String,
    pub public_key: String,
    pub address: SocketAddr,
}

impl DiscoveredPeer {
    fn from_service(service: &mdns_sd::ResolvedService) -> Result<Self, DiscoveryError> {
        if service.get_property_val_str("version") != Some(ADVERTISEMENT_VERSION) {
            return Err(DiscoveryError::UnsupportedAdvertisement);
        }
        let port = service.get_port();
        let address = service
            .get_addresses_v4()
            .into_iter()
            .next()
            .map(|ip| SocketAddr::new(IpAddr::V4(ip), port))
            .ok_or(DiscoveryError::AddressMissing)?;
        let advertisement = DiscoveryAdvertisement {
            team_id: property(service, "team_id")?,
            member_id: property(service, "member_id")?,
            device_id: property(service, "device_id")?,
            public_key: property(service, "public_key")?,
            port,
            signature: property(service, "signature")?,
        };
        advertisement.verify()?;
        Ok(Self {
            team_id: advertisement.team_id,
            member_id: advertisement.member_id,
            device_id: advertisement.device_id,
            public_key: advertisement.public_key,
            address,
        })
    }
}

pub(crate) fn publish(
    daemon: &ServiceDaemon,
    advertisement: &DiscoveryAdvertisement,
) -> Result<(), DiscoveryError> {
    daemon.register(advertisement.service_info()?)?;
    Ok(())
}

pub(crate) fn discover(
    daemon: &ServiceDaemon,
    timeout: Duration,
) -> Result<Vec<DiscoveredPeer>, DiscoveryError> {
    let receiver = daemon.browse(SERVICE_TYPE)?;
    let deadline = Instant::now() + timeout;
    let mut peers = Vec::new();
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                if let Ok(peer) = DiscoveredPeer::from_service(&service) {
                    if !peers
                        .iter()
                        .any(|existing: &DiscoveredPeer| existing.device_id == peer.device_id)
                    {
                        peers.push(peer);
                    }
                }
            }
            Ok(ServiceEvent::ServiceRemoved(_, _)) => {}
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    Ok(peers)
}

pub(crate) fn parse_manual_peer_address(value: &str) -> Result<SocketAddr, DiscoveryError> {
    value
        .parse()
        .map_err(|_| DiscoveryError::InvalidManualAddress(value.into()))
}

fn property(
    service: &mdns_sd::ResolvedService,
    key: &'static str,
) -> Result<String, DiscoveryError> {
    service
        .get_property_val_str(key)
        .map(str::to_string)
        .ok_or(DiscoveryError::MissingProperty(key))
}

fn validate_text(value: &str, field: &'static str) -> Result<(), DiscoveryError> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(DiscoveryError::InvalidField(field));
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum DiscoveryError {
    Mdns(mdns_sd::Error),
    InvalidField(&'static str),
    InvalidSignature,
    UnsupportedAdvertisement,
    AddressMissing,
    MissingProperty(&'static str),
    InvalidManualAddress(String),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mdns(error) => write!(formatter, "mDNS discovery error: {error}"),
            Self::InvalidField(field) => write!(formatter, "invalid discovery field: {field}"),
            Self::InvalidSignature => formatter.write_str("discovery signature is invalid"),
            Self::UnsupportedAdvertisement => {
                formatter.write_str("unsupported discovery advertisement")
            }
            Self::AddressMissing => formatter.write_str("discovery record has no IPv4 address"),
            Self::MissingProperty(key) => write!(formatter, "discovery property is missing: {key}"),
            Self::InvalidManualAddress(address) => {
                write!(formatter, "invalid manual peer address: {address}")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

impl From<mdns_sd::Error> for DiscoveryError {
    fn from(error: mdns_sd::Error) -> Self {
        Self::Mdns(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn identity_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-discovery-{label}-{}-{nanos}.identity",
            std::process::id()
        ))
    }

    #[test]
    fn signed_advertisements_produce_standard_mdns_service_info() {
        let path = identity_path("advertisement");
        let identity = DeviceIdentity::load_or_create(&path).unwrap();
        let advertisement =
            DiscoveryAdvertisement::for_identity(&identity, "team-1", "member-1", 43123).unwrap();
        advertisement.verify().unwrap();
        let service = advertisement.service_info().unwrap();
        assert_eq!(service.get_port(), 43123);
        assert_eq!(service.get_property_val_str("team_id"), Some("team-1"));
        assert_eq!(
            service.get_property_val_str("device_id"),
            Some(identity.device_id())
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn tampered_advertisement_is_not_accepted_as_authenticated() {
        let path = identity_path("tampered");
        let identity = DeviceIdentity::load_or_create(&path).unwrap();
        let mut advertisement =
            DiscoveryAdvertisement::for_identity(&identity, "team-1", "member-1", 43123).unwrap();
        advertisement.team_id = "other-team".into();
        assert!(matches!(
            advertisement.verify(),
            Err(DiscoveryError::InvalidSignature)
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn manual_address_fallback_is_explicit() {
        assert_eq!(
            parse_manual_peer_address("192.168.1.20:43123").unwrap(),
            "192.168.1.20:43123".parse().unwrap()
        );
        assert!(parse_manual_peer_address("not-an-address").is_err());
    }
}
