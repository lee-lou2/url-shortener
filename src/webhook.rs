use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    redirect::Policy,
    Client,
};
use serde::Serialize;
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;
use url::{Host, Url};

pub fn is_global_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, third, _] = ip.octets();
            !matches!(
                (first, second, third),
                (0 | 10 | 127 | 224..=255, _, _)
                    | (100, 64..=127, _)
                    | (169, 254, _)
                    | (172, 16..=31, _)
                    | (192, 0, 0 | 2)
                    | (192, 88, 99)
                    | (192, 168, _)
                    | (198, 18 | 19, _)
                    | (198, 51, 100)
                    | (203, 0, 113)
            )
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_global_ip(mapped.into());
            }
            let [first, second, ..] = ip.segments();
            // Restrict egress to global unicast, excluding protocol and transition prefixes.
            (0x2000..=0x3fff).contains(&first)
                && !matches!(
                    (first, second),
                    (0x2001, 0..=0x01ff | 0x0db8) | (0x2002 | 0x3fff, _)
                )
        }
    }
}

pub(crate) fn allowed_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && match url.host() {
            Some(Host::Ipv4(ip)) => is_global_ip(ip.into()),
            Some(Host::Ipv6(ip)) => is_global_ip(ip.into()),
            Some(Host::Domain(domain)) => {
                let domain = domain.trim_end_matches('.');
                domain.contains('.') && domain != "localhost" && !domain.ends_with(".localhost")
            }
            None => false,
        }
}

struct SafeDnsResolver;

impl Resolve for SafeDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|address| !is_global_ip(address.ip())) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Webhook DNS address is disallowed",
                )
                .into());
            }
            let addrs: Addrs = Box::new(addresses.into_iter());
            Ok(addrs)
        })
    }
}

#[derive(Clone)]
pub struct Webhooks {
    client: Client,
    slots: Arc<Semaphore>,
    tasks: TaskTracker,
}

#[derive(Serialize)]
struct WebhookPayload {
    short_key: String,
    user_agent: String,
}

impl Webhooks {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .referer(false)
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(0)
            .dns_resolver(Arc::new(SafeDnsResolver))
            .build()?;
        Ok(Self {
            client,
            slots: Arc::new(Semaphore::new(64)),
            tasks: TaskTracker::new(),
        })
    }

    pub fn dispatch(&self, url: &Url, short_key: String, user_agent: &str) {
        if !allowed_url(url) {
            return;
        }
        let Ok(permit) = self.slots.clone().try_acquire_owned() else {
            tracing::debug!("Webhook concurrency limit reached");
            return;
        };

        let client = self.client.clone();
        let url = url.clone();
        let payload = WebhookPayload {
            short_key,
            user_agent: user_agent.chars().take(512).collect(),
        };
        self.tasks.spawn(async move {
            let _permit = permit;
            match client.post(url).json(&payload).send().await {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => tracing::debug!(status = %response.status(), "Webhook rejected"),
                Err(error) => {
                    tracing::debug!(error = %error.without_url(), "Webhook request failed");
                }
            }
        });
    }

    pub async fn drain(&self) {
        self.slots.close();
        self.tasks.close();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.tasks.wait()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_global_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.17.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "ff00::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_global_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn accepts_global_addresses() {
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(is_global_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn rejects_local_hostnames_and_credentials() {
        for value in [
            "http://localhost./",
            "http://a.localhost",
            "http://postgres",
            "https://user@example.com",
        ] {
            assert!(!allowed_url(&Url::parse(value).unwrap()), "{value}");
        }
    }
}
