//! The file pass: body files and client certificate files must exist, inside the root.

use std::path::Path;

use super::report::{Finding, FindingKind, Severity};
use crate::CertificateFiles;
use crate::execute::escapes_root;
use crate::resolve::ResolvedRequest;

/// Check every file one resolved request names. Only metadata is read, never contents.
pub(super) fn check_files(
    item: &str,
    env: Option<&str>,
    resolved: &ResolvedRequest,
    out: &mut Vec<Finding>,
) {
    let mut missing = |message: String| {
        out.push(Finding::new(
            item,
            env,
            Severity::Error,
            FindingKind::MissingFile,
            message,
        ));
    };
    for file in resolved.files() {
        let written = file.written.display();
        if escapes_root(&file.given) {
            missing(format!("file `{written}` is outside the collection"));
        } else if !exists(&file.path) {
            missing(format!("file `{written}` does not exist"));
        }
    }
    let Some(cert) = resolved.network().certificate() else {
        return;
    };
    let paths: Vec<_> = match cert.files() {
        CertificateFiles::Pem {
            certificate,
            private_key,
        } => vec![certificate, private_key],
        CertificateFiles::Pkcs12 { file } => vec![file],
    };
    for path in paths {
        if !exists(&path.read) {
            missing(format!(
                "client certificate file `{}` does not exist",
                path.written
            ));
        }
    }
}

fn exists(path: &Path) -> bool {
    std::fs::metadata(path).is_ok()
}
