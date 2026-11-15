//! Compile-only syntax check of script segments.

use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Function, Module, Runtime};

use super::engine::{SCRIPT_FILE, Segment, attribute, bounded_runtime, error_line, wrap};
use crate::ScriptOptions;

/// One compile problem of one segment. `sloppy_only` is true when the code is valid for the
/// engine (sloppy mode) and fails only a stricter parse, so the caller reports a Warning.
pub(crate) struct Problem {
    /// The level of the segment that failed.
    pub(crate) level: String,
    pub(crate) message: String,
    pub(crate) sloppy_only: bool,
}

/// Compile-only check of every segment; never evaluates, never calls. Fails only when the
/// engine cannot start, so the caller can say the scripts went unchecked.
pub(crate) fn syntax_problems(segments: &[Segment]) -> Result<Vec<Problem>, rquickjs::Error> {
    let rt = check_runtime()?;
    let ctx = Context::full(&rt)?;
    Ok(ctx.with(|ctx| syntax_problems_in(&ctx, segments)))
}

/// How long a check's runtime may run bytecode; a parse runs none of the checked code.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// A runtime for compile-only checks of untrusted text: the memory limit of a trusted run and
/// a short deadline, so a check that went wrong can neither hang nor exhaust memory. The
/// deadline bounds running bytecode only, not parse time; only the memory limit bounds a parse.
pub(crate) fn check_runtime() -> Result<Runtime, rquickjs::Error> {
    let memory = ScriptOptions::default().limits(0).memory;
    bounded_runtime(Instant::now().checked_add(CHECK_TIMEOUT), memory).map(|(rt, _)| rt)
}

/// The same on a caller-supplied context (a fresh one in `syntax_problems`), so a test can
/// inject a context that holds a probe global.
pub(crate) fn syntax_problems_in(ctx: &Ctx<'_>, segments: &[Segment]) -> Vec<Problem> {
    segments
        .iter()
        .filter_map(|segment| problem(ctx, segment))
        .collect()
}

/// Declares one wrapped segment as a module, which parses it without running anything. Module
/// code is strict, so a failure that the engine's sloppy parse accepts is `sloppy_only`.
fn problem(ctx: &Ctx<'_>, segment: &Segment) -> Option<Problem> {
    let one = std::slice::from_ref(segment);
    let (source, starts) = wrap(one);
    let CaughtError::Exception(e) = Module::declare(ctx.clone(), SCRIPT_FILE, source.clone())
        .catch(ctx)
        .err()?
    else {
        return None;
    };
    let text = e.message().unwrap_or_default();
    let message = attribute(one, &starts, error_line(e.as_object()), 0, &text);
    Some(Problem {
        level: segment.level.clone(),
        message,
        sloppy_only: parses_sloppy(ctx, &source),
    })
}

/// Whether `source` parses as a sloppy-mode script, as the engine evaluates scripts, without
/// running any of it. Untrusted text must be checked through this, in a `check_runtime` context.
///
/// The script is prefixed with a throw of a marker, so evaluation stops before the first
/// token of `source` whatever brackets it closes; the marker coming back means it parsed.
/// Top-level declarations a breakout makes are still bound, hence the check-only context.
pub(crate) fn parses_sloppy(ctx: &Ctx<'_>, source: &str) -> bool {
    let mut options = EvalOptions::default();
    options.strict = false;
    options.filename = Some(SCRIPT_FILE.to_owned());
    let guarded = format!("throw {PARSED:?};\n{source}");
    match ctx.eval_with_options::<(), _>(guarded, options).catch(ctx) {
        Err(CaughtError::Value(thrown)) => {
            thrown
                .as_string()
                .and_then(|s| s.to_string().ok())
                .as_deref()
                == Some(PARSED)
        }
        _ => false,
    }
}

/// Whether `text` parses as the body of a template literal, the code an assertion's right
/// side runs as. The function is built and dropped, never called; an engine that cannot start
/// counts as parsing, so the caller still asks before anything runs.
pub(crate) fn parses_as_template(text: &str) -> bool {
    let Ok(rt) = Runtime::new() else { return true };
    let Ok(ctx) = Context::full(&rt) else {
        return true;
    };
    ctx.with(|ctx| {
        ctx.globals()
            .get::<_, Function>("Function")
            .and_then(|constructor| constructor.call::<_, Function>((format!("`{text}`"),)))
            .is_ok()
    })
}

/// The marker `parses_sloppy` throws ahead of the checked text.
const PARSED: &str = "apinata: parsed";

#[cfg(test)]
mod tests {
    use super::super::engine::wrap;
    use super::*;
    use rquickjs::{Context, Runtime};

    fn seg(level: &str, code: &str) -> Segment {
        Segment {
            level: level.to_owned(),
            phase: crate::ScriptPhase::PreRequest,
            code: code.to_owned(),
        }
    }

    fn errors(segs: &[Segment]) -> Vec<String> {
        syntax_problems(segs)
            .expect("engine starts")
            .into_iter()
            .filter(|p| !p.sloppy_only)
            .map(|p| p.message)
            .collect()
    }

    #[test]
    fn valid_segments_have_no_problems() {
        let segs = [
            seg("request", "const a = 1;\nreturn a;"),
            seg("collection", "await Promise.resolve(2);"),
        ];
        assert!(syntax_problems(&segs).unwrap().is_empty());
    }

    #[test]
    fn a_syntax_error_names_the_level_and_the_segment_line() {
        let errs = errors(&[seg("request", "let ok = 1;\nlet a = ;")]);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(
            errs[0].starts_with("this request's script, line 2:"),
            "{errs:?}"
        );
    }

    #[test]
    fn a_tests_syntax_error_names_the_tests() {
        let tests = Segment {
            phase: crate::ScriptPhase::Tests,
            ..seg("folder a", "let ok = 1;\nlet a = ;")
        };
        let errs = errors(&[tests]);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].starts_with("folder a tests, line 2:"), "{errs:?}");
    }

    #[test]
    fn each_bad_segment_is_reported_once() {
        let errs = errors(&[
            seg("collection", "}{"),
            seg("request", "fine()"),
            seg("folder a", "("),
        ]);
        assert_eq!(errs.len(), 2, "{errs:?}");
        assert!(errs[0].starts_with("collection script"));
        assert!(errs[1].starts_with("folder a script"));
    }

    #[test]
    fn code_the_engine_accepts_is_never_an_error() {
        for code in ["var static = 1;", "x = 010;", "with (a) {}"] {
            assert!(errors(&[seg("request", code)]).is_empty(), "{code}");
        }
    }

    #[test]
    fn strict_only_failures_are_sloppy_only_and_real_errors_are_not() {
        let classify = |code: &str| -> Vec<bool> {
            let problems = syntax_problems(&[seg("request", code)]).expect("engine starts");
            problems.into_iter().map(|p| p.sloppy_only).collect()
        };
        for code in [
            "var static = 1;",
            "x = 010;",
            "with (a) {}",
            "<!-- old comment",
        ] {
            assert_eq!(classify(code), [true], "{code}");
        }
        for code in ["let a = ;", "with (a) {}\nlet a = ;", "}{", "("] {
            assert_eq!(classify(code), [false], "{code}");
        }
    }

    /// A segment that closes the wrapper and runs code of its own.
    const BREAKOUT: &str = "})(); globalThis.__ran = true; (function(){";

    fn probe_context(rt: &Runtime) -> Context {
        let ctx = Context::full(rt).unwrap();
        ctx.with(|c| {
            c.eval::<(), _>("globalThis.__apinata = {}; globalThis.__ran = false;")
                .unwrap()
        });
        ctx
    }

    fn ran(ctx: &Context) -> bool {
        ctx.with(|c| c.eval::<bool, _>("globalThis.__ran === true").unwrap())
    }

    fn drain(rt: &Runtime) {
        while rt.is_job_pending() {
            let _ = rt.execute_pending_job();
        }
    }

    #[test]
    fn a_breakout_segment_is_never_executed() {
        // Control: evaluating the wrapped breakout does set the flag, so the probe can see it.
        let rt = Runtime::new().unwrap();
        let ctx = probe_context(&rt);
        let (source, _) = wrap(&[seg("request", BREAKOUT)]);
        ctx.with(|c| {
            let _ = c.eval::<rquickjs::Value, _>(source);
        });
        drain(&rt);
        assert!(ran(&ctx), "control: evaluation must run the breakout");

        // Under test: checking the same segment in a fresh probe context leaves the flag false.
        let rt = Runtime::new().unwrap();
        let ctx = probe_context(&rt);
        ctx.with(|c| {
            let _ = syntax_problems_in(&c, &[seg("request", BREAKOUT)]);
        });
        drain(&rt);
        assert!(!ran(&ctx), "checking must not run the segment");
    }

    #[test]
    fn the_check_runtime_has_the_trusted_memory_limit_and_a_deadline() {
        // The loop runs on its own thread so a missing interrupt fails the test, never hangs it.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = check_runtime().expect("engine starts");
            let limit = rt.memory_usage().malloc_limit;
            let ctx = Context::full(&rt).expect("context starts");
            let looped = ctx.with(|c| c.eval::<(), _>("for (;;) {}").is_err());
            let _ = tx.send((limit, looped));
        });
        let (limit, stopped) = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the interrupt stops an endless loop");
        let trusted = crate::ScriptOptions::default().limits(0).memory;
        assert_eq!(usize::try_from(limit).ok(), Some(trusted), "memory limit");
        assert!(stopped, "the loop ends in an error");
    }

    /// Segments that fail the strict module parse (`with`) and close a sloppy wrapper early: the
    /// first closes one function, the second both of the engine's wrapper functions.
    const SLOPPY_BREAKOUTS: [&str; 2] = [
        "}); globalThis.__ran = true; for (;;) {} with (Math) {} (function () {",
        "})(); })(); globalThis.__ran = true; for (;;) {} with (Math) {} (async () => { (async () => {",
    ];

    #[test]
    fn a_sloppy_breakout_is_never_executed() {
        for code in SLOPPY_BREAKOUTS {
            // The bounded check runtime ends the loop should the breakout ever run.
            let rt = check_runtime().unwrap();
            let ctx = probe_context(&rt);
            let problems = ctx.with(|c| syntax_problems_in(&c, &[seg("request", code)]));
            drain(&rt);
            assert_eq!(problems.len(), 1, "the strict parse rejects {code}");
            assert!(problems[0].sloppy_only, "the sloppy parse accepts {code}");
            assert!(!ran(&ctx), "checking must not run {code}");
        }
    }
}
