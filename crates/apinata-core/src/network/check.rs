//! The check before a send: the network settings a send resolved, in memory, against what the
//! person acknowledged. A difference is asked about before anything goes out, and the answer
//! records exactly what the question showed.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::SystemTime;

use super::fingerprint::{
    AckState, Expanded, NetworkAck, NetworkChange, NetworkSnapshot, content_hash, expand_fixed,
    path_hash, root_key, scope_expansions, templates,
};
use super::model::CollectionNetwork;
use super::resolved::domain_matches;
use super::{CertificateFiles, RequestNetwork};
use crate::cancel::CancelHandle;
use crate::context::VarContext;
use crate::document::read_regular_async;
use crate::dynamic::{Generator, next_generated};
use crate::execute::ExecError;
use crate::interpolate::{Vars, interpolate_tracked};
use crate::resolve::ResolvedRequest;
use crate::trust::with_lock;

/// Why a question is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QuestionKind {
    /// The settings differ from what was acknowledged, or a folder, request or run variable
    /// sends this request elsewhere than the collection's settings say.
    Changed,
    /// The collection was never seen here and sets something; this only informs.
    FirstSeen,
}

/// The answer to a [`NetworkQuestion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkAnswer {
    /// Send, and record what the question showed as seen.
    Send,
    /// Send this once without recording anything.
    SendOnce,
    /// Do not send.
    Cancel,
}

/// What a question or notice showed, so acknowledging records that and not a later state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShownNetwork {
    pub(crate) salt: String,
    pub(crate) collection: Option<NetworkSnapshot>,
    pub(crate) send: Option<String>,
    /// The fingerprint of the collection settings `send` was judged against.
    pub(crate) judged: Option<String>,
}

impl ShownNetwork {
    /// The collection's settings as `snapshot` shows them, built with the record's `salt`.
    pub fn collection(salt: impl Into<String>, snapshot: NetworkSnapshot) -> ShownNetwork {
        ShownNetwork {
            salt: salt.into(),
            collection: Some(snapshot),
            send: None,
            judged: None,
        }
    }
}

/// A question before a send: what changed, worded without any secret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetworkQuestion {
    pub(crate) root: PathBuf,
    pub(crate) kind: QuestionKind,
    pub(crate) changes: Vec<NetworkChange>,
    pub(crate) shown: ShownNetwork,
}

impl NetworkQuestion {
    /// The collection the send belongs to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Why it is asked.
    pub fn kind(&self) -> QuestionKind {
        self.kind
    }

    /// What changed, or what a first sight sets.
    pub fn changes(&self) -> &[NetworkChange] {
        &self.changes
    }

    /// One line per change.
    pub fn lines(&self) -> Vec<String> {
        self.changes.iter().map(NetworkChange::describe).collect()
    }

    /// The changes joined into one line.
    pub fn summary(&self) -> String {
        NetworkChange::summarize(&self.changes)
    }

    /// What the question shows, for recording it.
    pub fn shown(&self) -> &ShownNetwork {
        &self.shown
    }
}

/// The boxed answer future an ask returns.
pub(crate) type AskFuture = Pin<Box<dyn Future<Output = NetworkAnswer> + Send>>;
type Ask = Arc<dyn Fn(NetworkQuestion) -> AskFuture + Send + Sync>;

/// The check an executor runs before each send: the acknowledgement record at `store` and the
/// callback that asks the person.
#[derive(Clone)]
#[non_exhaustive]
pub struct NetworkCheck {
    pub(crate) store: PathBuf,
    pub(crate) ask: Ask,
}

impl NetworkCheck {
    /// A check against the record at `store`; `ask` answers each question.
    pub fn new<F, Fut>(store: impl Into<PathBuf>, ask: F) -> NetworkCheck
    where
        F: Fn(NetworkQuestion) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = NetworkAnswer> + Send + 'static,
    {
        NetworkCheck {
            store: store.into(),
            ask: Arc::new(move |q| Box::pin(ask(q)) as AskFuture),
        }
    }

    /// The acknowledgement record.
    pub fn store(&self) -> &Path {
        &self.store
    }
}

impl fmt::Debug for NetworkCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NetworkCheck")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// What a send's network settings were resolved from, kept beside the resolved request so the
/// check judges the same in-memory state the send uses: the collection's settings, each text
/// expanded in the send's own scope and in the collection's network scope, and the certificate
/// the send chose. Never serialised; `Debug` prints no value.
#[derive(Clone, PartialEq)]
pub(crate) struct NetworkBasis {
    root: PathBuf,
    network: CollectionNetwork,
    url: String,
    collection: BTreeMap<String, Expanded>,
    baseline: BTreeMap<String, Option<String>>,
    send: BTreeMap<String, Option<String>>,
    send_cert: Option<usize>,
    /// Whether the send's own expansions match `send`; when not, nothing here vouches for it.
    verified: bool,
}

impl Eq for NetworkBasis {}

impl fmt::Debug for NetworkBasis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NetworkBasis { .. }")
    }
}

// Each text of `network` expanded over `vars`, its dynamic tokens kept as written; `None` when
// a variable is undefined.
fn expand_all(network: &CollectionNetwork, vars: &Vars) -> BTreeMap<String, Option<String>> {
    let generator = RefCell::new(Generator::seeded(0, SystemTime::UNIX_EPOCH));
    templates(network)
        .into_iter()
        .filter(|t| t.contains("{{"))
        .map(|t| {
            let value = expand_fixed(t, |t| interpolate_tracked(t, vars, None, &generator)).ok();
            (t.to_owned(), value)
        })
        .collect()
}

// Whether every text the send expanded (`actual`) reads as the snapshot's view of it (`send`)
// does: equal, with each generated token standing for any text, and resolved where the send
// resolved it. Anything else means the snapshot cannot vouch for the send.
fn consistent(send: &BTreeMap<String, Option<String>>, actual: &BTreeMap<String, String>) -> bool {
    send.iter()
        .all(|(template, view)| match (view, actual.get(template)) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(view), Some(used)) => matches_with_tokens(view, used),
        })
}

// `used` matched against `pattern`, each generated token in it a wildcard and every other
// character literal.
fn matches_with_tokens(pattern: &str, used: &str) -> bool {
    let mut parts = Vec::new();
    let mut rest = pattern;
    while let Some((start, _, end)) = next_generated(rest) {
        parts.push(&rest[..start]);
        rest = &rest[end..];
    }
    parts.push(rest);
    if parts.len() == 1 {
        return pattern == used;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    let Some(mut middle) = used.strip_prefix(first).and_then(|u| u.strip_suffix(last)) else {
        return false;
    };
    for part in &parts[1..parts.len() - 1] {
        match middle.find(part) {
            Some(at) => middle = &middle[at + part.len()..],
            None => return false,
        }
    }
    true
}

impl NetworkBasis {
    /// The basis of a send in `ctx` to `url` whose own lookup map is `vars`; `send_cert` is the
    /// index in the collection's certificates of the one the send chose.
    pub(crate) fn new(
        ctx: &VarContext,
        vars: &Vars,
        url: &str,
        send_cert: Option<usize>,
        actual: &BTreeMap<String, String>,
    ) -> NetworkBasis {
        let network = ctx.network().clone();
        let (scope, _) = ctx.network_scope();
        let send = expand_all(&network, vars);
        NetworkBasis {
            root: ctx.root().to_path_buf(),
            collection: scope_expansions(&network, ctx),
            baseline: expand_all(&network, &scope),
            verified: consistent(&send, actual),
            send,
            url: url.to_owned(),
            network,
            send_cert,
        }
    }

    /// The collection's settings as the Network section would show them.
    fn collection_snapshot(&self, salt: &str) -> NetworkSnapshot {
        NetworkSnapshot::from_expansions(&self.network, &self.collection, &self.root, salt)
    }

    // A text as `values` expanded it, secret when it holds any of `sensitive`, so the send's
    // view and the collection's are labelled by one rule.
    fn label(
        text: &str,
        values: &BTreeMap<String, Option<String>>,
        sensitive: &[String],
    ) -> Expanded {
        let value = if !text.contains("{{") {
            text.to_owned()
        } else {
            match values.get(text) {
                Some(Some(v)) => v.clone(),
                _ => return Expanded::Unresolved,
            }
        };
        if sensitive
            .iter()
            .any(|s| !s.is_empty() && value.contains(s.as_str()))
        {
            Expanded::Secret(value)
        } else {
            Expanded::Plain(value)
        }
    }

    // The collection's settings with only certificate `chosen` among the certificates.
    fn with_only(&self, chosen: Option<usize>) -> CollectionNetwork {
        let mut network = self.network.clone();
        network.certificates = chosen
            .and_then(|i| self.network.certificates.get(i).cloned())
            .into_iter()
            .collect();
        network
    }

    /// What the send uses: its proxy and overrides as its own scope expanded them, and the
    /// certificate it chose, hashed from the bytes it will load.
    fn send_snapshot(
        &self,
        salt: &str,
        sensitive: &[String],
        loaded: &Preloaded,
    ) -> NetworkSnapshot {
        let expand = |t: &str| Self::label(t, &self.send, sensitive);
        let content = |i: usize, _: &Expanded| match loaded.0.get(i) {
            Some(Ok(bytes)) => content_hash(bytes),
            _ => "unreadable".to_owned(),
        };
        NetworkSnapshot::build(
            &self.with_only(self.send_cert),
            salt,
            &expand,
            Some(&content),
        )
    }

    /// What the same request would use under the collection's settings alone: the network
    /// scope's values and the certificate they choose for the same URL.
    fn baseline_snapshot(&self, salt: &str, sensitive: &[String]) -> NetworkSnapshot {
        let expand = |t: &str| Self::label(t, &self.baseline, sensitive);
        let chosen = self.network.certificates.iter().position(|cert| {
            cert.enabled
                && match expand(&cert.domain) {
                    Expanded::Plain(d) | Expanded::Secret(d) => domain_matches(&d, &self.url),
                    _ => false,
                }
        });
        let root = &self.root;
        let content = |_: usize, p: &Expanded| path_hash(root, p);
        NetworkSnapshot::build(&self.with_only(chosen), salt, &expand, Some(&content))
    }
}

/// The bytes of the chosen certificate's files, read once by the check so the send loads
/// exactly what was hashed; an entry is the read's error when the file could not be read.
#[derive(Default)]
pub(crate) struct Preloaded(pub(crate) Vec<Result<Vec<u8>, String>>);

impl Preloaded {
    async fn read(network: &RequestNetwork) -> Preloaded {
        let Some(cert) = network.certificate() else {
            return Preloaded::default();
        };
        let paths = match cert.files() {
            CertificateFiles::Pem {
                certificate,
                private_key,
            } => vec![certificate, private_key],
            CertificateFiles::Pkcs12 { file } => vec![file],
        };
        let mut out = Vec::new();
        for path in paths {
            out.push(
                read_regular_async(&path.read)
                    .await
                    .map_err(|e| e.to_string()),
            );
        }
        Preloaded(out)
    }
}

/// What the judgement decided.
enum Verdict {
    Proceed,
    Ask(Box<NetworkQuestion>),
}

// One question at a time per record and collection: a send waiting here judges again once the
// one before it was answered, so parallel sends of one state ask once.
fn serial(store: &Path, root: &Path) -> Arc<tokio::sync::Mutex<()>> {
    type Turns = HashMap<(PathBuf, String), Arc<tokio::sync::Mutex<()>>>;
    static LOCKS: LazyLock<Mutex<Turns>> = LazyLock::new(Mutex::default);
    let mut locks = LOCKS.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (store.to_path_buf(), root_key(root));
    Arc::clone(locks.entry(key).or_default())
}

// The differences between the collection's view of this request and the send's, each marked
// as set by a variable; never empty when the two differ.
fn shadowed(baseline: &NetworkSnapshot, send: &NetworkSnapshot) -> Vec<NetworkChange> {
    let mut changes: Vec<NetworkChange> = NetworkSnapshot::diff(baseline, send)
        .into_iter()
        .map(|c| NetworkChange::SetByVariable(Box::new(c)))
        .collect();
    if changes.is_empty() {
        changes.push(NetworkChange::RequestDiffers);
    }
    changes
}

/// Judges one send against the record at `store`, under the record's lock. A collection that
/// sets nothing is recorded on the spot; a record that cannot be read asks.
fn judge(store: &Path, basis: &NetworkBasis, sensitive: &[String], loaded: &Preloaded) -> Verdict {
    let root = basis.root.as_path();
    let unreadable = |salt: &str| {
        let send = basis.send_snapshot(salt, sensitive, loaded);
        let baseline = basis.baseline_snapshot(salt, sensitive);
        let mut changes = vec![NetworkChange::RecordUnreadable];
        if send.fingerprint() != baseline.fingerprint() {
            changes.extend(shadowed(&baseline, &send));
        }
        Verdict::Ask(Box::new(NetworkQuestion {
            root: root.to_path_buf(),
            kind: QuestionKind::Changed,
            changes,
            shown: ShownNetwork {
                salt: String::new(),
                collection: None,
                send: None,
                judged: None,
            },
        }))
    };
    let judged = with_lock(store, || {
        let fresh = !store.exists();
        let Ok(mut ack) = NetworkAck::load(store) else {
            return Ok(unreadable(""));
        };
        // A new record is written at once, so the salt the question is keyed with is the one
        // the acknowledgement that follows will find.
        if fresh {
            ack.save()?;
        }
        let salt = ack.salt().to_owned();
        let send = basis.send_snapshot(&salt, sensitive, loaded);
        let send_fp = send.fingerprint();
        // The collection's state comes first: an approved send vouches only for the
        // acknowledged settings it was approved under, never for a later change to them.
        let collection = basis.collection_snapshot(&salt);
        let judged = collection.fingerprint();
        let state = ack.check_snapshot(root, &collection);
        if state == AckState::Unchanged && ack.has_send(root, &send_fp) {
            return Ok(Verdict::Proceed);
        }
        let baseline = basis.baseline_snapshot(&salt, sensitive);
        let differs = send_fp != baseline.fingerprint();
        let (kind, mut changes, shown_collection) = match state {
            AckState::Unchanged if !differs => return Ok(Verdict::Proceed),
            AckState::FirstSeen(changes) if !differs && changes.is_empty() => {
                ack.acknowledge_snapshot(root, &collection);
                ack.save()?;
                return Ok(Verdict::Proceed);
            }
            AckState::FirstSeen(changes) if !differs => {
                (QuestionKind::FirstSeen, changes, Some(collection))
            }
            AckState::Unchanged => (QuestionKind::Changed, Vec::new(), None),
            AckState::FirstSeen(changes) | AckState::Changed(changes) => {
                (QuestionKind::Changed, changes, Some(collection))
            }
        };
        if differs {
            changes.extend(shadowed(&baseline, &send));
        }
        Ok(Verdict::Ask(Box::new(NetworkQuestion {
            root: root.to_path_buf(),
            kind,
            changes,
            shown: ShownNetwork {
                salt,
                collection: shown_collection,
                send: Some(send_fp),
                judged: Some(judged),
            },
        })))
    });
    judged.unwrap_or_else(|_| unreadable(""))
}

impl NetworkCheck {
    // A send the snapshot cannot vouch for: asked about every time, never recorded.
    async fn ask_unverified(
        &self,
        basis: &NetworkBasis,
        cancel: &CancelHandle,
        loaded: Preloaded,
    ) -> Result<Preloaded, ExecError> {
        let question = NetworkQuestion {
            root: basis.root.clone(),
            kind: QuestionKind::Changed,
            changes: vec![NetworkChange::Unverifiable],
            shown: ShownNetwork {
                salt: String::new(),
                collection: None,
                send: None,
                judged: None,
            },
        };
        match cancel.run((self.ask)(question)).await {
            None => Err(ExecError::Cancelled),
            Some(NetworkAnswer::Cancel) => Err(ExecError::NetworkDeclined),
            Some(_) => Ok(loaded),
        }
    }

    /// Judges `req` before its send and asks when its settings changed. Returns the chosen
    /// certificate's bytes for the send to load; fails with [`ExecError::NetworkDeclined`] on
    /// Cancel or for a request with no network basis, and [`ExecError::Cancelled`] when
    /// `cancel` fires while asking.
    pub(crate) async fn run(
        &self,
        req: &ResolvedRequest,
        cancel: &CancelHandle,
    ) -> Result<Preloaded, ExecError> {
        let Some(basis) = req.network().basis.clone() else {
            return Err(ExecError::NetworkDeclined);
        };
        let loaded = Preloaded::read(req.network()).await;
        let sensitive: Vec<String> = (req.sensitive_values().iter())
            .chain(req.auth_secrets())
            .cloned()
            .collect();
        if !basis.verified {
            return self.ask_unverified(&basis, cancel, loaded).await;
        }
        let serial = serial(&self.store, &basis.root);
        let _turn = match cancel.run(serial.lock_owned()).await {
            Some(turn) => turn,
            None => return Err(ExecError::Cancelled),
        };
        let question = match judge(&self.store, &basis, &sensitive, &loaded) {
            Verdict::Proceed => return Ok(loaded),
            Verdict::Ask(question) => *question,
        };
        let shown = question.shown.clone();
        let answer = match cancel.run((self.ask)(question)).await {
            Some(answer) => answer,
            None => return Err(ExecError::Cancelled),
        };
        match answer {
            NetworkAnswer::Send => {
                // A record that cannot be read or written leaves the next send to ask again.
                let _ = NetworkAck::update(&self.store, |ack| {
                    ack.acknowledge_shown(&basis.root, &shown);
                });
                Ok(loaded)
            }
            NetworkAnswer::SendOnce => Ok(loaded),
            NetworkAnswer::Cancel => Err(ExecError::NetworkDeclined),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Collection, ContextOptions};

    fn basis(bypass: &str, actual: &[(&str, &str)]) -> NetworkBasis {
        let dir = tempfile::TempDir::new().unwrap();
        let root = format!(
            "opencollection: \"1.0.0\"\ninfo:\n  name: t\nconfig:\n  proxy:\n    inherit: false\n    config:\n      protocol: http\n      hostname: 127.0.0.1\n      port: \"9\"\n      bypassProxy: \"{bypass}\"\nrequest:\n  variables:\n    - name: bp\n      value: a.test\n"
        );
        std::fs::write(dir.path().join("opencollection.yml"), root).unwrap();
        let c = Collection::open(dir.path()).unwrap();
        let ctx = VarContext::new(&c, ContextOptions::new()).unwrap();
        let (vars, _) = ctx.network_scope();
        let actual = actual
            .iter()
            .map(|(t, v)| ((*t).to_owned(), (*v).to_owned()))
            .collect();
        NetworkBasis::new(&ctx, &vars, "http://x.test/", None, &actual)
    }

    // The snapshot vouches only for what the send actually used.
    #[test]
    fn a_send_value_that_differs_from_the_snapshot_is_unverified() {
        assert!(basis("{{bp}}", &[("{{bp}}", "a.test")]).verified);
        assert!(!basis("{{bp}}", &[("{{bp}}", "127.0.0.1")]).verified);
        assert!(!basis("{{nope}}", &[("{{nope}}", "127.0.0.1")]).verified);
    }

    // A generated token matches whatever the send generated for it, and nothing more.
    #[test]
    fn a_generated_token_is_a_wildcard_between_literal_text() {
        assert!(basis("{{$randomInt}}", &[("{{$randomInt}}", "417")]).verified);
        let mixed = "x{{$randomInt}}.{{bp}}";
        assert!(basis(mixed, &[(mixed, "x417.a.test")]).verified);
        assert!(!basis(mixed, &[(mixed, "y417.a.test")]).verified);
        assert!(!basis(mixed, &[(mixed, "x417.b.test")]).verified);
    }
}
