//! A button, menu entry or tooltip for a registered command takes its text from the registry.

use std::path::Path;

use apinata_app::commands::registry;

// Calls whose first string argument is text a person reads or a screen reader speaks.
const TEXT_CALLS: &[&str] = &[
    ".tooltip(",
    ".accessibility_label(",
    ".label(",
    "tooltip_with_action(",
    "menu_item(",
    "MenuLine::new(",
    "PopupMenuItem::new(",
];

// Text that matches a command's label but names something else, with why.
const NOT_THE_COMMAND: &[(&str, &str)] = &[
    // Renames an environment, not the selected sidebar row.
    ("env_editor.rs", "Rename"),
    ("global_env_editor.rs", "Rename"),
    // Saves the dialog's own form or the editor's file, not the active tab.
    ("cookie_jar.rs", "Save"),
    ("save_example.rs", "Save"),
    ("save_draft.rs", "Save"),
    ("env_editor.rs", "Save"),
    ("env_editor.rs", "Discard changes"),
    // Reveals the file an export just wrote, not the selected sidebar row.
    ("export.rs", "Show in file manager"),
    // The history palette's group heading, inside the palette the command opens.
    ("history_palette.rs", "History"),
    // Exports one logged exchange from the console, not the active tab's response.
    ("console_dock.rs", "Export as HAR…"),
];

// Wordings a command's label has had in other places, which count as naming it.
const DRIFTED: &[&str] = &["Export HAR", "Export as HAR"];

// Every source file, its comment lines dropped and its whitespace folded, so a call split
// over several lines reads as one.
fn sources() -> Vec<(String, String)> {
    fn walk(src: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("src is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(src, &path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("source is utf-8");
                let code: Vec<&str> = (text.lines())
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect();
                let folded = code
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let rel = path.strip_prefix(src).expect("under src");
                let name = rel.to_string_lossy().replace('\\', "/");
                out.push((name, folded));
            }
        }
    }
    let mut out = Vec::new();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    walk(&src, &src, &mut out);
    out
}

// The string literal starting at `at` (just after its opening quote), if one closes.
fn literal(code: &str, at: usize) -> Option<&str> {
    code[at..].find('"').map(|end| &code[at..at + end])
}

// The literal texts `code` hands to a text call or an icon button, or pairs with an action
// as an empty state's or a menu's `(text, action)` row does.
fn shown_texts(code: &str) -> Vec<&str> {
    let mut texts = Vec::new();
    for call in TEXT_CALLS {
        for (ix, _) in code.match_indices(call) {
            let at = ix + call.len();
            if code[at..].starts_with('"') {
                texts.extend(literal(code, at + 1));
            }
        }
    }
    for (ix, _) in code.match_indices("icon_button(") {
        let args = &code[ix..];
        // The arguments end at the context, the last one, however the call is formatted.
        let end = (args.find(" cx)").into_iter())
            .chain(args.find(" cx, )"))
            .min()
            .unwrap_or(args.len().min(200));
        let args = &args[..end];
        texts.extend(args.split('"').skip(1).step_by(2));
    }
    for paired in ["\", &", "\", Box::new("] {
        for (ix, _) in code.match_indices(paired) {
            if let Some(open) = code[..ix].rfind('"') {
                texts.push(&code[open + 1..ix]);
            }
        }
    }
    texts
}

#[test]
fn ui_text_for_a_command_comes_from_the_registry() {
    let mut labels: Vec<&str> = registry().iter().map(|c| c.label).collect();
    labels.extend(DRIFTED);
    let mut hard_coded = Vec::new();
    for (file, code) in sources() {
        if file == "commands.rs" {
            continue;
        }
        for text in shown_texts(&code) {
            let names_a_command = labels
                .iter()
                .any(|l| *l == text || l.trim_end_matches('…') == text.trim_end_matches('…'));
            if names_a_command && !NOT_THE_COMMAND.contains(&(file.as_str(), text)) {
                hard_coded.push(format!("{file}: {text:?}"));
            }
        }
    }
    assert!(
        hard_coded.is_empty(),
        "use commands::label for {hard_coded:#?}"
    );
}

#[test]
fn the_environment_selector_entries_read_as_their_commands() {
    let label = apinata_app::commands::label;
    assert_eq!(
        apinata_app::env_picker::MANAGE_ENVIRONMENTS,
        label("environment.open")
    );
    assert_eq!(
        apinata_app::env_picker::IMPORT_ENVIRONMENT,
        label("environment.import")
    );
}
