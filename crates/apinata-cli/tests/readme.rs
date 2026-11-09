//! The README's and GUIDE's claims checked against the real binary and the house rules.

use crate::common;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn readme() -> String {
    std::fs::read_to_string(root().join("README.md")).unwrap()
}

fn guide() -> String {
    std::fs::read_to_string(root().join("GUIDE.md")).unwrap()
}

/// The user-facing docs whose commands are checked, by file name.
fn docs() -> [(&'static str, String); 2] {
    [("README.md", readme()), ("GUIDE.md", guide())]
}

/// Lines inside fenced code blocks, without a leading `$ ` prompt.
fn code_lines(markdown: &str) -> Vec<String> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
        } else if inside {
            lines.push(line.trim().trim_start_matches("$ ").to_owned());
        }
    }
    lines
}

/// The stdout of `apinata ARGS --help`.
fn help(args: &[&str]) -> String {
    let out = common::apinata_std()
        .args(args)
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success(), "apinata {args:?} --help failed");
    String::from_utf8(out.stdout).unwrap()
}

/// The subcommand names listed under `Commands:` in `apinata ARGS --help`.
fn subcommands(args: &[&str]) -> Vec<String> {
    help(args)
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .filter(|name| name != "help")
        .collect()
}

// Each `apinata` line in either doc must name a real subcommand, a real action under it when it
// has actions, and only flags that the innermost command's help lists.
#[test]
fn every_apinata_command_in_the_docs_exists() {
    let top = subcommands(&[]);
    for (file, text) in docs() {
        check_commands(file, &text, &top);
    }
}

fn check_commands(file: &str, text: &str, top: &[String]) {
    let mut seen = 0;
    for line in code_lines(text) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() != Some(&"apinata") || words.len() < 2 || words[1].starts_with('-') {
            continue;
        }
        seen += 1;
        assert!(
            top.contains(&words[1].to_owned()),
            "{file} uses `apinata {}`; the CLI has {top:?}",
            words[1]
        );
        let mut command = vec![words[1]];
        let nested = subcommands(&command);
        if !nested.is_empty() {
            let action = words.get(2).copied().unwrap_or("");
            assert!(
                nested.contains(&action.to_owned()),
                "{file} uses `{line}`; `apinata {}` has {nested:?}",
                words[1]
            );
            command.push(action);
        }
        let options = help(&command);
        for flag in words.iter().filter(|w| w.starts_with("--")) {
            let name = flag.split('=').next().unwrap();
            assert!(
                // An optional value prints as `--name[=<V>]`; long help puts a bare flag alone on its line.
                options.contains(&format!("{name} "))
                    || options.contains(&format!("{name}["))
                    || options.contains(&format!("{name}\n")),
                "{file} uses `{name}` in `{line}`, which `apinata {} --help` lacks",
                command.join(" ")
            );
        }
    }
    assert!(seen >= 6, "{file} shows only {seen} apinata commands");
}

#[test]
fn every_make_target_in_the_docs_exists() {
    let makefile = std::fs::read_to_string(root().join("Makefile")).unwrap();
    for (file, text) in docs() {
        for line in code_lines(&text) {
            if let Some(targets) = line.strip_prefix("make ") {
                for t in targets.split_whitespace() {
                    assert!(
                        makefile.contains(&format!("\n{t}:")),
                        "{file} uses `make {t}`, which the Makefile lacks"
                    );
                }
            }
        }
    }
}

/// The anchor GitHub gives a heading: lower case, punctuation dropped, spaces as dashes.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn anchors(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|l| l.strip_prefix('#'))
        .map(|l| slug(l.trim_start_matches('#')))
        .collect()
}

// Every `#anchor` link between and within the two docs must land on a heading, so moving a
// section between them cannot leave a dead link behind.
#[test]
fn every_doc_link_lands_on_a_heading() {
    let docs = docs();
    let mut checked = 0;
    for (file, text) in &docs {
        for (at, _) in text.match_indices("](") {
            let target = &text[at + 2..];
            let target = &target[..target.find(')').unwrap()];
            let Some((path, anchor)) = target.split_once('#') else {
                continue;
            };
            let into = if path.is_empty() { *file } else { path };
            let (_, body) = docs
                .iter()
                .find(|(name, _)| *name == into)
                .unwrap_or_else(|| panic!("{file} links {target}, which is not a checked doc"));
            assert!(
                anchors(body).contains(&anchor.to_owned()),
                "{file} links {target}, which has no such heading"
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} anchor links checked");
}

#[test]
fn the_readme_links_the_guide() {
    assert!(readme().contains("](GUIDE.md)"));
}

#[test]
fn user_facing_docs_never_name_another_client() {
    let mut files = vec![
        root().join("README.md"),
        root().join("GUIDE.md"),
        root().join("RELEASING.md"),
    ];
    for entry in std::fs::read_dir(root().join(".github/workflows")).unwrap() {
        files.push(entry.unwrap().path());
    }
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap().to_lowercase();
        for name in ["bruno", "zaku"] {
            assert!(!text.contains(name), "{} names {name}", path.display());
        }
        // "zed" is checked as a whole word, since words such as "authorized" contain it.
        let mut words = text.split(|c: char| !c.is_ascii_alphanumeric());
        assert!(!words.any(|w| w == "zed"), "{} names zed", path.display());
    }
}

#[test]
fn readme_states_the_licence_and_links_both_texts() {
    let text = readme();
    assert!(text.contains("MIT OR Apache-2.0"));
    for file in ["LICENSE-MIT", "LICENSE-APACHE"] {
        assert!(
            text.contains(&format!("]({file})")),
            "README does not link {file}"
        );
        assert!(root().join(file).is_file(), "{file} is missing");
    }
}

#[test]
fn the_guide_documents_the_stream_limits_and_help_lists_them() {
    let guide = guide();
    let run_help = help(&["run"]);
    for flag in [
        "--stream-max-time",
        "--stream-max-events",
        "--max-body-bytes",
        "--timeout",
    ] {
        assert!(guide.contains(flag), "GUIDE lacks {flag}");
        assert!(run_help.contains(flag), "`apinata run --help` lacks {flag}");
    }
}

/// The codes a command can exit with according to the GUIDE's "Exit codes" table, by command.
fn table_codes() -> std::collections::BTreeMap<String, Vec<u8>> {
    let text = guide();
    let table = text
        .split("### Exit codes")
        .nth(1)
        .expect("an Exit codes section")
        .lines()
        .filter(|l| l.starts_with("| `"));
    table
        .map(|row| {
            let cells: Vec<&str> = row.trim_matches('|').split('|').map(str::trim).collect();
            let name = cells[0].trim_matches('`').to_owned();
            let codes = (0..4u8)
                .filter(|i| cells[1 + usize::from(*i)] != "-")
                .collect();
            (name, codes)
        })
        .collect()
}

/// The codes in a command's `Exit codes:` help block, leaving out 130 (an interrupt).
fn help_codes(name: &str) -> Vec<u8> {
    let text = help(&[name]);
    text.lines()
        .skip_while(|l| !l.starts_with("Exit codes:"))
        .skip(1)
        .map_while(|l| {
            l.strip_prefix("  ")?
                .split_whitespace()
                .next()?
                .parse::<u8>()
                .ok()
        })
        .filter(|code| *code < 4)
        .collect()
}

// The table and every command's `--help` must name the same exit codes.
#[test]
fn the_exit_code_table_matches_every_commands_help() {
    let table = table_codes();
    let commands = subcommands(&[]);
    for name in &commands {
        if name == "run" || table.contains_key(name) {
            continue;
        }
        panic!("`{name}` has no row in the GUIDE exit code table");
    }
    for (name, codes) in &table {
        assert_eq!(
            &help_codes(name),
            codes,
            "`apinata {name} --help` vs the GUIDE row"
        );
    }
    assert!(table.len() >= commands.len() - 1, "{table:?}");
}

// The GUIDE names the entries that leave a folder "empty" for a new collection; the list must
// be the one the core rule uses.
#[test]
fn the_guide_lists_the_entries_that_count_as_an_empty_folder() {
    let rule = std::fs::read_to_string(root().join("crates/apinata-core/src/collection/create.rs"))
        .unwrap();
    let names = [".git", ".gitignore", ".gitattributes", ".DS_Store"];
    let quoted = names.map(|n| format!("\"{n}\"")).join(", ");
    assert!(rule.contains(&format!("[{quoted}]")), "core's list changed");
    let text = guide().split_whitespace().collect::<Vec<_>>().join(" ");
    let phrase = "is empty, or holds only";
    let at = text
        .find(phrase)
        .expect("the empty-folder rule in the GUIDE");
    let sentence = &text[at..text.len().min(at + 200)];
    for name in names {
        assert!(
            sentence.contains(&format!("`{name}`")),
            "{name}: {sentence}"
        );
    }
}
