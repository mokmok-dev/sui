use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc};

use rhai::{
    AST, Array, Dynamic, Engine as RhaiEngine, EvalAltResult, INT, ImmutableString, Map, Position,
    Scope,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgentOptions, AgentRequest, AgentResult, Capability, Host, Journal, JournalEntry, ParallelSlot,
    WorkflowError,
    hash::{self, ContentHash},
    journal::{
        gate_content_hash, optional_result_content_hash, panel_content_hash, result_content_hash,
        scratch_content_hash,
    },
    schema::validate_args,
    value::{dynamic_to_json, json_to_dynamic},
};

const DEFAULT_AGENT_BUDGET: usize = 128;
const MAX_AGENT_BUDGET: usize = 1024;

/// A compiled, metadata-validated workflow.
#[derive(Clone)]
pub struct CompiledWorkflow {
    source: String,
    ast: AST,
    metadata: crate::WorkflowMeta,
}

impl CompiledWorkflow {
    /// Returns the workflow's validated metadata.
    #[must_use]
    pub const fn metadata(&self) -> &crate::WorkflowMeta {
        &self.metadata
    }

    /// Returns the SHA-256 of the workflow source.
    #[must_use]
    pub fn workflow_hash(&self) -> ContentHash {
        hash::hash_str(&self.source)
    }
}

/// Inputs controlling one workflow execution.
#[derive(Clone, Debug)]
pub struct RunOptions {
    /// Verbatim JSON value exposed as the `args` global.
    pub args: Option<Value>,
    /// Previously committed journal to replay before making live host calls.
    pub journal: Journal,
    /// Maximum number of agent slots admitted by this run.
    pub agent_budget: usize,
    /// Optional durable journal checkpoint updated after every committed effect.
    pub checkpoint: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            args: None,
            journal: Journal::new(),
            agent_budget: DEFAULT_AGENT_BUDGET,
            checkpoint: None,
        }
    }
}

/// Information describing a paused workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseInfo {
    /// Machine-readable pause kind.
    pub kind: String,
    /// Human-readable pause message.
    pub message: String,
}

/// The deterministic output of a workflow execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// Validated workflow metadata.
    pub metadata: crate::WorkflowMeta,
    /// Workflow source content hash for this run.
    pub workflow_hash: ContentHash,
    /// Input (`args`) content hash for this run.
    pub input_hash: ContentHash,
    /// Value passed to `complete`, if the workflow called it.
    pub complete: Option<Value>,
    /// Pause information when execution stopped at a gate.
    pub paused: Option<PauseInfo>,
    /// Ordered messages emitted with `log`.
    pub logs: Vec<String>,
    /// Ordered phase titles passed to `phase`.
    pub phases: Vec<String>,
    /// Final committed host-call journal.
    pub journal: Journal,
}

/// Compiles and executes deterministic Rhai workflows against a host.
pub struct Engine<H> {
    host: Rc<H>,
}

impl<H> Engine<H>
where
    H: Host + 'static,
{
    /// Creates an engine backed by the supplied host implementation.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host: Rc::new(host),
        }
    }

    /// Compiles a workflow and extracts its pure-literal metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when metadata is invalid or Rhai compilation fails.
    pub fn compile(
        &self,
        script: &str,
    ) -> Result<CompiledWorkflow, WorkflowError> {
        let metadata = crate::meta::extract(script)?;
        let mut engine = RhaiEngine::new();
        configure_rhai_limits(&mut engine);
        let ast = engine
            .compile(script)
            .map_err(|error| WorkflowError::Compile(error.to_string()))?;
        Ok(CompiledWorkflow {
            source: script.to_owned(),
            ast,
            metadata,
        })
    }

    /// Executes a compiled workflow, replaying the supplied journal by content hash.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid budgets, divergent journals, host failures,
    /// unsupported values, or Rhai runtime failures.
    pub fn run(
        &self,
        workflow: &CompiledWorkflow,
        mut options: RunOptions,
    ) -> Result<RunResult, WorkflowError> {
        if !(1..=MAX_AGENT_BUDGET).contains(&options.agent_budget) {
            return Err(WorkflowError::InvalidBudget(options.agent_budget));
        }
        options.journal.validate()?;

        if let Some(schema) = &workflow.metadata.args_schema {
            validate_args(schema, options.args.as_ref())?;
        }

        let workflow_hash = workflow.workflow_hash();
        let input_value = options.args.clone().unwrap_or(Value::Null);
        let input_hash = hash::hash_json(&input_value)?;
        options
            .journal
            .bind_or_verify(workflow_hash.clone(), input_hash.clone())?;

        let args = options
            .args
            .as_ref()
            .map(json_to_dynamic)
            .transpose()?
            .unwrap_or(Dynamic::UNIT);
        let runtime = Rc::new(RefCell::new(Runtime::new(
            Rc::clone(&self.host),
            options.journal,
            options.agent_budget,
            options.checkpoint,
        )));
        let mut engine = RhaiEngine::new();
        configure_rhai_limits(&mut engine);
        register_host_api(&mut engine, &runtime);
        let mut scope = Scope::new();
        scope.push_dynamic("args", args);
        let evaluation = engine.eval_ast_with_scope::<Dynamic>(&mut scope, &workflow.ast);

        let runtime = runtime.borrow();
        if let Some(message) = &runtime.divergence {
            return Err(WorkflowError::JournalDivergence(message.clone()));
        }
        if let Some(failure) = &runtime.capability_failure {
            return Err(failure.to_error());
        }
        if runtime.cursor != runtime.journal.entries().len() {
            return Err(WorkflowError::JournalDivergence(format!(
                "execution consumed {} of {} committed entries",
                runtime.cursor,
                runtime.journal.entries().len()
            )));
        }
        if let Err(error) = evaluation
            && runtime.complete.is_none()
            && runtime.paused.is_none()
        {
            return Err(WorkflowError::Runtime(error.to_string()));
        }
        Ok(RunResult {
            metadata: workflow.metadata.clone(),
            workflow_hash,
            input_hash,
            complete: runtime.complete.clone(),
            paused: runtime.paused.clone(),
            logs: runtime.logs.clone(),
            phases: runtime.phases.clone(),
            journal: runtime.journal.clone(),
        })
    }

    /// Compiles and executes a workflow in one operation.
    ///
    /// # Errors
    ///
    /// Returns any compilation or execution error.
    pub fn run_script(
        &self,
        script: &str,
        options: RunOptions,
    ) -> Result<RunResult, WorkflowError> {
        let workflow = self.compile(script)?;
        self.run(&workflow, options)
    }
}

#[derive(Clone, Debug)]
enum CapabilityFailure {
    Denied {
        requested: Capability,
        granted: Capability,
    },
    InvalidMode(String),
}

impl CapabilityFailure {
    fn to_error(&self) -> WorkflowError {
        match self {
            Self::Denied { requested, granted } => WorkflowError::CapabilityDenied {
                requested: *requested,
                granted: *granted,
            },
            Self::InvalidMode(mode) => WorkflowError::InvalidCapabilityMode(mode.clone()),
        }
    }
}

struct Runtime<H> {
    host: Rc<H>,
    journal: Journal,
    cursor: usize,
    budget: usize,
    spent: usize,
    divergence: Option<String>,
    capability_failure: Option<CapabilityFailure>,
    complete: Option<Value>,
    paused: Option<PauseInfo>,
    logs: Vec<String>,
    phases: Vec<String>,
    scratch: BTreeMap<String, String>,
    checkpoint: Option<PathBuf>,
}

impl<H> Runtime<H>
where
    H: Host,
{
    const fn new(
        host: Rc<H>,
        journal: Journal,
        budget: usize,
        checkpoint: Option<PathBuf>,
    ) -> Self {
        Self {
            host,
            journal,
            cursor: 0,
            budget,
            spent: 0,
            divergence: None,
            capability_failure: None,
            complete: None,
            paused: None,
            logs: Vec::new(),
            phases: Vec::new(),
            scratch: BTreeMap::new(),
            checkpoint,
        }
    }

    fn admit(
        &mut self,
        slots: usize,
    ) -> Result<(), WorkflowError> {
        let next = self
            .spent
            .checked_add(slots)
            .ok_or_else(|| WorkflowError::Runtime("agent budget accounting overflowed".into()))?;
        if next > self.budget {
            return Err(WorkflowError::Runtime(format!(
                "agent budget exceeded: requested {slots} slot(s) with {} remaining",
                self.budget.saturating_sub(self.spent)
            )));
        }
        self.spent = next;
        Ok(())
    }

    fn run_agent(
        &mut self,
        request: AgentRequest,
    ) -> Result<AgentResult, WorkflowError> {
        self.check_capability(&request)?;
        let request_hash = request.content_hash()?;
        let invocation = self.cursor;
        if let Some(entry) = self.journal.get(invocation).cloned() {
            let JournalEntry::Agent {
                request_hash: recorded_hash,
                result,
                ..
            } = entry
            else {
                return Err(self.wrong_entry("agent"));
            };
            if recorded_hash != request_hash {
                return Err(self.diverge(format!(
                    "agent request_hash at invocation {invocation} changed"
                )));
            }
            self.admit(1)?;
            self.cursor += 1;
            return Ok(*result);
        }

        self.admit(1)?;
        let result = self
            .host
            .run_agent(&request)
            .map_err(|error| WorkflowError::Runtime(format!("host agent failure: {error}")))?;
        let result_hash = result_content_hash(&result)?;
        self.journal.push(JournalEntry::Agent {
            invocation,
            request_hash,
            result_hash,
            request: Box::new(request),
            result: Box::new(result.clone()),
        });
        if let Err(error) = self.checkpoint() {
            self.journal.pop();
            return Err(error);
        }
        self.cursor += 1;
        Ok(result)
    }

    fn run_parallel(
        &mut self,
        requests: &[AgentRequest],
    ) -> Result<Vec<Option<AgentResult>>, WorkflowError> {
        for request in requests {
            self.check_capability(request)?;
        }
        let panel_hash = panel_content_hash(requests)?;
        let invocation = self.cursor;
        let slots = if let Some(entry) = self.journal.get(invocation).cloned() {
            let JournalEntry::Parallel {
                panel_hash: recorded_hash,
                slots,
                requests: recorded_requests,
                ..
            } = entry
            else {
                return Err(self.wrong_entry("parallel"));
            };
            if recorded_hash != panel_hash {
                return Err(self.diverge(format!(
                    "parallel panel_hash at invocation {invocation} changed"
                )));
            }
            if slots.len() != requests.len() || recorded_requests.len() != requests.len() {
                return Err(self.diverge(format!(
                    "parallel entry {invocation} has mismatched request/slot cardinality"
                )));
            }
            slots
        } else {
            vec![ParallelSlot::Pending; requests.len()]
        };

        self.admit(requests.len())?;
        if self.journal.get(invocation).is_none() {
            self.journal.push(JournalEntry::Parallel {
                invocation,
                panel_hash,
                requests: requests.to_vec(),
                slots: slots.clone(),
            });
            if let Err(error) = self.checkpoint() {
                self.journal.pop();
                return Err(error);
            }
        }

        let mut results = Vec::with_capacity(requests.len());
        for (index, (request, slot)) in requests.iter().zip(slots).enumerate() {
            let result = match slot {
                ParallelSlot::Pending => {
                    // Host infrastructure errors become soft failures for one
                    // panel slot (Completed { result: None }); serial `agent`
                    // still aborts the run. Callers that need fail-fast panels
                    // should check for unit results in Rhai.
                    let result = self.host.run_agent(request).ok();
                    self.commit_parallel_slot(invocation, index, result.clone())?;
                    result
                },
                ParallelSlot::Completed { result, .. } => result,
            };
            results.push(result);
        }
        self.cursor += 1;
        Ok(results)
    }

    fn commit_parallel_slot(
        &mut self,
        invocation: usize,
        index: usize,
        result: Option<AgentResult>,
    ) -> Result<(), WorkflowError> {
        let result_hash = optional_result_content_hash(result.as_ref())?;
        let previous = {
            let Some(JournalEntry::Parallel { slots, .. }) = self.journal.get_mut(invocation)
            else {
                return Err(self.diverge(format!(
                    "parallel entry {invocation} disappeared while committing slot {index}"
                )));
            };
            let Some(slot) = slots.get_mut(index) else {
                return Err(
                    self.diverge(format!("parallel entry {invocation} has no slot {index}"))
                );
            };
            let previous = slot.clone();
            *slot = ParallelSlot::Completed {
                result_hash,
                result,
            };
            previous
        };
        if let Err(error) = self.checkpoint() {
            if let Some(JournalEntry::Parallel { slots, .. }) = self.journal.get_mut(invocation)
                && let Some(slot) = slots.get_mut(index)
            {
                *slot = previous;
            }
            return Err(error);
        }
        Ok(())
    }

    fn write_scratch(
        &mut self,
        name: String,
        content: String,
    ) -> Result<(), WorkflowError> {
        validate_scratch_name(&name)?;
        let content_hash = scratch_content_hash(&name, &content)?;
        let invocation = self.cursor;
        if let Some(entry) = self.journal.get(invocation).cloned() {
            let JournalEntry::WriteScratch {
                content_hash: recorded_hash,
                ..
            } = entry
            else {
                return Err(self.wrong_entry("write_scratch_file"));
            };
            if recorded_hash != content_hash {
                return Err(
                    self.diverge(format!("scratch write at invocation {invocation} changed"))
                );
            }
        } else {
            self.journal.push(JournalEntry::WriteScratch {
                invocation,
                content_hash,
                name: name.clone(),
                content: content.clone(),
            });
            if let Err(error) = self.checkpoint() {
                self.journal.pop();
                return Err(error);
            }
        }
        self.scratch.insert(name, content);
        self.cursor += 1;
        Ok(())
    }

    fn read_scratch(
        &mut self,
        name: String,
    ) -> Result<String, WorkflowError> {
        validate_scratch_name(&name)?;
        let invocation = self.cursor;
        if let Some(entry) = self.journal.get(invocation).cloned() {
            let JournalEntry::ReadScratch {
                content_hash: recorded_hash,
                name: recorded_name,
                content,
                ..
            } = entry
            else {
                return Err(self.wrong_entry("read_scratch_file"));
            };
            if recorded_name != name {
                return Err(
                    self.diverge(format!("scratch read at invocation {invocation} changed"))
                );
            }
            let current_hash = scratch_content_hash(&name, &content)?;
            if recorded_hash != current_hash {
                return Err(self.diverge(format!(
                    "scratch content hash at invocation {invocation} changed"
                )));
            }
            match self.scratch.get(&name) {
                Some(current) if current == &content => {},
                Some(_) => {
                    return Err(self.diverge(format!(
                        "scratch content at invocation {invocation} changed"
                    )));
                },
                None => {
                    return Err(self.diverge(format!(
                        "scratch file `{name}` was not recreated before replayed read"
                    )));
                },
            }
            self.cursor += 1;
            return Ok(content);
        }

        let content = self.scratch.get(&name).cloned().ok_or_else(|| {
            WorkflowError::Runtime(format!("scratch file `{name}` does not exist"))
        })?;
        let content_hash = scratch_content_hash(&name, &content)?;
        self.journal.push(JournalEntry::ReadScratch {
            invocation,
            content_hash,
            name,
            content: content.clone(),
        });
        if let Err(error) = self.checkpoint() {
            self.journal.pop();
            return Err(error);
        }
        self.cursor += 1;
        Ok(content)
    }

    fn await_user(
        &mut self,
        kind: String,
        message: String,
    ) -> Result<bool, WorkflowError> {
        let gate_hash = gate_content_hash(&kind, &message)?;
        let invocation = self.cursor;
        if let Some(entry) = self.journal.get(invocation).cloned() {
            let JournalEntry::AwaitUser {
                gate_hash: recorded_hash,
                ..
            } = entry
            else {
                return Err(self.wrong_entry("await_user"));
            };
            if recorded_hash != gate_hash {
                return Err(self.diverge(format!("user gate at invocation {invocation} changed")));
            }
            self.cursor += 1;
            return Ok(false);
        }

        self.journal.push(JournalEntry::AwaitUser {
            invocation,
            gate_hash,
            kind: kind.clone(),
            message: message.clone(),
        });
        if let Err(error) = self.checkpoint() {
            self.journal.pop();
            return Err(error);
        }
        self.cursor += 1;
        self.paused = Some(PauseInfo { kind, message });
        Ok(true)
    }

    fn checkpoint(&self) -> Result<(), WorkflowError> {
        self.checkpoint
            .as_ref()
            .map_or(Ok(()), |path| self.journal.write_atomic(path))
    }

    fn check_capability(
        &mut self,
        request: &AgentRequest,
    ) -> Result<(), WorkflowError> {
        let requested = match request.options.capability_mode.as_deref() {
            None => Capability::default(),
            Some(mode) => mode.parse::<Capability>().map_err(|_| {
                self.fail_capability(&CapabilityFailure::InvalidMode(mode.to_owned()))
            })?,
        };
        let granted = self.host.granted_capability();
        if requested > granted {
            return Err(self.fail_capability(&CapabilityFailure::Denied { requested, granted }));
        }
        Ok(())
    }

    fn fail_capability(
        &mut self,
        failure: &CapabilityFailure,
    ) -> WorkflowError {
        if self.capability_failure.is_none() {
            self.capability_failure = Some(failure.clone());
        }
        failure.to_error()
    }

    fn wrong_entry(
        &mut self,
        expected: &str,
    ) -> WorkflowError {
        self.diverge(format!(
            "expected {expected} at invocation {}, found a different call",
            self.cursor
        ))
    }

    fn diverge(
        &mut self,
        message: String,
    ) -> WorkflowError {
        if self.divergence.is_none() {
            self.divergence = Some(message.clone());
        }
        WorkflowError::JournalDivergence(message)
    }
}

#[allow(clippy::too_many_lines)]
fn register_host_api<H>(
    engine: &mut RhaiEngine,
    runtime: &Rc<RefCell<Runtime<H>>>,
) where
    H: Host + 'static,
{
    let state = Rc::clone(runtime);
    engine.register_fn(
        "agent",
        move |prompt: ImmutableString| -> Result<Map, Box<EvalAltResult>> {
            call_agent(&state, &prompt, &Map::new())
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn(
        "agent",
        move |prompt: ImmutableString, options: Map| -> Result<Map, Box<EvalAltResult>> {
            call_agent(&state, &prompt, &options)
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn(
        "parallel",
        move |items: Array| -> Result<Array, Box<EvalAltResult>> {
            let requests = items
                .iter()
                .map(parallel_request_from_dynamic)
                .collect::<Result<Vec<_>, _>>()?;
            let results = state
                .borrow_mut()
                .run_parallel(&requests)
                .map_err(|error| runtime_error(&error))?;
            results
                .iter()
                .map(|result| {
                    result.as_ref().map_or_else(
                        || Ok(Dynamic::UNIT),
                        |result| agent_result_to_map(result).map(Dynamic::from_map),
                    )
                })
                .collect()
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn("phase", move |title: ImmutableString| {
        state.borrow_mut().phases.push(title.to_string());
    });

    let state = Rc::clone(runtime);
    engine.register_fn("log", move |message: ImmutableString| {
        state.borrow_mut().logs.push(message.to_string());
    });

    let state = Rc::clone(runtime);
    engine.register_fn(
        "complete",
        move |value: Dynamic| -> Result<Dynamic, Box<EvalAltResult>> {
            let value = dynamic_to_json(&value).map_err(|error| runtime_error(&error))?;
            state.borrow_mut().complete = Some(value);
            Err(terminated("workflow completed"))
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn(
        "await_user",
        move |kind: ImmutableString,
              message: ImmutableString|
              -> Result<Dynamic, Box<EvalAltResult>> {
            if state
                .borrow_mut()
                .await_user(kind.to_string(), message.to_string())
                .map_err(|error| runtime_error(&error))?
            {
                Err(terminated("workflow awaiting user"))
            } else {
                Ok(Dynamic::UNIT)
            }
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn("budget", move || -> Result<Map, Box<EvalAltResult>> {
        budget_map(&state.borrow())
    });

    engine.register_fn(
        "json_encode",
        |value: Dynamic| -> Result<ImmutableString, Box<EvalAltResult>> {
            let value = dynamic_to_json(&value).map_err(|error| runtime_error(&error))?;
            serde_json::to_string(&value)
                .map(Into::into)
                .map_err(|error| runtime_error(&WorkflowError::Json(error)))
        },
    );

    engine.register_fn("fingerprint", |text: ImmutableString| -> ImmutableString {
        hash::hash_str(&text).as_str().into()
    });

    let state = Rc::clone(runtime);
    engine.register_fn(
        "write_scratch_file",
        move |name: ImmutableString, content: ImmutableString| -> Result<(), Box<EvalAltResult>> {
            state
                .borrow_mut()
                .write_scratch(name.to_string(), content.to_string())
                .map_err(|error| runtime_error(&error))
        },
    );

    let state = Rc::clone(runtime);
    engine.register_fn(
        "read_scratch_file",
        move |name: ImmutableString| -> Result<ImmutableString, Box<EvalAltResult>> {
            state
                .borrow_mut()
                .read_scratch(name.to_string())
                .map(Into::into)
                .map_err(|error| runtime_error(&error))
        },
    );

    register_determinism_guards(engine);
}

fn call_agent<H>(
    state: &Rc<RefCell<Runtime<H>>>,
    prompt: &ImmutableString,
    options: &Map,
) -> Result<Map, Box<EvalAltResult>>
where
    H: Host,
{
    let request = AgentRequest {
        prompt: prompt.to_string(),
        options: agent_options_from_map(options)?,
    };
    let result = state
        .borrow_mut()
        .run_agent(request)
        .map_err(|error| runtime_error(&error))?;
    agent_result_to_map(&result)
}

fn agent_options_from_map(options: &Map) -> Result<AgentOptions, Box<EvalAltResult>> {
    Ok(AgentOptions {
        label: optional_string(options, "label")?,
        phase: optional_string(options, "phase")?,
        capability_mode: optional_string(options, "capability_mode")?,
        output_schema: options
            .get("output_schema")
            .map(dynamic_to_json)
            .transpose()
            .map_err(|error| runtime_error(&error))?,
        agent_type: optional_string(options, "agent_type")?,
        model: optional_string(options, "model")?,
    })
}

fn optional_string(
    options: &Map,
    key: &str,
) -> Result<Option<String>, Box<EvalAltResult>> {
    let Some(value) = options.get(key) else {
        return Ok(None);
    };
    if !value.is::<ImmutableString>() {
        return Err(runtime_message(format!(
            "agent option `{key}` must be a string"
        )));
    }
    Ok(Some(value.clone_cast::<ImmutableString>().to_string()))
}

fn parallel_request_from_dynamic(value: &Dynamic) -> Result<AgentRequest, Box<EvalAltResult>> {
    if !value.is::<Map>() {
        return Err(runtime_message("parallel items must be maps"));
    }
    let map = value.clone_cast::<Map>();
    let prompt = map
        .get("prompt")
        .ok_or_else(|| runtime_message("parallel item is missing `prompt`"))?;
    if !prompt.is::<ImmutableString>() {
        return Err(runtime_message("parallel item `prompt` must be a string"));
    }
    Ok(AgentRequest {
        prompt: prompt.clone_cast::<ImmutableString>().to_string(),
        options: agent_options_from_map(&map)?,
    })
}

fn agent_result_to_map(result: &AgentResult) -> Result<Map, Box<EvalAltResult>> {
    let tokens = INT::try_from(result.tokens_used)
        .map_err(|_| runtime_message("host tokens_used exceeds Rhai's integer range"))?;
    let duration = INT::try_from(result.duration_ms)
        .map_err(|_| runtime_message("host duration_ms exceeds Rhai's integer range"))?;
    Ok(Map::from_iter([
        ("agent_id".into(), Dynamic::from(result.agent_id.clone())),
        ("success".into(), Dynamic::from_bool(result.success)),
        ("output".into(), Dynamic::from(result.output.clone())),
        ("cancelled".into(), Dynamic::from_bool(result.cancelled)),
        ("tokens_used".into(), Dynamic::from_int(tokens)),
        ("duration_ms".into(), Dynamic::from_int(duration)),
    ]))
}

fn budget_map<H>(runtime: &Runtime<H>) -> Result<Map, Box<EvalAltResult>> {
    let total = INT::try_from(runtime.budget)
        .map_err(|_| runtime_message("agent budget exceeds Rhai's integer range"))?;
    let spent = INT::try_from(runtime.spent)
        .map_err(|_| runtime_message("spent budget exceeds Rhai's integer range"))?;
    let remaining = INT::try_from(runtime.budget.saturating_sub(runtime.spent))
        .map_err(|_| runtime_message("remaining budget exceeds Rhai's integer range"))?;
    Ok(Map::from_iter([
        ("total".into(), Dynamic::from_int(total)),
        ("spent".into(), Dynamic::from_int(spent)),
        ("reserved".into(), Dynamic::from_int(0)),
        ("remaining".into(), Dynamic::from_int(remaining)),
    ]))
}

fn configure_rhai_limits(engine: &mut RhaiEngine) {
    engine.set_max_operations(100_000);
    engine.set_max_call_levels(64);
    engine.set_max_string_size(1_048_576);
    engine.set_max_array_size(65_536);
    engine.set_max_map_size(65_536);
}

fn validate_scratch_name(name: &str) -> Result<(), WorkflowError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(WorkflowError::Runtime(format!(
            "invalid scratch file name `{name}`"
        )));
    }
    Ok(())
}

fn register_determinism_guards(engine: &mut RhaiEngine) {
    engine.register_fn("timestamp", || -> Result<Dynamic, Box<EvalAltResult>> {
        Err(runtime_message(
            "timestamp() is unavailable in deterministic workflows",
        ))
    });
    engine.register_fn(
        "sleep",
        |_milliseconds: INT| -> Result<(), Box<EvalAltResult>> {
            Err(runtime_message(
                "sleep() is unavailable in deterministic workflows",
            ))
        },
    );
    engine.register_fn("rand", || -> Result<Dynamic, Box<EvalAltResult>> {
        Err(runtime_message(
            "rand() is unavailable in deterministic workflows",
        ))
    });
    engine.register_fn("random", || -> Result<Dynamic, Box<EvalAltResult>> {
        Err(runtime_message(
            "random() is unavailable in deterministic workflows",
        ))
    });
    engine.register_fn(
        "rand_int",
        |_lower: INT, _upper: INT| -> Result<INT, Box<EvalAltResult>> {
            Err(runtime_message(
                "rand_int() is unavailable in deterministic workflows",
            ))
        },
    );
}

#[allow(clippy::unnecessary_box_returns)]
fn runtime_error(error: &WorkflowError) -> Box<EvalAltResult> {
    runtime_message(error.to_string())
}

#[allow(clippy::unnecessary_box_returns)]
fn runtime_message(message: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        Dynamic::from(message.into()),
        Position::NONE,
    ))
}

#[allow(clippy::unnecessary_box_returns)]
fn terminated(message: &str) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorTerminated(
        Dynamic::from(message.to_owned()),
        Position::NONE,
    ))
}
