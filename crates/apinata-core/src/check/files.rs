//! The file pass: body files and client certificate files must exist, inside the root unless
//! the context reads files anywhere.

use super::report::{Finding, FindingKind, Severity};
use crate::CertificateFiles;
use crate::file_access::ReadFailure;
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
    let scope = resolved.file_scope();
    for file in resolved.files() {
        let written = file.written.display();
        match scope.probe(&file.given) {
            Ok(()) => {}
            Err(ReadFailure::Outside) => {
                missing(format!("file `{written}` is outside the collection"));
            }
            Err(ReadFailure::Io(_)) => missing(format!("file `{written}` does not exist")),
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
        let problem = match path.scope.probe(&path.given) {
            Ok(()) => continue,
            Err(ReadFailure::Outside) => "is outside the collection",
            Err(ReadFailure::Io(_)) => "does not exist",
        };
        missing(format!(
            "client certificate file `{}` {problem}",
            path.written
        ));
    }
}
