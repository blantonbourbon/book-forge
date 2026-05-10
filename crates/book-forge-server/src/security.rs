use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use thiserror::Error;
use tokio::net::lookup_host;
use url::{Host, Url};

const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Error)]
#[error("{message}")]
pub struct SecurityError {
    pub code: &'static str,
    pub message: &'static str,
}

impl SecurityError {
    fn new(message: &'static str) -> Self {
        Self {
            code: "unsafe_url",
            message,
        }
    }
}

pub async fn validate_network_url(url: &Url) -> Result<(), SecurityError> {
    validate_url_without_dns(url)?;

    let Some(domain) = public_domain_for_dns(url) else {
        return Ok(());
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let lookup = tokio::time::timeout(DNS_LOOKUP_TIMEOUT, lookup_host((domain.as_str(), port)));
    let Ok(Ok(addresses)) = lookup.await else {
        return Ok(());
    };

    for address in addresses {
        ensure_public_ip(address.ip())?;
    }

    Ok(())
}

fn validate_url_without_dns(url: &Url) -> Result<(), SecurityError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SecurityError::new(
            "Only HTTP and HTTPS source URLs are supported.",
        ));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(SecurityError::new(
            "URLs with embedded credentials are not allowed.",
        ));
    }

    let Some(host) = url.host() else {
        return Err(SecurityError::new("Source URLs must include a host."));
    };

    match host {
        Host::Ipv4(address) => ensure_public_ip(IpAddr::V4(address)),
        Host::Ipv6(address) => ensure_public_ip(IpAddr::V6(address)),
        Host::Domain(domain) => {
            let normalized = normalize_domain(domain);
            if normalized.is_empty() {
                return Err(SecurityError::new("Source URLs must include a host."));
            }
            if is_blocked_hostname(&normalized) {
                return Err(SecurityError::new(
                    "Localhost and metadata-service targets are not allowed.",
                ));
            }
            if let Ok(address) = normalized.parse::<IpAddr>() {
                ensure_public_ip(address)?;
            }
            Ok(())
        }
    }
}

fn public_domain_for_dns(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(domain) => {
            let normalized = normalize_domain(domain);
            if is_fixture_domain(&normalized) || normalized.parse::<IpAddr>().is_ok() {
                None
            } else {
                Some(normalized)
            }
        }
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

fn is_fixture_domain(domain: &str) -> bool {
    domain == "example.test"
}

fn is_blocked_hostname(domain: &str) -> bool {
    domain == "localhost"
        || domain.ends_with(".localhost")
        || matches!(
            domain,
            "metadata" | "metadata.google.internal" | "169.254.169.254"
        )
}

fn ensure_public_ip(address: IpAddr) -> Result<(), SecurityError> {
    match address {
        IpAddr::V4(address) => ensure_public_ipv4(address),
        IpAddr::V6(address) => ensure_public_ipv6(address),
    }
}

fn ensure_public_ipv4(address: Ipv4Addr) -> Result<(), SecurityError> {
    let octets = address.octets();
    let blocked = address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_unspecified()
        || octets[0] == 0
        || octets[0] >= 240
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 169 && octets[1] == 254)
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (18..=19).contains(&octets[1]));

    if blocked {
        Err(SecurityError::new(
            "Private, local, link-local, metadata, and multicast targets are not allowed.",
        ))
    } else {
        Ok(())
    }
}

fn ensure_public_ipv6(address: Ipv6Addr) -> Result<(), SecurityError> {
    if let Some(mapped) = ipv4_mapped(address) {
        return ensure_public_ipv4(mapped);
    }

    let segments = address.segments();
    let blocked = address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80;

    if blocked {
        Err(SecurityError::new(
            "Private, local, link-local, metadata, and multicast targets are not allowed.",
        ))
    } else {
        Ok(())
    }
}

fn ipv4_mapped(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = address.segments();
    if segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff {
        return Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    if segments[..6] == [0, 0, 0, 0, 0, 0] && (segments[6] != 0 || segments[7] != 0) {
        return Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    None
}
