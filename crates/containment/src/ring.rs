use sjel_inference::ReviewedProvidersList;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ring {
    Trusted,
    Contained,
}

/// `trusted` only for a listed provider whose entry says `ring = "trusted"` and whose review
/// has not expired. Every other provider, listed or not, is `contained`.
pub fn classify_ring(providers: &ReviewedProvidersList, provider: &str, today: &str) -> Ring {
    let trusted = providers
        .check_admission(provider, "c0", today)
        .is_ok_and(|entry| entry.ring.as_deref() == Some("trusted"));
    if trusted {
        Ring::Trusted
    } else {
        Ring::Contained
    }
}

/// The API hosts the egress proxy admits for `provider`. Empty for an unlisted provider.
pub fn provider_hosts(providers: &ReviewedProvidersList, provider: &str) -> Vec<String> {
    providers
        .get(provider)
        .map(|entry| entry.hosts.clone())
        .unwrap_or_default()
}
