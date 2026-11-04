//! Static keys from the AWS shared credentials and config files, for SigV4 auth that names a
//! profile. Only plain key profiles are read; role, SSO and process profiles are not.

use std::collections::HashMap;
use std::path::PathBuf;

/// Where the AWS shared credentials and config files are read from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AwsSharedFiles {
    credentials: PathBuf,
    config: PathBuf,
}

impl AwsSharedFiles {
    /// Read the credentials file at `credentials` and the config file at `config`.
    pub fn new(credentials: impl Into<PathBuf>, config: impl Into<PathBuf>) -> Self {
        AwsSharedFiles {
            credentials: credentials.into(),
            config: config.into(),
        }
    }

    /// `$AWS_SHARED_CREDENTIALS_FILE` / `$AWS_CONFIG_FILE`, else `~/.aws/credentials` and `~/.aws/config`.
    pub fn from_env() -> Self {
        let aws = dirs::home_dir().unwrap_or_default().join(".aws");
        let path = |var: &str, file: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map_or_else(|| aws.join(file), PathBuf::from)
        };
        AwsSharedFiles::new(
            path("AWS_SHARED_CREDENTIALS_FILE", "credentials"),
            path("AWS_CONFIG_FILE", "config"),
        )
    }
}

/// Static keys of `profile`: `Some((access_key_id, secret_access_key, session_token))` when the
/// merged profile (credentials file `[name]` over config file `[profile name]`, `[default]` for
/// `default`) has both keys and no `role_arn`, `source_profile`, `credential_process` or
/// `sso_*` key; the token is empty when absent. Unreadable files read as empty.
pub(crate) fn profile_keys(
    files: &AwsSharedFiles,
    profile: &str,
) -> Option<(String, String, String)> {
    let config_section = match profile {
        "default" => "default".to_owned(),
        name => format!("profile {name}"),
    };
    // Credentials keys win key by key, so they are laid over the config's.
    let mut keys = section(&read(&files.config), &config_section);
    keys.extend(section(&read(&files.credentials), profile));
    let unsupported = ["role_arn", "source_profile", "credential_process"];
    if keys
        .keys()
        .any(|k| unsupported.contains(&k.as_str()) || k.starts_with("sso_"))
    {
        return None;
    }
    let key = |name: &str| keys.get(name).filter(|v| !v.is_empty()).cloned();
    let access = key("aws_access_key_id")?;
    let secret = key("aws_secret_access_key")?;
    Some((access, secret, key("aws_session_token").unwrap_or_default()))
}

fn read(path: &std::path::Path) -> String {
    crate::document::read_regular_text(path).unwrap_or_default()
}

/// The `key = value` pairs of every `[name]` section in INI `text`; later keys win, and lines
/// starting with `#` or `;` are comments.
fn section(text: &str, name: &str) -> HashMap<String, String> {
    let mut keys = HashMap::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let header: Vec<&str> = header.split_whitespace().collect();
            inside = header.join(" ") == name;
        } else if inside && let Some((key, value)) = line.split_once('=') {
            keys.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_pipe_reads_as_an_empty_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let (pipe, conf) = (dir.path().join("credentials"), dir.path().join("config"));
        crate::hang_guard::mkfifo(&pipe);
        std::fs::write(
            &conf,
            "[default]\naws_access_key_id = A\naws_secret_access_key = B\n",
        )
        .unwrap();
        let files = AwsSharedFiles::new(&pipe, &conf);
        let keys = crate::hang_guard::off_the_pipe(&pipe, move || profile_keys(&files, "default"));
        assert_eq!(keys, Some(("A".into(), "B".into(), String::new())));
    }

    #[test]
    fn credentials_win_key_by_key_over_config_and_default_has_no_prefix() {
        let dir = tempfile::TempDir::new().unwrap();
        let (cred, conf) = (dir.path().join("credentials"), dir.path().join("config"));
        std::fs::write(&cred, "[default]\naws_access_key_id=CRED\n ; note\n").unwrap();
        std::fs::write(&conf, "[default]\naws_access_key_id = CONF\naws_secret_access_key = S\n[profile default]\naws_secret_access_key = WRONG\n").unwrap();
        let files = AwsSharedFiles::new(&cred, &conf);
        assert_eq!(
            profile_keys(&files, "default"),
            Some(("CRED".into(), "S".into(), String::new()))
        );
    }

    #[test]
    fn missing_files_and_role_profiles_give_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let files = AwsSharedFiles::new(dir.path().join("nope"), dir.path().join("nope2"));
        assert_eq!(profile_keys(&files, "dev"), None);
        let cred = dir.path().join("credentials");
        std::fs::write(
            &cred,
            "[dev]\naws_access_key_id = A\naws_secret_access_key = B\nrole_arn = arn:x\n",
        )
        .unwrap();
        assert_eq!(
            profile_keys(&AwsSharedFiles::new(&cred, dir.path().join("nope")), "dev"),
            None
        );
    }
}
