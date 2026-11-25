//! `cargo xtask notes-json`: writes the `notes.json` the updater shows, rebuilt each release
//! from every published release body plus this release's generated notes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, anyhow, bail};
use apinata_core::update::{GroupKind, NotesGroup, NotesItem, NotesRelease, ReleaseNotes};
use semver::Version;
use serde::Deserialize;

/// The largest notes.json written, in bytes.
pub const SIZE_CAP: usize = 48 * 1024;

/// The most releases one notes.json lists.
const MAX_RELEASES: usize = 20;

/// The release being built.
pub struct Current<'a> {
    /// Its version, a stable semver without the `v`.
    pub version: &'a str,
    /// Its date, `YYYY-MM-DD`.
    pub date: &'a str,
    /// The generate-notes body for its tag.
    pub generated: &'a str,
    /// Its tag annotation, the source of its intro.
    pub intro: Option<&'a str>,
}

/// One published release, as the releases API lists it.
pub struct Published {
    /// The tag, such as `v1.2.0`.
    pub tag: String,
    /// The publication date, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// The release body markdown.
    pub body: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Page {
    Many(Vec<GhRelease>),
    One(Box<GhRelease>),
}

/// The published releases in every `*.json` file in `dir`, each holding releases API pages
/// (one after another, as `gh api --paginate` prints them). Drafts and pre-releases are left out.
pub fn read_published(dir: &Path) -> anyhow::Result<Vec<Published>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|p| p.extension().is_some_and(|e| e == "json"));
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        for page in serde_json::Deserializer::from_str(&text).into_iter::<Page>() {
            let page = page.with_context(|| format!("parsing {}", file.display()))?;
            let releases = match page {
                Page::Many(all) => all,
                Page::One(one) => vec![*one],
            };
            out.extend(
                (releases.into_iter())
                    .filter(|r| !r.draft && !r.prerelease)
                    .map(|r| Published {
                        tag: r.tag_name,
                        date: r.published_at.and_then(|d| d.get(..10).map(str::to_owned)),
                        body: r.body.unwrap_or_default(),
                    }),
            );
        }
    }
    Ok(out)
}

/// The notes.json text for `current` and the releases before it, with section kinds taken
/// from `categories` (the `.github/release.yml` text). Fails on unreadable notes or when the
/// newest release cannot fit in [`SIZE_CAP`].
pub fn build(
    current: &Current,
    published: Vec<Published>,
    categories: &str,
) -> anyhow::Result<String> {
    let version = Version::parse(current.version)
        .with_context(|| format!("version {:?}", current.version))?;
    if !version.pre.is_empty() {
        bail!("version {version} is a pre-release");
    }
    let kinds = kinds(categories)?;

    // Only stable app releases older than this one count; other crates' tags do not parse.
    let mut history: Vec<(Version, Published)> = (published.into_iter())
        .filter_map(|p| {
            let v = Version::parse(p.tag.strip_prefix('v')?).ok()?;
            (v.pre.is_empty() && v < version).then_some((v, p))
        })
        .collect();
    history.sort_by(|(a, _), (b, _)| b.cmp(a));
    history.dedup_by(|(a, _), (b, _)| a == b);

    let newest = parse_body(current.generated, &kinds)
        .map_err(|e| anyhow!("the generated notes for {version}: {e}"))?;
    let mut releases = vec![NotesRelease::new(
        version.to_string(),
        None,
        Some(current.date.to_owned()),
        newest.groups,
    )];
    if let Some(intro) = current.intro.and_then(|a| tag_intro(a, &version)) {
        releases[0] = releases[0].clone().with_intro(intro);
    }
    // `previous` comes from the whole history, so the oldest entry kept still names its
    // predecessor after the count cap.
    let previous: Vec<String> = history.iter().map(|(v, _)| v.to_string()).collect();
    releases[0].previous = previous.first().cloned();
    for (i, (v, p)) in history.iter().take(MAX_RELEASES - 1).enumerate() {
        let parsed =
            parse_body(&p.body, &kinds).map_err(|e| anyhow!("the published notes for {v}: {e}"))?;
        let mut r = NotesRelease::new(
            v.to_string(),
            previous.get(i + 1).cloned(),
            p.date.clone(),
            parsed.groups,
        );
        if let Some(intro) = parsed.intro {
            r = r.with_intro(intro);
        }
        releases.push(r);
    }
    fit(releases)
}

/// Category title to kind, from the labels each `.github/release.yml` category collects.
fn kinds(categories: &str) -> anyhow::Result<HashMap<String, GroupKind>> {
    #[derive(Deserialize)]
    struct Config {
        changelog: Changelog,
    }
    #[derive(Deserialize)]
    struct Changelog {
        #[serde(default)]
        categories: Vec<Category>,
    }
    #[derive(Deserialize)]
    struct Category {
        title: String,
        #[serde(default)]
        labels: Vec<String>,
    }
    let config: Config = yaml_serde::from_str(categories).context("parsing release.yml")?;
    Ok((config.changelog.categories.into_iter())
        .map(|c| {
            let kind = (c.labels.iter())
                .find_map(|l| match l.as_str() {
                    "feature" => Some(GroupKind::Features),
                    "bug" => Some(GroupKind::Fixes),
                    "ux" | "accessibility" => Some(GroupKind::Usability),
                    "perf" => Some(GroupKind::Performance),
                    _ => None,
                })
                .unwrap_or(GroupKind::Other);
            (c.title, kind)
        })
        .collect())
}

struct Parsed {
    intro: Option<String>,
    groups: Vec<NotesGroup>,
}

/// Reads the `## What's Changed` section of a release body, up to the next `##` heading
/// (Downloads, or the generated New Contributors). The text above it is the intro.
fn parse_body(body: &str, kinds: &HashMap<String, GroupKind>) -> Result<Parsed, String> {
    let lines: Vec<&str> = body.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim() == "## What's Changed") else {
        return Ok(Parsed {
            intro: None,
            groups: Vec::new(),
        });
    };
    let intro = plain(&lines[..start].join(" "));
    let mut groups: Vec<NotesGroup> = Vec::new();
    let mut bullets = 0;
    for line in lines[start + 1..].iter().map(|l| l.trim()) {
        if line.starts_with("## ") {
            break;
        }
        if let Some(title) = line.strip_prefix("### ") {
            let title = plain(title);
            let kind = kinds.get(&title).copied().unwrap_or(GroupKind::Other);
            groups.push(NotesGroup::new(kind, title, Vec::new()));
            continue;
        }
        let Some(text) = ["* ", "- ", "+ "].iter().find_map(|b| line.strip_prefix(b)) else {
            continue;
        };
        bullets += 1;
        let Some(item) = parse_item(text) else {
            continue;
        };
        if groups.is_empty() {
            groups.push(NotesGroup::new(GroupKind::Other, "Changes", Vec::new()));
        }
        if let Some(group) = groups.last_mut() {
            group.items.push(item);
        }
    }
    groups.retain(|g| !g.items.is_empty());
    if bullets > 0 && groups.is_empty() {
        return Err(format!("none of its {bullets} bullets has readable text"));
    }
    Ok(Parsed {
        intro: Some(intro).filter(|i| !i.is_empty()),
        groups,
    })
}

/// One bullet as plain text and its pull request: the generated ` by @user in URL` tail,
/// a pull request URL or a trailing `#n` gives the number; links and markdown are dropped.
fn parse_item(text: &str) -> Option<NotesItem> {
    let mut s = format!(" {}", text.trim());
    let mut pr = None;
    if let Some(at) = s.rfind(" by @")
        && let Some((user, url)) = s[at + 5..].split_once(" in ")
        && !user.contains(char::is_whitespace)
        && !url.trim().contains(char::is_whitespace)
    {
        pr = pull_number(url.trim());
        s.truncate(at);
    }
    let s = strip_urls(&strip_html(&unlink(&s)), &mut pr);
    let mut s = collapse(&strip_emphasis(&s));
    // A trailing reference such as `(#12)` or `#12`.
    let bare = s.strip_suffix(')').unwrap_or(&s);
    if let Some(hash) = bare.rfind('#')
        && let Ok(n) = bare[hash + 1..].parse::<u64>()
        && (hash == 0 || bare[..hash].ends_with([' ', '(']))
    {
        pr = pr.or(Some(n));
        let head = bare[..hash].trim_end_matches('(').trim_end();
        s = head.to_owned();
    }
    if s.is_empty() || s.starts_with("Full Changelog") {
        return None;
    }
    let item = NotesItem::new(s);
    Some(match pr {
        Some(n) => item.with_pr(n),
        None => item,
    })
}

/// The number in a `.../pull/N` URL.
fn pull_number(url: &str) -> Option<u64> {
    let (_, tail) = url.rsplit_once("/pull/")?;
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Markdown and links removed, whitespace collapsed: the form of titles and intros.
fn plain(text: &str) -> String {
    collapse(&strip_emphasis(&strip_urls(
        &strip_html(&unlink(text)),
        &mut None,
    )))
}

/// `[text](url)` and `![text](url)` become `text`.
fn unlink(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            let head = &rest[..open];
            out.push_str(head.strip_suffix('!').unwrap_or(head));
            out.push_str(&after[..close]);
            rest = &after[close + 2 + end + 1..];
        } else {
            out.push_str(&rest[..=open]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// HTML tags, comments and `<url>` autolinks removed.
fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let tag = after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!');
        match after.find('>') {
            Some(end) if tag => {
                out.push_str(&rest[..open]);
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Bare URLs removed; a pull request URL fills `pr` when it is still empty.
fn strip_urls(s: &str, pr: &mut Option<u64>) -> String {
    let words: Vec<&str> = (s.split_whitespace())
        .filter(|w| {
            let url = w.trim_start_matches('(');
            let is_url = url.starts_with("https://") || url.starts_with("http://");
            if is_url && pr.is_none() {
                *pr = pull_number(url);
            }
            !is_url
        })
        .collect();
    words.join(" ")
}

/// Emphasis, strike-through and code markers removed; an underscore inside a word stays.
fn strip_emphasis(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let word = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_alphanumeric())
    };
    (chars.iter().enumerate())
        .filter(|&(i, &c)| match c {
            '*' | '`' | '~' => false,
            '_' => word(i.checked_sub(1)) && word(Some(i + 1)),
            _ => true,
        })
        .map(|(_, &c)| c)
        .collect()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The intro from a tag annotation, or none when the annotation only names the release.
fn tag_intro(annotation: &str, version: &Version) -> Option<String> {
    let text = plain(annotation);
    let names = [
        version.to_string(),
        format!("v{version}"),
        format!("Apinata {version}"),
        format!("Apinata v{version}"),
    ];
    let names_it = names.iter().any(|n| n.eq_ignore_ascii_case(&text));
    (!text.is_empty() && !names_it).then_some(text)
}

fn render(releases: &[NotesRelease]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(&ReleaseNotes::new(releases.to_vec()))? + "\n")
}

/// Drops the oldest releases until the notes fit; if the newest alone is still too large,
/// keeps as many of its items as fit and counts the rest in a last "...and N more".
fn fit(mut releases: Vec<NotesRelease>) -> anyhow::Result<String> {
    loop {
        let json = render(&releases)?;
        if json.len() <= SIZE_CAP {
            return Ok(json);
        }
        if releases.len() == 1 {
            break;
        }
        releases.pop();
    }
    let newest = &releases[0];
    let total: usize = newest.groups.iter().map(|g| g.items.len()).sum();
    let fits = |keep: usize| -> anyhow::Result<Option<String>> {
        let json = render(&[cut(newest, keep, total)])?;
        Ok((json.len() <= SIZE_CAP).then_some(json))
    };
    let Some(mut best) = (if total > 0 { fits(0)? } else { None }) else {
        bail!(
            "the notes for {} do not fit in {SIZE_CAP} bytes on their own",
            newest.version
        );
    };
    // Keeping `lo` items fits and keeping `hi` does not.
    let (mut lo, mut hi) = (0, total);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        match fits(mid)? {
            Some(json) => (lo, best) = (mid, json),
            None => hi = mid,
        }
    }
    Ok(best)
}

/// `release` with only its first `keep` items, ending its last group with a count of the rest.
fn cut(release: &NotesRelease, keep: usize, total: usize) -> NotesRelease {
    let mut r = release.clone();
    let mut left = keep;
    for g in &mut r.groups {
        let n = g.items.len().min(left);
        g.items.truncate(n);
        left -= n;
    }
    let first = r.groups.first().map(|g| (g.kind, g.title.clone()));
    r.groups.retain(|g| !g.items.is_empty());
    if r.groups.is_empty()
        && let Some((kind, title)) = first
    {
        r.groups.push(NotesGroup::new(kind, title, Vec::new()));
    }
    if let Some(last) = r.groups.last_mut() {
        last.items
            .push(NotesItem::new(format!("...and {} more", total - keep)));
    }
    r
}

/// Reads the inputs, then writes `output`.
pub fn run(
    version: &str,
    date: &str,
    generated: &Path,
    published: &Path,
    intro: Option<&Path>,
    categories: Option<&Path>,
    output: &Path,
) -> anyhow::Result<ExitCode> {
    let read =
        |p: &Path| std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()));
    let generated = read(generated)?;
    let intro = intro.map(read).transpose()?;
    let categories = match categories {
        Some(p) => read(p)?,
        None => read(&crate::workspace_root().join(".github/release.yml"))?,
    };
    let current = Current {
        version,
        date,
        generated: &generated,
        intro: intro.as_deref(),
    };
    let json = build(&current, read_published(published)?, &categories)?;
    std::fs::write(output, json).with_context(|| format!("writing {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(ExitCode::SUCCESS)
}
