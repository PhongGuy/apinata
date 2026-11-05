//! How jsonwebtoken reads keys. The case files under `fixtures/jwt` run here and, through
//! `record.js`, under Node 24 and jsonwebtoken 9.0.3; each answer here must equal Node's line.

use crate::script_crypto::run_crypto;

const KEYS: &str = include_str!("fixtures/jwt/keys.json");
const PRELUDE: &str = include_str!("fixtures/jwt/cases-prelude.js");
const PLAIN: &str = include_str!("fixtures/jwt/plain.js");
const PLAIN_NODE: &str = include_str!("fixtures/jwt/plain-node.txt");
const FORMS: &str = include_str!("fixtures/jwt/forms.js");
const FORMS_NODE: &str = include_str!("fixtures/jwt/forms-node.txt");
const EDGE_KEYS: &str = include_str!("fixtures/jwt/edge.json");
const EDGES: &str = include_str!("fixtures/jwt/edges.js");
const EDGES_NODE: &str = include_str!("fixtures/jwt/edges-node.txt");
const CLOSED: &str = include_str!("fixtures/jwt/closed.js");
const CLOSED_NODE: &str = include_str!("fixtures/jwt/closed-node.txt");
const STRICT: &str = include_str!("fixtures/jwt/strict.js");
const STRICT_NODE: &str = include_str!("fixtures/jwt/strict-node.txt");
/// Each line a tag, a tab and the shim's answer, for lines Node answers another way.
const REFUSED: &str = include_str!("fixtures/jwt/refused.tsv");

/// The refusal for text with a PEM marker that does not open.
const NOT_A_KEY: &str = "Error Unsupported key: text with a PEM marker is never a secret, and this is no key that opens here (supported: RSA, ECDSA P-256, P-384 or P-521, and Ed25519)";

const NOT_MATERIAL: &str = "Error secretOrPrivateKey is not valid key material";

/// The case file's lines. The block keeps its names apart from the ones `run_crypto` declares.
async fn run_cases(cases: &str) -> Vec<String> {
    let code = format!("const K = {KEYS};\nconst E = {EDGE_KEYS};\n{{\n{PRELUDE}\n{cases}\n}}");
    let lines = run_crypto(&code).await;
    lines
        .iter()
        .map(|l| l.strip_prefix("log:").unwrap_or(l).to_owned())
        .collect()
}

/// Asserts every line equals Node's, after `adjust` swaps the lines where the shim differs by design.
fn assert_matches(shim: &[String], node: &str, adjust: impl Fn(&str) -> Option<String>) {
    let node: Vec<String> = node
        .lines()
        .map(|l| adjust(l).unwrap_or_else(|| l.to_owned()))
        .collect();
    for (ours, theirs) in shim.iter().zip(&node) {
        assert_eq!(ours, theirs);
    }
    assert_eq!(shim.len(), node.len());
}

/// The allowlisted answer for a Node line, which may only be a refusal: never a token or a key
/// Node did not give.
fn refused_line(line: &str) -> Option<String> {
    let (tag, answer) = line.split_once(": ")?;
    let (_, ours) = REFUSED
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(listed, _)| *listed == tag)?;
    let (node, shim): (Vec<&str>, Vec<&str>) =
        (answer.split(' ').collect(), ours.split(' ').collect());
    let opened_only = tag.ends_with(" crypto")
        && node.len() == shim.len()
        && node
            .iter()
            .zip(&shim)
            .all(|(node, shim)| *shim == "unreadable" || shim == node);
    let refused = ours.starts_with("Error Unsupported key: ") || ours == NOT_MATERIAL;
    assert!(opened_only || refused, "{tag}: {ours}");
    Some(format!("{tag}: {ours}"))
}

/// Node's `foreign` lines open a key no crate holds, or a public key on sign; both stop here.
/// RSA-PSS keys open here as in Node, so their other lines are Node's.
fn foreign_line(line: &str) -> Option<String> {
    let (tag, _) = line.split_once(": ")?;
    let opens = tag.starts_with("foreign PSS") && !tag.ends_with("_PUB sign HS256");
    (tag.starts_with("foreign ") && !opens).then(|| format!("{tag}: {NOT_A_KEY}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_keys_are_typed_as_node_types_them() {
    let adjust = |line: &str| refused_line(line).or_else(|| foreign_line(line));
    assert_matches(&run_cases(PLAIN).await, PLAIN_NODE, adjust);
}

/// Where the shim stops and Node goes on: crypto's RSA size range and PSS room, and ECDSA output
/// Node pads or cuts to another curve's size once the type check is off.
fn forms_line(line: &str) -> Option<String> {
    let (tag, answer) = line.split_once(": ")?;
    let refused = |answer: &str| Some(format!("{tag}: {answer}"));
    let small = (tag.starts_with("size 512 ") && tag.ends_with(" allowed"))
        || tag.starts_with("size verify 4 ")
        || tag.starts_with("size verify 7 ");
    if small {
        return refused("Error Unsupported key: a 512-bit RSA key (supported: 1024 to 8192 bits)");
    }
    if answer.ends_with("data too large for key size") {
        return refused(
            "Error Unsupported key: RSA-PSS with sha512 and saltLength 64 does not fit a 1024-bit RSA key (data too large for key size)",
        );
    }
    let curve_mismatch = ["EC256 ES384", "EC384 ES256", "EC521 ES256", "EC521 ES384"]
        .iter()
        .find(|pair| tag.starts_with(&format!("loose {pair}")))?;
    let alg = curve_mismatch.split_once(' ')?.1;
    refused(&format!("Error the key cannot be used with {alg}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_forms_and_the_size_rule_follow_node() {
    let adjust = |line: &str| refused_line(line).or_else(|| forms_line(line));
    assert_matches(&run_cases(FORMS).await, FORMS_NODE, adjust);
}

/// `EDGES`'s lines on one side of the split between signatures and PEM files.
async fn edge_lines(tampered: bool) -> (Vec<String>, String) {
    let shim = run_cases(EDGES).await;
    let keep = |line: &str| line.starts_with("tamper ") == tampered;
    let shim = shim.into_iter().filter(|l| keep(l)).collect();
    let node: Vec<&str> = EDGES_NODE.lines().filter(|l| keep(l)).collect();
    (shim, node.join("\n"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_signature_stays_refused_like_node() {
    let (shim, node) = edge_lines(true).await;
    assert_matches(&shim, &node, |_| None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_block_that_is_the_key_is_the_one_node_picks() {
    let (shim, node) = edge_lines(false).await;
    let adjust = |line: &str| refused_line(line).or_else(|| foreign_line(line));
    assert_matches(&shim, &node, adjust);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_with_a_pem_marker_is_a_key_or_a_refusal() {
    assert_matches(&run_cases(CLOSED).await, CLOSED_NODE, refused_line);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reader_opens_clean_files_and_refuses_the_rest() {
    assert_matches(&run_cases(STRICT).await, STRICT_NODE, refused_line);
    let refused = REFUSED.lines().map(|l| l.split('\t').next().unwrap_or(l));
    assert!(
        !refused
            .into_iter()
            .any(|t| t.starts_with("strict ordinary"))
    );
}

#[test]
fn every_refused_row_names_a_recorded_line() {
    let recorded = [PLAIN_NODE, FORMS_NODE, EDGES_NODE, CLOSED_NODE, STRICT_NODE];
    let tags: std::collections::HashSet<&str> = recorded
        .iter()
        .flat_map(|file| file.lines())
        .filter_map(|line| line.split_once(": ").map(|(tag, _)| tag))
        .collect();
    for row in REFUSED.lines() {
        let tag = row.split_once('\t').map(|(tag, _)| tag);
        assert!(tag.is_some_and(|tag| tags.contains(tag)), "{row}");
    }
}

/// The same paths with sentinel text in every position a key could leak from.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn key_text_never_reaches_an_error() {
    let code = r#"const body = "U0VOVElORUxLRVk=";
const broken = (label) => `-----BEGIN ${label}-----\n${body}\n-----END ${label}-----\n`;
const keys = [broken("PRIVATE KEY"), broken("PUBLIC KEY"), broken("EC PRIVATE KEY"), broken("RSA PRIVATE KEY"), broken("CERTIFICATE"), K.PSS, K.K256_SEC1, "SENTINELKEY"];
const seen = [];
const note = (f) => { try { f(); } catch (e) { seen.push(`${e.name} ${e.message} ${e.stack}`); } };
for (const key of keys) for (const form of [key, Buffer.from(key), { key }]) {
  for (const algorithm of ["HS256", "RS256", "PS256", "ES256"]) {
    note(() => jwt.sign({}, form, { algorithm }));
    note(() => jwt.verify(K.tokens[algorithm], form, { algorithms: [algorithm] }));
  }
}
console.log("failed", seen.length > 20);
console.log(seen.join("\n"));"#;
    let lines = run_cases(code).await;
    assert_eq!(lines.first().map(String::as_str), Some("failed true"));
    for line in &lines {
        for needle in ["SENTINEL", "U0VOVEl", "-----BEGIN"] {
            assert!(!line.contains(needle), "{line}");
        }
    }
}
