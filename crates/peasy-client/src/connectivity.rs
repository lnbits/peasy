//! Conservative presentation of failed Internet operations. Never gate local
//! work or cached Nix operations, and never infer offline from a server error.

pub const OFFLINE_MESSAGE: &str = "Sorry, no internet connection";

pub fn offline_message(diagnostics: &str) -> Option<&'static str> {
    if confirmed_offline(diagnostics, disconnected()) {
        Some(OFFLINE_MESSAGE)
    } else {
        None
    }
}

fn confirmed_offline(diagnostics: &str, disconnected: Option<bool>) -> bool {
    disconnected == Some(true) && internet_transport_failure(diagnostics)
}

fn internet_transport_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    // A loopback service failure is never evidence of an Internet failure.
    let remote = message.split_whitespace().any(|word| {
        let Some(start) = word.find("http") else {
            return false;
        };
        let url = word[start..].trim_end_matches([')', ']', '\'', '"', ',', ':']);
        reqwest::Url::parse(url).ok().is_some_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some_and(|host| {
                    let host = host.trim_matches(['[', ']']);
                    host != "localhost"
                        && host.parse::<std::net::IpAddr>().map_or(
                            !host.ends_with(".local"),
                            |ip| {
                                !ip.is_loopback()
                                    && !ip.is_unspecified()
                                    && match ip {
                                        std::net::IpAddr::V4(ip) => {
                                            !ip.is_private() && !ip.is_link_local()
                                        }
                                        std::net::IpAddr::V6(ip) => {
                                            !ip.is_unique_local() && !ip.is_unicast_link_local()
                                        }
                                    }
                            },
                        )
                })
        })
    });
    remote
        && [
            "could not resolve host",
            "couldn't resolve host",
            "dns error",
            "failed to lookup address",
            "no such host",
            "network is unreachable",
            "network is down",
            "failed to connect",
            "couldn't connect",
            "connection refused",
            "connection timed out",
            "operation timed out",
            "connect error",
        ]
        .iter()
        .any(|reason| message.contains(reason))
}

/// Only assert disconnected when the OS has no running, addressed external
/// interface. Link-local addresses, tunnels and unrecognised states stay unknown.
/// An active interface does not prove Internet access: retain the real error in
/// that case, including captive portals, broken DNS and upstream outages.
fn disconnected() -> Option<bool> {
    let mut interfaces = std::ptr::null_mut();
    // SAFETY: getifaddrs owns a linked list until freeifaddrs; nullable addresses
    // are checked before reading their family. No pointers escape this function.
    unsafe {
        if libc::getifaddrs(&mut interfaces) != 0 {
            return None;
        }
        let mut current = interfaces;
        let mut external = false;
        while !current.is_null() {
            let interface = &*current;
            let flags = interface.ifa_flags as libc::c_int;
            if flags & libc::IFF_LOOPBACK == 0
                && flags & libc::IFF_UP != 0
                && flags & libc::IFF_RUNNING != 0
                && !interface.ifa_addr.is_null()
                && matches!(
                    (*interface.ifa_addr).sa_family as libc::c_int,
                    libc::AF_INET | libc::AF_INET6
                )
            {
                external = true;
                break;
            }
            current = interface.ifa_next;
        }
        libc::freeifaddrs(interfaces);
        Some(!external)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_independent_os_evidence() {
        let error = "https://api.openai.com: network is unreachable";
        assert!(confirmed_offline(error, Some(true)));
        assert!(!confirmed_offline(error, Some(false)));
        assert!(!confirmed_offline(error, None));
        assert!(!confirmed_offline(
            "https://api.openai.com: HTTP 401",
            Some(true)
        ));
        assert!(!confirmed_offline(
            "http://127.0.0.1:11434: connection refused",
            Some(true)
        ));
    }
    #[test]
    fn only_remote_transport_failures_are_candidates() {
        for error in [
            "unable to download 'https://cache.nixos.org/file': Could not resolve host",
            "contacting OpenAI: error sending request for url (https://api.openai.com/v1/responses): network is unreachable",
            "https://api.github.com/releases: operation timed out",
            "Ollama: Get \"https://registry.ollama.ai/v2/library/qwen3/manifests/4b\": dial tcp: lookup registry.ollama.ai: no such host",
        ] {
            assert!(internet_transport_failure(error), "{error}");
        }
        for error in [
            "http://127.0.0.1:11434/api/chat: connection refused",
            "http://localhost:11434/api/chat: operation timed out",
            "http://[::1]:11434/api/chat: connection refused",
            "http://printer.local/: failed to connect",
            "http://192.168.1.2/printer: failed to connect",
            "https://api.openai.com/v1/responses: HTTP 401 invalid API key",
            "https://api.openai.com/v1/responses: HTTP 429 rate limit",
            "https://api.github.com: HTTP 503 Service Unavailable",
            "https://cache.nixos.org: certificate verification failed",
            "https://cache.nixos.org: hash mismatch",
            "invalid request: group is not allowed",
            "connecting to /run/peasy/peasy.sock: connection refused",
        ] {
            assert!(!internet_transport_failure(error), "{error}");
        }
    }
}
