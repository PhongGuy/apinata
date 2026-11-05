//! RSA-PSS keys in `require("crypto")`. `fixtures/crypto-keys/pss.js` runs here and under Node 24
//! (`gen.js` records `pss-node.txt`); `pss-keys.json` holds the keys and Node's signatures.

use crate::script_crypto::run_crypto;

const VECTORS: &str = include_str!("fixtures/crypto-keys/vectors.json");
const PSS_KEYS: &str = include_str!("fixtures/crypto-keys/pss-keys.json");
const CASES: &str = include_str!("fixtures/crypto-keys/pss.js");
const NODE_ANSWERS: &str = include_str!("fixtures/crypto-keys/pss-node.txt");

/// A corpus line without OpenSSL's error code and prefix: the shim gives OpenSSL's message alone.
fn comparable(line: &str) -> String {
    let Some((head, message)) = line.split_once(" | ") else {
        return line.to_string();
    };
    match message.strip_prefix("error:") {
        Some(openssl) => {
            let (name, _) = head.rsplit_once(' ').expect("an error line has a code");
            let text = openssl.rsplit_once("::").map_or(openssl, |(_, text)| text);
            format!("{name} undefined | {text}")
        }
        None => line.to_string(),
    }
}

/// Every line of `pss.js` as Node 24 answered it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rsa_pss_keys_get_nodes_answers() {
    let code = format!(
        "const K = {VECTORS};\nconst P = {PSS_KEYS};\nconst PW = \"correct horse\";\n{CASES}"
    );
    let ours: Vec<String> = run_crypto(&code)
        .await
        .iter()
        .map(|line| comparable(line.strip_prefix("log:").unwrap_or(line)))
        .collect();
    let node: Vec<String> = NODE_ANSWERS.lines().map(comparable).collect();
    let wrong: Vec<String> = ours
        .iter()
        .zip(&node)
        .filter(|(ours, node)| ours != node)
        .map(|(ours, node)| format!("ours {ours}\nnode {node}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "{} of {} differ:\n{}",
        wrong.len(),
        node.len(),
        wrong.join("\n")
    );
    assert_eq!(ours.len(), node.len(), "{:?}", ours.last());
}
