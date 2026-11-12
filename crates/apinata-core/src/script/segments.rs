//! Reading a request's script segments for one phase from the collection, folder and request files.

use super::ScriptPhase;
use super::engine::Segment;
use crate::context::VarContext;
use crate::request::RequestFile;

/// The non-empty scripts of `phase` above and in `req`, in the order the phase runs them.
///
/// The pre-request phase always runs outermost first; the later two run innermost first unless
/// the collection's flow is `sequential`. Unknown script types and blank code are ignored.
pub(crate) fn segments(req: &RequestFile, ctx: &VarContext, phase: ScriptPhase) -> Vec<Segment> {
    let kind = phase.script_type();
    let mut blocks = ctx.script_blocks(req);
    // Reverse whole levels, so two scripts of one level keep their written order.
    if phase != ScriptPhase::PreRequest && !ctx.sequential_flow() {
        blocks.reverse();
    }
    let mut out = Vec::new();
    for (level, block) in blocks {
        let Some(yaml_serde::Value::Sequence(items)) = block.and_then(|b| b.get("scripts")) else {
            continue;
        };
        for item in items {
            let is_kind = item.get("type").and_then(yaml_serde::Value::as_str) == Some(kind);
            let code = item.get("code").and_then(yaml_serde::Value::as_str);
            if let Some(code) = code.filter(|c| is_kind && !c.trim().is_empty()) {
                out.push(Segment {
                    level: level.clone(),
                    phase,
                    code: code.to_owned(),
                });
            }
        }
    }
    out
}
