//! The keymap.json model: canonical chords, each command's effective keys, diagnostics for
//! unknown and invalid entries, and the pure edits the Keyboard section makes (assign with
//! Replace or Swap, reset, reset all). Nothing here touches the window or the disk.

use std::collections::{BTreeMap, BTreeSet};

use gpui_kit::{KeyBinding, KeyBindingContextPredicate, Keystroke, Modifiers};

use crate::commands::{Command, Place, Scope, places, registry};

/// keymap.json: command id to keystroke strings; absent means defaults, [] means unbound.
pub type Overrides = BTreeMap<String, Vec<String>>;

const MODIFIER_KEYS: [&str; 5] = ["control", "shift", "alt", "platform", "function"];

// GPUI's own rule for a key without case: the platform reports the shifted symbol itself
// (`{`, not `shift-[`), so shift on such a key never reaches a binding.
fn caseless_char(key: &str) -> bool {
    key.chars().count() == 1 && key.to_lowercase() == key.to_uppercase()
}

/// One parsed keystroke in canonical form: no key_char, no shift on a one-character key
/// without case. Equality is on the parsed keystroke, so `ctrl-k` == `secondary-k` on Linux.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord(Keystroke);

impl Chord {
    /// File syntax. Refuses: unparsable text, an empty key, a sequence (whitespace),
    /// a `->` key_char, a modifier-only key, and shift with a one-character key without case.
    pub fn parse(text: &str) -> Option<Chord> {
        if text.chars().any(char::is_whitespace) {
            return None;
        }
        let k = Keystroke::parse(text).ok()?;
        let refused = k.key.is_empty()
            || k.key_char.is_some()
            || MODIFIER_KEYS.contains(&k.key.as_str())
            || (k.modifiers.shift && caseless_char(&k.key));
        (!refused).then_some(Chord(k))
    }

    /// A recorded keystroke; None for a modifier-only key (control, shift, alt, platform,
    /// function). Drops key_char and drops shift on a one-character key without case.
    pub fn from_keystroke(k: &Keystroke) -> Option<Chord> {
        if k.key.is_empty() || MODIFIER_KEYS.contains(&k.key.as_str()) {
            return None;
        }
        let mut modifiers = k.modifiers;
        if caseless_char(&k.key) {
            modifiers.shift = false;
        }
        Some(Chord(Keystroke {
            modifiers,
            key: k.key.clone(),
            key_char: None,
        }))
    }

    /// The parsed keystroke.
    pub fn keystroke(&self) -> &Keystroke {
        &self.0
    }

    /// `[secondary-][ctrl-][alt-][shift-][cmd-][fn-]key`, the platform's secondary written as `secondary`.
    pub fn to_file_string(&self) -> String {
        let m = self.0.modifiers;
        let mac = cfg!(target_os = "macos");
        let mut out = String::new();
        // The secondary modifier is written once as `secondary` and never again by its own name.
        let parts = [
            (m.secondary(), "secondary-"),
            (m.control && mac, "ctrl-"),
            (m.alt, "alt-"),
            (m.shift, "shift-"),
            (m.platform && !mac, "cmd-"),
            (m.function, "fn-"),
        ];
        for (on, text) in parts {
            if on {
                out.push_str(text);
            }
        }
        out.push_str(&self.0.key);
        out
    }

    /// True for no modifier, or shift alone, on a key other than f1..f24.
    pub fn needs_modifier(&self) -> bool {
        let m = self.0.modifiers;
        let shift_only = Modifiers { shift: false, ..m } == Modifiers::none();
        shift_only && !is_function_key(&self.0.key)
    }
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n) && !key.starts_with("f0"))
}

fn defaults(cmd: &Command) -> Vec<Chord> {
    cmd.default_chords
        .iter()
        .filter_map(|d| Chord::parse(d))
        .collect()
}

// An entry counts only when every keystroke in it parses; one bad string voids the entry.
fn parsed(entry: &[String]) -> Option<Vec<Chord>> {
    entry.iter().map(|s| Chord::parse(s)).collect()
}

/// The chords a command answers to: its valid override, else its defaults.
pub fn effective(cmd: &Command, overrides: &Overrides) -> Vec<Chord> {
    match overrides.get(cmd.id).map(|e| parsed(e)) {
        Some(Some(chords)) => chords,
        _ => defaults(cmd),
    }
}

/// What is wrong with a keymap.json, in file key order.
#[derive(Debug, Default, PartialEq)]
pub struct Diagnostics {
    /// Ids that name no command; kept in the file but ignored.
    pub unknown: Vec<String>,
    /// Commands whose entry does not parse; their defaults are in use.
    pub invalid: Vec<&'static str>,
}

impl Diagnostics {
    /// Whether the command's entry is invalid.
    pub fn is_invalid(&self, id: &str) -> bool {
        self.invalid.contains(&id)
    }

    /// The user-facing warnings, empty when the file is clean.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let n = self.unknown.len();
        if n > 0 {
            let noun = if n == 1 { "command" } else { "commands" };
            out.push(format!(
                "keymap.json names {n} unknown {noun}; they are kept but ignored"
            ));
        }
        let n = self.invalid.len();
        if n > 0 {
            let noun = if n == 1 { "entry" } else { "entries" };
            out.push(format!(
                "keymap.json has {n} invalid {noun}; the default keys are in use"
            ));
        }
        out
    }
}

/// Sorts the file's entries into unknown ids and known commands with an invalid entry.
pub fn diagnose(cmds: &[Command], overrides: &Overrides) -> Diagnostics {
    let mut d = Diagnostics::default();
    for (id, entry) in overrides {
        match cmds.iter().find(|c| c.id == id) {
            None => d.unknown.push(id.clone()),
            Some(c) if parsed(entry).is_none() => d.invalid.push(c.id),
            Some(_) => {}
        }
    }
    d
}

// The key context a scope names; the shell names none.
fn context(s: Scope) -> Option<&'static str> {
    match s {
        Scope::Shell => None,
        Scope::Editor(c) | Scope::Pane(c) => Some(c),
    }
}

/// Whether context `inner` can sit inside `outer`, directly or through other places.
pub fn nested_in(places: &[Place], inner: &str, outer: &str) -> bool {
    // Walk outward one ring of `within` at a time; a table of n places nests at most n deep,
    // which also stops a cyclic table.
    let mut ring = vec![inner];
    for _ in 0..places.len() {
        ring = places
            .iter()
            .filter(|p| ring.contains(&p.context))
            .flat_map(|p| p.within.iter().copied())
            .collect();
        if ring.contains(&outer) {
            return true;
        }
    }
    false
}

/// Two scopes can see the same keystroke when they are equal, when either is the shell
/// (except that a pane deliberately shadows the shell inside itself), or when one's context
/// sits inside the other's, where both answer and only one wins.
pub fn scopes_overlap_in(places: &[Place], a: Scope, b: Scope) -> bool {
    if a == b {
        return true;
    }
    let pane = |s: Scope| matches!(s, Scope::Pane(_));
    if a == Scope::Shell || b == Scope::Shell {
        return !pane(a) && !pane(b);
    }
    match (context(a), context(b)) {
        (Some(x), Some(y)) => nested_in(places, x, y) || nested_in(places, y, x),
        _ => false,
    }
}

/// `scopes_overlap_in` over the registry's place table.
pub fn scopes_overlap(a: Scope, b: Scope) -> bool {
    scopes_overlap_in(places(), a, b)
}

/// Where `other` takes a key that `this` also answers to, when `other` binds deeper than
/// `this` and so wins there; None when `other` is as wide or wider.
pub fn shown_place(this: Scope, other: Scope) -> Option<&'static str> {
    let theirs = context(other)?;
    let deeper = match context(this) {
        None => true,
        Some(ours) => nested_in(places(), theirs, ours),
    };
    let place = places().iter().find(|p| p.context == theirs)?;
    deeper.then_some(place.place)
}

/// The short name of the one area a pane-scoped command works in; None for any other scope.
pub fn scope_label(scope: Scope) -> Option<&'static str> {
    let Scope::Pane(ctx) = scope else {
        return None;
    };
    places().iter().find(|p| p.context == ctx).map(|p| p.label)
}

/// The other commands, in registry order, that answer to `chord` where command `id` does.
pub fn conflicts_with(
    cmds: &[Command],
    overrides: &Overrides,
    id: &str,
    chord: &Chord,
) -> Vec<&'static str> {
    let Some(this) = cmds.iter().find(|c| c.id == id) else {
        return Vec::new();
    };
    cmds.iter()
        .filter(|c| c.id != id && scopes_overlap(this.scope, c.scope))
        .filter(|c| effective(c, overrides).contains(chord))
        .map(|c| c.id)
        .collect()
}

/// Every command that shares one of its chords with another command in an overlapping scope.
pub fn conflicted(cmds: &[Command], overrides: &Overrides) -> BTreeSet<&'static str> {
    let keys: Vec<Vec<Chord>> = cmds.iter().map(|c| effective(c, overrides)).collect();
    let mut out = BTreeSet::new();
    for (i, a) in cmds.iter().enumerate() {
        for (j, b) in cmds.iter().enumerate().skip(i + 1) {
            let shared = keys[i].iter().any(|k| keys[j].contains(k));
            if shared && scopes_overlap(a.scope, b.scope) {
                out.insert(a.id);
                out.insert(b.id);
            }
        }
    }
    out
}

/// How a recorded chord settles a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The other commands lose the chord.
    Replace,
    /// The single other command takes this command's previous keys.
    Swap,
}

// Stores a command's chords, or drops the entry when they equal the defaults as a set.
fn write(cmd: &Command, overrides: &mut Overrides, chords: Vec<Chord>) {
    let defaults = defaults(cmd);
    let same = chords.len() == defaults.len() && chords.iter().all(|c| defaults.contains(c));
    if same {
        overrides.remove(cmd.id);
    } else {
        let strings = chords.iter().map(Chord::to_file_string).collect();
        overrides.insert(cmd.id.to_string(), strings);
    }
}

/// Gives command `id` exactly `chord` and settles every conflict as `how` says; Swap with
/// other than one conflict acts as Replace.
pub fn assign(
    cmds: &[Command],
    overrides: &mut Overrides,
    id: &str,
    chord: Chord,
    how: Resolution,
) {
    let Some(this) = cmds.iter().find(|c| c.id == id) else {
        return;
    };
    let others = conflicts_with(cmds, overrides, id, &chord);
    let previous = effective(this, overrides);
    for other in &others {
        // `others` holds only ids taken from `cmds`.
        #[allow(clippy::expect_used)]
        let c = cmds
            .iter()
            .find(|c| c.id == *other)
            .expect("conflicts are known ids");
        let keys = if how == Resolution::Swap && others.len() == 1 {
            previous.clone()
        } else {
            let mut keys = effective(c, overrides);
            keys.retain(|k| *k != chord);
            keys
        };
        write(c, overrides, keys);
    }
    write(this, overrides, vec![chord]);
}

/// Returns command `id` to its defaults, removing an invalid entry too.
pub fn reset(overrides: &mut Overrides, id: &str) {
    overrides.remove(id);
}

// The other commands, in registry order, holding any of `this` command's default chords.
fn holders_of_defaults(
    cmds: &[Command],
    overrides: &Overrides,
    this: &Command,
) -> Vec<&'static str> {
    let wanted = defaults(this);
    cmds.iter()
        .filter(|c| c.id != this.id && scopes_overlap(this.scope, c.scope))
        .filter(|c| effective(c, overrides).iter().any(|k| wanted.contains(k)))
        .map(|c| c.id)
        .collect()
}

/// The other commands, in registry order, that would share a chord with command `id` once it
/// is reset to its defaults; `None` when the reset is free.
pub fn reset_conflict(overrides: &Overrides, id: &str) -> Option<Vec<&'static str>> {
    let cmds = registry();
    let this = cmds.iter().find(|c| c.id == id)?;
    let others = holders_of_defaults(&cmds, overrides, this);
    (!others.is_empty()).then_some(others)
}

/// Resets command `id` and settles a taken default as `how` says: Replace strips the defaults
/// from the other commands, Swap (one other only, else Replace) gives it the chords this
/// command gives up.
pub fn reset_resolving(cmds: &[Command], overrides: &mut Overrides, id: &str, how: Resolution) {
    let Some(this) = cmds.iter().find(|c| c.id == id) else {
        return;
    };
    let wanted = defaults(this);
    let mut given_up = effective(this, overrides);
    given_up.retain(|k| !wanted.contains(k));
    let others = holders_of_defaults(cmds, overrides, this);
    for other in &others {
        // `others` holds only ids taken from `cmds`.
        #[allow(clippy::expect_used)]
        let c = cmds
            .iter()
            .find(|c| c.id == *other)
            .expect("conflicts are known ids");
        let keys = if how == Resolution::Swap && others.len() == 1 {
            given_up.clone()
        } else {
            let mut keys = effective(c, overrides);
            keys.retain(|k| !wanted.contains(k));
            keys
        };
        write(c, overrides, keys);
    }
    overrides.remove(id);
}

/// Returns every known command to its defaults; unknown entries stay in the file.
pub fn reset_all(cmds: &[Command], overrides: &mut Overrides) {
    overrides.retain(|id, _| !cmds.iter().any(|c| c.id == id));
}

// Whether the predicate's innermost context is `Input`, as in `Input` or `Foo > Input`.
fn names_input(p: &KeyBindingContextPredicate) -> bool {
    use KeyBindingContextPredicate as P;
    match p {
        P::Identifier(name) => name.as_ref() == "Input",
        P::Descendant(_, child) => names_input(child),
        P::And(a, b) | P::Or(a, b) => names_input(a) || names_input(b),
        _ => false,
    }
}

/// Whether the kit binds `chord` in a text input, where the input would take it first.
pub fn input_chord(kit: &[KeyBinding], chord: &Chord) -> bool {
    kit.iter().any(|b| {
        let single = match b.keystrokes() {
            [k] => Chord::from_keystroke(k.inner()).as_ref() == Some(chord),
            _ => false,
        };
        single && b.predicate().is_some_and(|p| names_input(&p))
    })
}
