//! `apinata completions <shell>`: scripts for every supported shell, printed to stdout.

use crate::common;

fn completions(shell: &str) -> std::process::Output {
    common::apinata_std()
        .args(["completions", shell])
        .output()
        .unwrap()
}

#[test]
fn every_shell_prints_a_script_naming_the_subcommands() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = completions(shell);
        assert_eq!(out.status.code(), Some(0), "{shell}");
        let script = String::from_utf8(out.stdout).unwrap();
        assert!(!script.is_empty(), "{shell} printed nothing");
        for word in ["run", "gen", "ls", "env"] {
            assert!(script.contains(word), "{shell} script lacks `{word}`");
        }
    }
}

#[test]
fn zsh_script_offers_ls_and_env() {
    let script = String::from_utf8(completions("zsh").stdout).unwrap();
    assert!(script.contains("'ls:"), "no ls subcommand");
    assert!(script.contains("'env:"), "no env subcommand");
}

#[test]
fn zsh_script_offers_the_env_var_flag() {
    let script = String::from_utf8(completions("zsh").stdout).unwrap();
    assert!(script.contains("--env-var"));
}

#[test]
fn unknown_shell_is_a_usage_error_listing_the_valid_ones() {
    let out = completions("tcsh");
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    let err = String::from_utf8(out.stderr).unwrap();
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        assert!(err.contains(shell), "stderr lacks `{shell}`: {err}");
    }
}
