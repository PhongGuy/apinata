//! `--save-example`: the response of a one-request run saved as an example in its request file.

use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use apinata_core::{
    Collection, ExampleResponse, ItemPath, IterationData, RequestFile, RequestResult, RunReport,
    TreeError, VarContext, iteration_total, request_secrets,
};

use crate::RunArgs;
use crate::run::Target;

/// What saving needs once the run is over, gathered before anything is sent.
pub struct ExamplePlan {
    name: String,
    keep_secrets: bool,
    file: RequestFile,
    ctx: VarContext,
    /// The run's data rows and iteration count, to rebuild the context of the saved send.
    data: Option<IterationData>,
    iterations: Option<usize>,
    root: PathBuf,
    item: ItemPath,
}

/// The plan for `args.save_example`, or `None` without it. Fails before any send when the run
/// selects other than one request, the name is blank, or the request already has that example.
pub fn plan(
    args: &RunArgs,
    target: &Target,
    data: Option<&IterationData>,
) -> anyhow::Result<Option<ExamplePlan>> {
    let Some(name) = &args.save_example else {
        return Ok(None);
    };
    let count = target.plan.requests().count();
    let (Some(file), 1) = (target.plan.requests().next(), count) else {
        bail!(
            "--save-example needs exactly one request, but {} selects {count}",
            args.target().display()
        );
    };
    let (root, item) = match &args.example_target {
        Some(into) => into.clone(),
        None => {
            let item = target
                .plan
                .items()
                .next()
                .expect("one request has one item");
            (target.collection.root().to_path_buf(), item.clone())
        }
    };
    let collection = Collection::open(&root)?;
    let name = checked_name(&collection, &item, name)?;
    Ok(Some(ExamplePlan {
        name,
        keep_secrets: args.keep_secrets,
        file: file.clone(),
        ctx: target.ctx.clone(),
        data: data.cloned(),
        iterations: args.iterations.map(|n| (n as usize).max(1)),
        root,
        item,
    }))
}

/// The trimmed name a new example at `item` gets, checked by core before the send and again
/// before the save, so a file changed during the run is checked as it is then.
fn checked_name(collection: &Collection, item: &ItemPath, name: &str) -> anyhow::Result<String> {
    collection
        .new_example_name(item, name)
        .map_err(|e| match e {
            TreeError::NotFound { .. } => anyhow!("no HTTP request at {item}"),
            TreeError::InvalidName { .. } => {
                anyhow!("--save-example needs a name that is not blank")
            }
            other => other.into(),
        })
}

/// Save the last response of `report` as `plan` says, scrubbed unless it keeps secrets.
pub fn save(plan: &ExamplePlan, report: &RunReport) -> anyhow::Result<()> {
    // Only the final result counts, so a failed last iteration never saves an older response.
    let result = report
        .results
        .last()
        .ok_or_else(|| anyhow!("no response to save as an example"))?;
    let response = (result.response.as_ref())
        .ok_or_else(|| anyhow!("no response to save as an example: the last send got none"))?;
    if !ExampleResponse::savable(response) {
        bail!(
            "the response was cut short (see --max-body-bytes) or is 5 MiB or more; no example was saved"
        );
    }
    let mut example = ExampleResponse::from_response(response);
    let mut description = String::new();
    if !plan.keep_secrets {
        let mut derived = result.derived_credentials().to_vec();
        derived.extend_from_slice(result.script_sensitive_values());
        let secrets = request_secrets(&plan.file, &plan.iteration_ctx(result), &derived).context(
            "cannot check the response for secret values, so no example was saved \
             (--keep-secrets saves it as received)",
        )?;
        let scrubbed = example.scrub(Some(&secrets));
        description = scrubbed.description("");
        if scrubbed.withheld {
            eprintln!("note: the binary body held a secret value and was left out");
        }
    }
    let mut collection = Collection::open(&plan.root)?;
    checked_name(&collection, &plan.item, &plan.name)?;
    collection.add_example(&plan.item, &plan.name, &description, &example)?;
    eprintln!(
        "saved example `{}` in {}",
        plan.name,
        plan.root.join(plan.item.as_str()).display()
    );
    Ok(())
}

impl ExamplePlan {
    /// The context the runner gave `result`'s send, so data-file variables resolve when the
    /// request is checked for secrets, with the iteration total the runner used.
    fn iteration_ctx(&self, result: &RequestResult) -> VarContext {
        let total = iteration_total(self.data.as_ref(), self.iterations);
        let index = result.iteration.unwrap_or(0);
        self.ctx.for_iteration(self.data.as_ref(), index, total)
    }
}
