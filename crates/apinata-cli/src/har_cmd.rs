//! `--har FILE`: every exchange a run sent, written as one HAR 1.2 log the way the app's
//! export writes a send: the request as sent, secrets and credentials `[redacted]`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Context as _;
use apinata_core::export::{HarExchange, har_log, sent_exchange, write_atomic};
use apinata_core::{
    ItemPath, IterationData, RequestFile, RequestResult, RunReport, VarContext, iteration_total,
    request_secrets,
};

use crate::RunArgs;
use crate::run::Target;

/// What writing the log needs once the run is over, gathered before anything is sent.
pub struct HarPlan {
    path: PathBuf,
    files: HashMap<ItemPath, RequestFile>,
    ctx: VarContext,
    data: Option<IterationData>,
    iterations: Option<usize>,
    /// When each result ended, keyed by its allocation, as the finished event saw it.
    ended: Arc<Mutex<HashMap<usize, SystemTime>>>,
}

/// The plan for `args.har`, or `None` without it.
pub fn plan(args: &RunArgs, target: &Target, data: Option<&IterationData>) -> Option<HarPlan> {
    let path = args.har.clone()?;
    let files = target
        .plan
        .items()
        .cloned()
        .zip(target.plan.requests().cloned())
        .collect();
    Some(HarPlan {
        path,
        files,
        ctx: target.ctx.clone(),
        data: data.cloned(),
        iterations: args.iterations.map(|n| (n as usize).max(1)),
        ended: Arc::default(),
    })
}

impl HarPlan {
    /// A recorder for the finished event, which notes when each result ended.
    pub fn recorder(&self) -> impl Fn(&Arc<RequestResult>) + Send + use<> {
        let ended = Arc::clone(&self.ended);
        move |result| {
            let mut ended = ended.lock().unwrap_or_else(|e| e.into_inner());
            ended.insert(Arc::as_ptr(result) as usize, SystemTime::now());
        }
    }

    /// Writes every result of `report` that got a response. A send that cannot be written
    /// safely is left out with a note on stderr; failing to write the file is an error.
    pub fn write(&self, report: &RunReport) -> anyhow::Result<()> {
        let ended = self.ended.lock().unwrap_or_else(|e| e.into_inner());
        let mut exchanges = Vec::new();
        for result in &report.results {
            let Some(response) = &result.response else {
                continue;
            };
            let at = ended
                .get(&(Arc::as_ptr(result) as usize))
                .copied()
                .unwrap_or_else(SystemTime::now);
            match self.exchange(result, response, at) {
                Ok(x) => exchanges.push(x),
                Err(why) => eprintln!("note: {} is left out of the HAR: {why}", result.path),
            }
        }
        write_atomic(&self.path, har_log(&exchanges).as_bytes())
            .with_context(|| format!("cannot write the HAR file {}", self.path.display()))
    }

    fn exchange(
        &self,
        result: &RequestResult,
        response: &apinata_core::Response,
        ended: SystemTime,
    ) -> Result<HarExchange, String> {
        let file = self
            .files
            .get(&result.path)
            .ok_or("the request is not in the run")?;
        let mut derived = result.derived_credentials().to_vec();
        derived.extend_from_slice(result.script_sensitive_values());
        let total = iteration_total(self.data.as_ref(), self.iterations);
        let ctx =
            (self.ctx).for_iteration(self.data.as_ref(), result.iteration.unwrap_or(0), total);
        let secrets = request_secrets(file, &ctx, &derived)
            .map_err(|e| format!("cannot check it for secret values: {e}"))?;
        sent_exchange(response, ended, &secrets).map_err(|e| e.to_string())
    }
}
