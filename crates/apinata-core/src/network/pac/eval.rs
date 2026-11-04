//! The PAC sandbox: a fresh QuickJS runtime per evaluation with a memory cap, a per-step time
//! budget, the standard PAC helpers and no other host access.

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rquickjs::context::EvalOptions;
use rquickjs::{CatchResultExt, CaughtError, Coerced, Context, Ctx, Function, Runtime, Value};

/// How long each of the two steps (the top level, then the call) may run.
pub(crate) const STEP_LIMIT: Duration = Duration::from_secs(1);
/// The heap a PAC script may allocate.
pub(crate) const MEMORY_LIMIT: usize = 16 << 20;
/// How long one DNS lookup may block. The interrupt handler cannot stop a lookup, so this keeps a
/// slow resolver from carrying a step far past its budget while leaving room for a few lookups.
pub(crate) const DNS_LIMIT: Duration = Duration::from_millis(250);

/// Resolves a name to its first IPv4 address; tests stand in a stub for DNS.
pub(crate) type Resolver = fn(&str) -> Option<Ipv4Addr>;
/// How many lookups may be blocked at once across the process.
pub(crate) const DNS_IN_FLIGHT_CAP: usize = 8;
/// The lookups now running, for `DNS_IN_FLIGHT_CAP`.
static DNS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// The standard helpers, defined over the private host functions that `install` sets.
const HELPERS: &str = include_str!("helpers.js");

/// Why a PAC evaluation produced no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvalError {
    /// The top level threw or did not parse, or it defined no `FindProxyForURL` function.
    Compile(String),
    /// `FindProxyForURL` threw, or ran out of memory or stack.
    Script(String),
    /// A step ran past its time budget.
    Timeout,
}

/// Blocking. Runs `script`'s top level, then `FindProxyForURL(url, host)`, each step under
/// `STEP_LIMIT`; a non-string answer is the empty string.
pub(crate) fn evaluate(script: &str, url: &str, host: &str) -> Result<String, EvalError> {
    evaluate_within(script, url, host, STEP_LIMIT)
}

/// `evaluate` with another step budget, for tests.
pub(crate) fn evaluate_within(
    script: &str,
    url: &str,
    host: &str,
    step: Duration,
) -> Result<String, EvalError> {
    evaluate_with(script, url, host, step, first_ipv4, &DNS_IN_FLIGHT)
}

/// `evaluate_within` over another resolver, for tests.
fn evaluate_with(
    script: &str,
    url: &str,
    host: &str,
    step: Duration,
    resolve: Resolver,
    in_flight: &'static AtomicUsize,
) -> Result<String, EvalError> {
    let no_host =
        |e: rquickjs::Error| EvalError::Script(format!("the PAC sandbox could not start: {e}"));
    let rt = Runtime::new().map_err(no_host)?;
    rt.set_memory_limit(MEMORY_LIMIT);
    // One deadline, moved forward before the call; `hit` tells a timeout from a throw.
    let deadline = Arc::new(Mutex::new(Instant::now() + step));
    let hit = Arc::new(AtomicBool::new(false));
    let (until, flag) = (Arc::clone(&deadline), Arc::clone(&hit));
    rt.set_interrupt_handler(Some(Box::new(move || {
        let over = Instant::now() >= *until.lock().unwrap_or_else(PoisonError::into_inner);
        if over {
            flag.store(true, Relaxed);
        }
        over
    })));
    let context = Context::full(&rt).map_err(no_host)?;
    context.with(|ctx| {
        install(&ctx, resolve, in_flight, &deadline).map_err(no_host)?;
        ctx.eval_with_options::<(), _>(HELPERS, sloppy())
            .catch(&ctx)
            .map_err(|e| failed(&hit, EvalError::Script, e))?;
        ctx.eval_with_options::<(), _>(script, sloppy())
            .catch(&ctx)
            .map_err(|e| failed(&hit, EvalError::Compile, e))?;
        let find: Value = ctx
            .globals()
            .get("FindProxyForURL")
            .map_err(|e| EvalError::Compile(e.to_string()))?;
        let Some(find) = find.into_function() else {
            return Err(EvalError::Compile(
                "the script defines no FindProxyForURL function".to_owned(),
            ));
        };
        *deadline.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now() + step;
        let answer: Value = find
            .call((url, host))
            .catch(&ctx)
            .map_err(|e| failed(&hit, EvalError::Script, e))?;
        Ok(answer
            .into_string()
            .and_then(|s| s.to_string().ok())
            .unwrap_or_default())
    })
}

/// Most PAC files are sloppy-mode JavaScript, so strict mode is off.
fn sloppy() -> EvalOptions {
    let mut options = EvalOptions::default();
    options.strict = false;
    options
}

/// A timeout when the interrupt fired, else `kind` with the thrown message.
fn failed(hit: &AtomicBool, kind: fn(String) -> EvalError, error: CaughtError<'_>) -> EvalError {
    if hit.load(Relaxed) {
        EvalError::Timeout
    } else {
        kind(describe(error))
    }
}

/// The thrown message without a stack; an empty one becomes a generic phrase.
fn describe(error: CaughtError<'_>) -> String {
    let text = match error {
        CaughtError::Exception(e) => e.message().unwrap_or_default(),
        CaughtError::Value(value) => value
            .get::<Coerced<String>>()
            .map(|text| text.0)
            .unwrap_or_default(),
        CaughtError::Error(e) => e.to_string(),
    };
    if text.is_empty() {
        "an error was thrown".to_owned()
    } else {
        text
    }
}

/// Sets the private host functions that `helpers.js` captures and then deletes.
fn install(
    ctx: &Ctx<'_>,
    resolve: Resolver,
    in_flight: &'static AtomicUsize,
    deadline: &Arc<Mutex<Instant>>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let until = Arc::clone(deadline);
    globals.set(
        "__pacDnsResolve",
        Function::new(ctx.clone(), move |host: String| {
            bounded(resolve, in_flight, host, &until).map(|ip| ip.to_string())
        })?,
    )?;
    let until = Arc::clone(deadline);
    globals.set(
        "__pacIsInNet",
        Function::new(
            ctx.clone(),
            move |host: String, pattern: String, mask: String| {
                is_in_net(bounded(resolve, in_flight, host, &until), &pattern, &mask)
            },
        )?,
    )?;
    globals.set(
        "__pacMyIpAddress",
        Function::new(ctx.clone(), || my_ip_address().to_string())?,
    )?;
    Ok(())
}

/// `resolve(host)` on its own thread, given up as unanswered after `DNS_LIMIT` or at the step
/// deadline, whichever comes first; a lookup still blocked then finishes on its abandoned thread.
/// No thread starts once the step is spent or while `in_flight` is at `DNS_IN_FLIGHT_CAP`.
fn bounded(
    resolve: Resolver,
    in_flight: &'static AtomicUsize,
    host: String,
    deadline: &Mutex<Instant>,
) -> Option<Ipv4Addr> {
    let until = *deadline.lock().unwrap_or_else(PoisonError::into_inner);
    let wait = DNS_LIMIT.min(until.saturating_duration_since(Instant::now()));
    if wait.is_zero() {
        return None;
    }
    let slot = Slot::take(in_flight)?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("pac-dns".to_owned())
        .spawn(move || {
            let _slot = slot;
            tx.send(resolve(&host))
        })
        .ok()?;
    rx.recv_timeout(wait).ok().flatten()
}

/// One of the `DNS_IN_FLIGHT_CAP` lookup slots, given back on drop so a panic frees it too.
struct Slot(&'static AtomicUsize);

impl Slot {
    fn take(in_flight: &'static AtomicUsize) -> Option<Slot> {
        in_flight
            .try_update(Relaxed, Relaxed, |n| {
                (n < DNS_IN_FLIGHT_CAP).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(in_flight))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

/// The first IPv4 address `host` resolves to; IPv6 answers are skipped.
fn first_ipv4(host: &str) -> Option<Ipv4Addr> {
    (host, 0)
        .to_socket_addrs()
        .ok()?
        .find_map(|addr| match addr.ip() {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
}

/// Whether `ip` lies in `pattern`/`mask` as netmask 2.0.2 reads them: `mask` is either a dotted
/// address of contiguous ones (not all zeros) or a bit count from 0 to 32. Any failure is `false`.
/// The JavaScript wrapper splits a CIDR pattern given without a mask, so `"10.0.0.0/8"` matches
/// here though browsers answer false for it (see the `pac` module docs).
fn is_in_net(ip: Option<Ipv4Addr>, pattern: &str, mask: &str) -> bool {
    let mask = if mask.contains('.') {
        ip2long(mask).filter(|&m| m != 0 && m.leading_ones() + m.trailing_zeros() == 32)
    } else {
        match mask.parse::<u32>() {
            Ok(0) => Some(0),
            Ok(bits @ 1..=32) => Some(u32::MAX << (32 - bits)),
            _ => None,
        }
    };
    let (Some(ip), Some(net), Some(mask)) = (ip, ip2long(pattern), mask) else {
        return false;
    };
    ip.to_bits() & mask == net & mask
}

/// netmask's `ip2long`: one to four dot-separated numbers, the last filling the remaining bytes
/// (so `10.1` is 10.0.0.1 and `167772160` is 10.0.0.0).
fn ip2long(text: &str) -> Option<u32> {
    let mut rest = text;
    let mut parts = Vec::with_capacity(4);
    for i in 0..4 {
        if rest.is_empty() {
            break;
        }
        if i > 0 {
            rest = rest.strip_prefix('.')?;
        }
        let (n, used) = leading_number(rest)?;
        parts.push(n);
        rest = &rest[used..];
    }
    if !rest.is_empty() {
        return None;
    }
    match parts[..] {
        [a] => Some(a),
        [a, b] if a <= 0xff && b <= 0xff_ffff => Some(a << 24 | b),
        [a, b, c] if a <= 0xff && b <= 0xff && c <= 0xffff => Some(a << 24 | b << 16 | c),
        [a, b, c, d] if [a, b, c, d].iter().all(|&x| x <= 0xff) => {
            Some(a << 24 | b << 16 | c << 8 | d)
        }
        _ => None,
    }
}

/// netmask's `atob`: the number at the start of `s` (hex after `0x`, octal after a leading `0`,
/// else decimal, wrapping at 32 bits like its `>>> 0`) and the bytes it used; `None` when empty.
fn leading_number(s: &str) -> Option<(u32, usize)> {
    let b = s.as_bytes();
    let (mut i, mut base) = (0, 10);
    if b.len() > 1 && b[0] == b'0' {
        if b[1] == b'x' || b[1] == b'X' {
            (i, base) = (2, 16);
        } else if b[1].is_ascii_digit() {
            (i, base) = (1, 8);
        }
    }
    let start = i;
    let mut n: u32 = 0;
    while let Some(digit) = b.get(i).and_then(|&c| char::from(c).to_digit(base)) {
        n = n.wrapping_mul(base).wrapping_add(digit);
        i += 1;
    }
    (i > start).then_some((n, i))
}

/// The address the default route leaves from: connecting a UDP socket picks it without
/// sending anything. Loopback when there is no route.
fn my_ip_address() -> Ipv4Addr {
    let routed = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
        socket.connect((Ipv4Addr::new(8, 8, 8, 8), 53))?;
        socket.local_addr()
    });
    match routed.map(|addr| addr.ip()) {
        Ok(IpAddr::V4(ip)) if !(ip.is_loopback() || ip.is_unspecified()) => ip,
        _ => Ipv4Addr::LOCALHOST,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    fn run(body: &str) -> Result<String, EvalError> {
        let script = format!("function FindProxyForURL(url, host) {{ {body} }}");
        evaluate(&script, "http://a.test/", "a.test")
    }

    /// `String(<expr>)` evaluated inside `FindProxyForURL`.
    fn value(expr: &str) -> String {
        run(&format!("return String({expr});")).unwrap_or_else(|e| panic!("{expr}: {e:?}"))
    }

    #[test]
    fn the_answer_and_both_arguments_pass_through() {
        let script = "function FindProxyForURL(url, host) { return url + ' ' + host; }";
        assert_eq!(
            evaluate(script, "http://a.test:8080/p?q=1", "a.test"),
            Ok("http://a.test:8080/p?q=1 a.test".to_owned())
        );
        assert_eq!(run("return 42;"), Ok(String::new()));
        assert_eq!(run("return undefined;"), Ok(String::new()));
    }

    #[test]
    fn scripts_run_in_sloppy_mode() {
        assert_eq!(
            run("undeclared = 1; return 'DIRECT';"),
            Ok("DIRECT".to_owned())
        );
    }

    #[test]
    fn a_bad_top_level_or_a_missing_function_is_a_compile_failure() {
        for script in [
            "var x = 1;",
            "function FindProxyForURL(",
            "var FindProxyForURL = 3;",
            "throw 1;",
        ] {
            assert!(
                matches!(
                    evaluate(script, "http://a.test/", "a.test"),
                    Err(EvalError::Compile(_))
                ),
                "{script}"
            );
        }
        match evaluate("var x = 1;", "http://a.test/", "a.test") {
            Err(EvalError::Compile(message)) => {
                assert!(message.contains("FindProxyForURL"), "{message}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_throw_is_a_script_failure_with_its_message() {
        assert_eq!(
            run("throw new Error('boom');"),
            Err(EvalError::Script("boom".to_owned()))
        );
    }

    #[test]
    fn a_runaway_call_stops_at_one_second() {
        assert_eq!(STEP_LIMIT, Duration::from_secs(1));
        let start = Instant::now();
        // The guard turns a missed limit into a failure; an upper bound would fail under load.
        let answer = crate::hang_guard::guarded(|| {
            run("try { while (true) {} } catch (e) {} return 'DIRECT';")
        });
        assert_eq!(answer, Err(EvalError::Timeout));
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(900), "{took:?}");
    }

    #[test]
    fn each_step_gets_its_own_budget() {
        let spin = "var t = Date.now(); while (Date.now() - t < 150) {}";
        let script =
            format!("{spin} function FindProxyForURL(url, host) {{ {spin} return 'DIRECT'; }}");
        let step = Duration::from_millis(200);
        assert_eq!(
            evaluate_within(&script, "http://a.test/", "a.test", step),
            Ok("DIRECT".to_owned())
        );
        // A fifth of `STEP_LIMIT`, so a step that fell back to the default is told apart.
        let short = STEP_LIMIT / 20;
        let start = Instant::now();
        assert_eq!(
            evaluate_within("while (true) {}", "http://a.test/", "a.test", short),
            Err(EvalError::Timeout)
        );
        assert!(start.elapsed() < STEP_LIMIT, "{:?}", start.elapsed());
    }

    #[test]
    fn memory_and_stack_exhaustion_are_script_failures() {
        assert_eq!(MEMORY_LIMIT, 16 << 20);
        let bomb = run("var a = []; while (true) a.push(new Array(1e5).fill(1));");
        assert!(matches!(bomb, Err(EvalError::Script(_))), "{bomb:?}");
        let deep = run("function f() { return f() + 1; } return f();");
        assert!(matches!(deep, Err(EvalError::Script(_))), "{deep:?}");
    }

    #[test]
    fn the_sandbox_exposes_the_helpers_and_nothing_else() {
        for name in [
            "isPlainHostName",
            "dnsDomainIs",
            "localHostOrDomainIs",
            "dnsDomainLevels",
            "shExpMatch",
            "isResolvable",
            "dnsResolve",
            "isInNet",
            "myIpAddress",
            "weekdayRange",
            "timeRange",
            "dateRange",
            "alert",
        ] {
            assert_eq!(value(&format!("typeof {name}")), "function", "{name}");
        }
        for name in [
            "dnsResolveEx",
            "isInNetEx",
            "myIpAddressEx",
            "convert_addr",
            "__pacDnsResolve",
            "__pacIsInNet",
            "__pacMyIpAddress",
            "require",
            "process",
            "fetch",
            "setTimeout",
            "std",
            "os",
        ] {
            assert_eq!(value(&format!("typeof {name}")), "undefined", "{name}");
        }
    }

    #[test]
    fn pure_helpers_follow_the_library() {
        let cases = [
            ("isPlainHostName('a')", "true"),
            ("isPlainHostName('a.b')", "false"),
            ("dnsDomainIs('www.x.test', '.x.test')", "true"),
            ("dnsDomainIs('wx.test', 'x.test')", "true"),
            ("dnsDomainIs('x.test', '.x.test')", "false"),
            ("localHostOrDomainIs('www', 'www.x.test')", "true"),
            ("localHostOrDomainIs('www.x.test', 'www.x.test')", "true"),
            ("localHostOrDomainIs('www.y.test', 'www.x.test')", "false"),
            ("dnsDomainLevels('a')", "0"),
            ("dnsDomainLevels('a.b.c')", "2"),
            ("shExpMatch('http://a.test/x', '*/x')", "true"),
            ("shExpMatch('a.test', 'a?test')", "true"),
            ("shExpMatch('aXtest', 'a.test')", "false"),
            ("shExpMatch('xa.test', 'a.test')", "false"),
            ("shExpMatch('aab', 'a+b')", "true"),
            ("dateRange(1)", "false"),
            ("dateRange('JAN', 'DEC')", "false"),
            ("alert('x')", "undefined"),
            ("weekdayRange('SUN', 'SAT')", "true"),
            ("weekdayRange('SUN', 'SAT', 'GMT')", "true"),
            ("weekdayRange('XYZ')", "false"),
            ("timeRange(0, 24)", "true"),
            ("timeRange(0, 24, 'GMT')", "true"),
            ("timeRange(0, 0, 0, 23, 59, 59)", "true"),
            ("timeRange(25)", "false"),
        ];
        for (expr, want) in cases {
            assert_eq!(value(expr), want, "{expr}");
        }
    }

    #[test]
    fn dns_helpers_keep_ipv4_answers_only() {
        let cases = [
            ("dnsResolve('localhost')", "127.0.0.1"),
            ("dnsResolve('127.0.0.1')", "127.0.0.1"),
            ("dnsResolve('::1')", "null"),
            ("isResolvable('localhost')", "true"),
            ("isResolvable('::1')", "false"),
            ("isInNet('127.0.0.1', '127.0.0.0', '255.0.0.0')", "true"),
            ("isInNet('localhost', '127.0.0.0', '255.0.0.0')", "true"),
            ("isInNet('10.1.2.3', '10.1.0.0', '255.255.0.0')", "true"),
            ("isInNet('10.1.2.3', '10.2.0.0', '255.255.0.0')", "false"),
            ("isInNet('127.0.0.1', '127.0.0.0', '255.0.255.0')", "false"),
            ("isInNet('::1', '0.0.0.0', '0.0.0.0')", "false"),
            ("isInNet('127.0.0.1', 'x', '255.0.0.0')", "false"),
        ];
        for (expr, want) in cases {
            assert_eq!(value(expr), want, "{expr}");
        }
    }

    #[test]
    fn is_in_net_reads_masks_as_netmask_does() {
        // Expected values from netmask 2.0.2's Netmask(pattern, mask).contains, as
        // pac-resolver 7.0.1 calls it.
        let cases = [
            ("isInNet('10.1.2.3', '10.0.0.0', '8')", "true"),
            ("isInNet('10.1.2.3', '11.0.0.0', '8')", "false"),
            ("isInNet('10.1.2.3', '10.0.0.0', 8)", "true"),
            ("isInNet('10.1.2.3', '10.0.0.0', '8abc')", "true"),
            ("isInNet('10.1.2.3', '10.0.0.0', '33')", "false"),
            ("isInNet('10.1.2.3', '10.0.0.0', 'x')", "false"),
            ("isInNet('10.1.2.3', '10.0.0.0', '-1')", "true"),
            ("isInNet('10.1.2.3', '0.0.0.0', '0')", "true"),
            ("isInNet('10.1.2.3', '0.0.0.0', '0.0.0.0')", "false"),
            ("isInNet('10.1.2.3', '10.0.0.0', 0)", "false"),
            ("isInNet('10.1.2.3', '10.0.0.0/8')", "true"),
            ("isInNet('10.1.2.3', '11.0.0.0/8')", "false"),
            ("isInNet('10.1.2.3', '10.1.2.0/255.255.255.0')", "true"),
            ("isInNet('10.1.2.3', '10.0.0.0/8/9')", "true"),
            ("isInNet('10.1.2.3', '10.0.0.0/8', '255.0.0.0')", "false"),
            ("isInNet('10.1.2.3', '10.1.2.3')", "true"),
            ("isInNet('10.1.2.3', '10.1.2.0/')", "false"),
            ("isInNet('10.1.2.3', null, '8')", "false"),
            ("isInNet('10.1.2.3', '012.1.0.0', '255.255.0.0')", "true"),
            ("isInNet('10.1.2.3', '0x0a.1.0.0', '255.255.0.0')", "true"),
            ("isInNet('10.1.2.3', '167772160', '255.0.0.0')", "true"),
            ("isInNet('10.1.2.3', '08.0.0.0', '8')", "false"),
            ("isInNet('10.0.0.1', '10.1', '255.255.255.255')", "true"),
            ("isInNet('10.1.2.3', '10.0.0.0', '0xff.0.0.0')", "true"),
            ("isInNet('10.1.2.3', '10.1.2.0', '255.255.255')", "false"),
        ];
        for (expr, want) in cases {
            assert_eq!(value(expr), want, "{expr}");
        }
    }

    /// A resolver that answers only long after any lookup bound.
    fn stalled(_host: &str) -> Option<Ipv4Addr> {
        std::thread::sleep(Duration::from_secs(5));
        Some(Ipv4Addr::new(10, 0, 0, 1))
    }

    #[test]
    fn a_stalled_lookup_fails_within_its_bound() {
        assert!(DNS_LIMIT * 3 < STEP_LIMIT, "{DNS_LIMIT:?}");
        for (expr, want) in [
            ("dnsResolve('slow.test')", "null"),
            ("isResolvable('slow.test')", "false"),
            ("isInNet('slow.test', '10.0.0.0', '8')", "false"),
        ] {
            let script =
                format!("function FindProxyForURL(url, host) {{ return String({expr}); }}");
            static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
            let answer = evaluate_with(
                &script,
                "http://a.test/",
                "a.test",
                Duration::from_secs(30),
                stalled,
                &IN_FLIGHT,
            );
            // The step is far longer than the 5 s stall, so a lookup that waited for its resolver
            // would return an address; only giving up at `DNS_LIMIT` yields these answers.
            assert_eq!(answer, Ok(want.to_owned()), "{expr}");
        }
    }

    static PAST_DEADLINE_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn stalled_past_deadline(host: &str) -> Option<Ipv4Addr> {
        PAST_DEADLINE_CALLS.fetch_add(1, Relaxed);
        stalled(host)
    }

    #[test]
    fn no_lookup_starts_once_the_step_is_spent() {
        static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let script = "function FindProxyForURL(url, host) { \
            while (true) { try { isInNet('x.test', '10.0.0.0', '8'); } catch (e) {} } }";
        let step = DNS_LIMIT + Duration::from_millis(50);
        let answer = evaluate_with(
            script,
            "http://a.test/",
            "a.test",
            step,
            stalled_past_deadline,
            &IN_FLIGHT,
        );
        assert_eq!(answer, Err(EvalError::Timeout));
        let calls = PAST_DEADLINE_CALLS.load(Relaxed);
        assert!(calls <= 2, "{calls} lookups");
    }

    static CAPPED_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn stalled_counted(host: &str) -> Option<Ipv4Addr> {
        CAPPED_CALLS.fetch_add(1, Relaxed);
        stalled(host)
    }

    #[test]
    fn stalled_lookups_in_flight_are_capped() {
        static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
        let script = "function FindProxyForURL(url, host) { \
            for (var i = 0; i < 20; i++) isInNet('x.test', '10.0.0.0', '8'); return 'DIRECT'; }";
        let run = || {
            evaluate_with(
                script,
                "http://a.test/",
                "a.test",
                STEP_LIMIT,
                stalled_counted,
                &IN_FLIGHT,
            )
        };
        while CAPPED_CALLS.load(Relaxed) < DNS_IN_FLIGHT_CAP {
            let _ = run();
        }
        // Twenty refusals that each waited `DNS_LIMIT` would take 5 s; the result alone does not
        // show it, as the step deadline is only checked between bytecodes.
        let start = Instant::now();
        assert_eq!(run(), Ok("DIRECT".to_owned()));
        assert!(start.elapsed() < STEP_LIMIT, "{:?}", start.elapsed());
        assert_eq!(CAPPED_CALLS.load(Relaxed), DNS_IN_FLIGHT_CAP);
        assert_eq!(IN_FLIGHT.load(Relaxed), DNS_IN_FLIGHT_CAP);
    }

    #[test]
    fn my_ip_address_is_a_routable_ipv4_or_loopback() {
        let text = value("myIpAddress()");
        let ip: Ipv4Addr = text.parse().unwrap_or_else(|e| panic!("{text}: {e}"));
        assert!(
            ip == Ipv4Addr::LOCALHOST || !(ip.is_loopback() || ip.is_unspecified()),
            "{ip}"
        );
    }
}
