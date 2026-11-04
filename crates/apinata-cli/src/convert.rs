//! `apinata convert`: turn a collection in the legacy .bru format into OpenCollection YAML,
//! into a new folder or in place, through core's legacy conversion.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use apinata_core::legacy::{
    ConversionPlan, ConvertError, ConvertTarget, convert, ensure_new_folder_free, plan_conversion,
};

use crate::EXIT_USAGE;

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("target").required(true).args(["out", "in_place"])))]
pub struct ConvertArgs {
    /// The collection directory in the legacy .bru format.
    pub dir: PathBuf,
    /// Convert into this new directory, leaving the original untouched. NEW_DIR must not exist yet.
    #[arg(long, value_name = "NEW_DIR")]
    pub out: Option<PathBuf>,
    /// Convert where it is: write the YAML files, then delete the .bru files.
    #[arg(long)]
    pub in_place: bool,
    /// Convert in place without asking.
    #[arg(long, conflicts_with = "out")]
    pub yes: bool,
    /// List what would change and write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Plan the conversion, list it when asked or before an in-place change, then convert and
/// print the converted root: 0 when converted, planned or declined, 3 on any failure.
pub fn run(args: &ConvertArgs) -> ExitCode {
    let fail = |message: String| {
        eprintln!("error: {message}");
        ExitCode::from(EXIT_USAGE)
    };
    let dir = match std::fs::canonicalize(&args.dir) {
        Ok(dir) => dir,
        Err(e) => return fail(format!("cannot open {}: {e}", args.dir.display())),
    };
    let target = match &args.out {
        Some(out) => match std::path::absolute(out) {
            Ok(out) => ConvertTarget::NewFolder(out),
            Err(e) => return fail(format!("cannot use {}: {e}", out.display())),
        },
        None => ConvertTarget::InPlace,
    };
    let plan = match plan_conversion(&dir) {
        Ok(plan) => plan,
        Err(e) => return report(&plan_error_lines(&e)),
    };

    // An in-place change needs a yes from a person, so refuse before listing anything when
    // nobody can answer.
    let ask = args.in_place && !args.dry_run && !args.yes;
    if ask && !std::io::stdin().is_terminal() {
        return fail("converting in place deletes the .bru files; pass --yes to confirm".into());
    }
    // A dry run refuses an existing new-folder target as the real run would, rather than
    // listing a plan that cannot be carried out.
    if let (true, ConvertTarget::NewFolder(out)) = (args.dry_run, &target)
        && let Err(error) = ensure_new_folder_free(out)
    {
        return report(&plan_error_lines(&error));
    }
    if args.dry_run || args.in_place {
        list(&plan, args.in_place);
    }
    if args.dry_run {
        return ExitCode::SUCCESS;
    }
    if ask && !confirm() {
        outln!("Nothing was converted.");
        return ExitCode::SUCCESS;
    }
    match convert(&plan, target) {
        Ok(root) => {
            outln!("{}", std::fs::canonicalize(&root).unwrap_or(root).display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            let mut lines = plan_error_lines(&e);
            if args.in_place {
                lines.extend(left_over_note(&plan, &e));
            }
            report(&lines)
        }
    }
}

/// Print the planned changes: the YAML files created, then the files copied into the new
/// folder or, in place, the legacy files deleted.
fn list(plan: &ConversionPlan, in_place: bool) {
    for rel in plan.created() {
        outln!("create {}", rel.display());
    }
    let (verb, rest) = if in_place {
        ("delete", plan.removed_in_place())
    } else {
        ("copy", plan.copied())
    };
    for rel in rest {
        outln!("{verb} {}", rel.display());
    }
}

/// The error line, then each failing file or colliding path on its own indented line.
fn plan_error_lines(error: &ConvertError) -> Vec<String> {
    let mut lines = vec![format!("error: {error}")];
    match error {
        ConvertError::Parse { errors } => lines.extend(errors.iter().map(|e| format!("  {e}"))),
        ConvertError::TargetExists { paths } => {
            lines.extend(paths.iter().map(|p| format!("  {}", p.display())));
        }
        _ => {}
    }
    lines
}

/// For an in-place failure while deleting a legacy file, say that the YAML collection is
/// complete and list the legacy files still to remove by hand. A write failure, which core
/// rolls back, gets no note.
fn left_over_note(plan: &ConversionPlan, error: &ConvertError) -> Vec<String> {
    let Some(left) = plan.left_over(error) else {
        return Vec::new();
    };
    let mut lines = vec![
        "The YAML collection is complete, but some legacy files were not deleted.".to_owned(),
        "Remove these leftover legacy files by hand; converting again stops at the YAML files:"
            .to_owned(),
    ];
    lines.extend(left.iter().map(|rel| format!("  {}", rel.display())));
    lines
}

fn report(lines: &[String]) -> ExitCode {
    for line in lines {
        eprintln!("{line}");
    }
    ExitCode::from(EXIT_USAGE)
}

/// Asks on stdin; only `y` or `yes`, in any case, agrees, and end of input declines.
fn confirm() -> bool {
    out!("Convert in place and delete the legacy files? [y/N] ");
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim();
    answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn legacy() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("bruno.json"),
            r#"{"version": "1", "name": "Legacy", "type": "collection"}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("a.bru"),
            "meta {\n  name: a\n  type: http\n  seq: 1\n}\n\nget {\n  url: https://example.com\n}\n",
        )
        .unwrap();
        dir
    }

    fn io(path: PathBuf) -> ConvertError {
        ConvertError::Io {
            path,
            message: "denied".into(),
        }
    }

    #[test]
    fn a_failed_delete_says_the_yaml_is_complete_and_lists_the_leftovers() {
        let src = legacy();
        let plan = plan_conversion(src.path()).unwrap();
        let note = left_over_note(&plan, &io(src.path().join("a.bru")));
        assert_eq!(
            note,
            [
                "The YAML collection is complete, but some legacy files were not deleted.",
                "Remove these leftover legacy files by hand; converting again stops at the YAML files:",
                "  a.bru",
                "  bruno.json",
            ]
        );
    }

    #[test]
    fn a_failed_write_gets_no_leftover_note() {
        let src = legacy();
        let plan = plan_conversion(src.path()).unwrap();
        assert!(left_over_note(&plan, &io(src.path().join("a.yml"))).is_empty());
    }
}
