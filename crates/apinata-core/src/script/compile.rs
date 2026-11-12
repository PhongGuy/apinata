//! Compile-only syntax check of script segments.

use rquickjs::{CatchResultExt, CaughtError, Context, Ctx, Function, Module, Runtime};

use super::engine::{SCRIPT_FILE, Segment, attribute, error_line, wrap};

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
    let rt = Runtime::new()?;
    let ctx = Context::full(&rt)?;
    Ok(ctx.with(|ctx| syntax_problems_in(&ctx, segments)))
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
/// code is strict, so a failure that the sloppy `AsyncFunction` parse accepts is `sloppy_only`.
fn problem(ctx: &Ctx<'_>, segment: &Segment) -> Option<Problem> {
    let one = std::slice::from_ref(segment);
    let (source, starts) = wrap(one);
    let CaughtError::Exception(e) = Module::declare(ctx.clone(), SCRIPT_FILE, source)
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
        sloppy_only: parses_sloppy(ctx, &segment.code),
    })
}

/// Whether the engine's own sloppy parse accepts the text as a function body. The function
/// is built and dropped, never called.
fn parses_sloppy(ctx: &Ctx<'_>, code: &str) -> bool {
    ctx.eval::<Function, _>("Object.getPrototypeOf(async function () {}).constructor")
        .and_then(|constructor| constructor.call::<_, Function>((code,)))
        .is_ok()
}

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
        assert!(errs[0].starts_with("request script, line 2:"), "{errs:?}");
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
}
