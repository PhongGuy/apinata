//! Network settings: the collection's proxy and client certificates, what they resolve to, and
//! how each hop is routed.

mod check;
mod clients;
mod fingerprint;
mod hosts;
mod identity;
mod model;
mod pac;
mod resolved;
mod route;

pub use check::{NetworkAnswer, NetworkCheck, NetworkQuestion, QuestionKind, ShownNetwork};
pub(crate) use check::{NetworkBasis, Preloaded};
pub(crate) use clients::{ClientKey, Clients};
pub use fingerprint::{AckState, NetworkAck, NetworkChange, NetworkSnapshot, saved_network};
pub use hosts::{HostOverride, ResolvedHostOverride};
pub(crate) use hosts::{
    OverrideResolve, label_for, overridden, resolve_host_overrides, write_host_overrides,
};
pub(crate) use identity::{load_identity, read_ca};
pub use model::{
    CertificateKind, ClientCertificate, CollectionNetwork, CollectionProxy, ProxyAuth, ProxyConfig,
};
pub(crate) use model::{read_network, write_certificates, write_proxy};
pub(crate) use pac::{FetchOptions, PAC_CACHE, PacCache, PacSend, resolve_hop};
pub use pac::{PacSource, PacSourceError, PacWarning, clear_pac_cache};
pub(crate) use resolved::domain_matches;
pub use resolved::{
    CertPath, CertificateFiles, RequestNetwork, RequestProxy, ResolvedCertificate, ResolvedProxy,
};
pub use route::{GlobalProxy, NetworkSettings};
pub(crate) use route::{ProxyRule, SystemProxy, apply_rule, route, rule_for};
