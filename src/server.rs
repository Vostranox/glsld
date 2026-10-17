mod diagnostics;
mod notifications;
mod requests;
mod worker;

use diagnostics::Diagnostics;
use worker::{Batch, ValidatorWorker};

use crate::{protocol, validate::Validation, workspace::Workspace};
use serde_json::{json, Value};
use std::{
    io::{self, BufReader, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(40);
const EDIT_DEBOUNCE: Duration = Duration::from_millis(180);
const WORKER_RETRY_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    AwaitingInitialize,
    Running,
    Shutdown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValidationTrigger {
    Edit,
    Immediate,
}

impl ValidationTrigger {
    fn delay(self) -> Duration {
        match self {
            Self::Edit => EDIT_DEBOUNCE,
            Self::Immediate => Duration::ZERO,
        }
    }
}

enum Event {
    Message(Vec<u8>),
    Closed,
    ReadError(String),
    Validated(u64, Vec<Validation>),
}
fn spawn_input_reader(input: mpsc::SyncSender<Event>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        loop {
            let event = match protocol::read_message(&mut reader) {
                Ok(Some(body)) => Event::Message(body),
                Ok(None) => {
                    let _ = input.send(Event::Closed);
                    break;
                }
                Err(error) => {
                    let _ = input.send(Event::ReadError(error.to_string()));
                    break;
                }
            };
            if input.send(event).is_err() {
                break;
            }
        }
    });
}

pub fn run(config: Option<PathBuf>) -> Result<i32, String> {
    let (events, receiver) = mpsc::sync_channel(16);
    spawn_input_reader(events.clone());
    let revision = Arc::new(AtomicU64::new(0));
    let worker = ValidatorWorker::spawn(events, revision.clone());
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut state = State::new(config);
    let mut deadline = None;
    let mut force = false;
    loop {
        match receiver.recv_timeout(EVENT_POLL_INTERVAL) {
            Ok(Event::Message(body)) => {
                let message: Value = match serde_json::from_slice(&body) {
                    Ok(value) => value,
                    Err(_) => {
                        send_error(&mut writer, Value::Null, -32700, "Invalid JSON")?;
                        continue;
                    }
                };
                if !message.is_object() || message["jsonrpc"] != "2.0" {
                    send_error(
                        &mut writer,
                        message.get("id").cloned().unwrap_or(Value::Null),
                        -32600,
                        "Invalid JSON-RPC request",
                    )?;
                    continue;
                }
                let Some(method) = message["method"].as_str() else {
                    continue;
                };
                let params = &message["params"];
                let id = message.get("id").cloned();
                if method == "exit" {
                    return Ok(state.exit_code());
                }
                if let Some(id) = id {
                    if !id.is_string() && !id.is_number() && !id.is_null() {
                        send_error(&mut writer, Value::Null, -32600, "Invalid request id")?;
                        continue;
                    }
                    let response = state.request(method, params);
                    match response {
                        Ok(result) => send(
                            &mut writer,
                            json!({"jsonrpc":"2.0","id":id,"result":result}),
                        )?,
                        Err(error) => send_error(&mut writer, id, error.code, &error.message)?,
                    }
                    if state.lifecycle == Lifecycle::Shutdown {
                        revision.fetch_add(1, Ordering::Relaxed);
                        deadline = None;
                    }
                } else if state.lifecycle != Lifecycle::Shutdown
                    && (state.lifecycle == Lifecycle::Running || method == "initialized")
                {
                    match state.notification(method, params) {
                        Ok(Some(trigger)) => {
                            revision.fetch_add(1, Ordering::Relaxed);
                            deadline = Some(Instant::now() + trigger.delay());
                            force |= trigger == ValidationTrigger::Immediate;
                            if method == "textDocument/didClose" {
                                state.diagnostics.publish(&state.workspace, &mut writer)?;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => send(
                            &mut writer,
                            json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":2,"message":error}}),
                        )?,
                    }
                }
            }
            Ok(Event::Validated(generation, results)) => {
                if generation != revision.load(Ordering::Relaxed)
                    || state.lifecycle == Lifecycle::Shutdown
                {
                    continue;
                }
                state
                    .diagnostics
                    .accept(&state.workspace, results, &mut writer)?;
                state.diagnostics.publish(&state.workspace, &mut writer)?;
            }
            Ok(Event::Closed) => return Ok(state.exit_code()),
            Ok(Event::ReadError(error)) => return Err(error),
            Err(RecvTimeoutError::Disconnected) => return Ok(1),
            Err(RecvTimeoutError::Timeout) => {}
        }
        if deadline.is_some_and(|d| Instant::now() >= d) && state.lifecycle != Lifecycle::Shutdown {
            let graphs = state
                .workspace
                .open
                .values()
                .filter_map(|source| {
                    let graph = state.workspace.graph(&source.uri)?;
                    (force || graph.config.config.glslang.on_change).then_some(graph)
                })
                .collect();
            if worker.try_submit(Batch {
                revision: revision.load(Ordering::Relaxed),
                graphs,
            })? {
                deadline = None;
                force = false;
            } else {
                deadline = Some(Instant::now() + WORKER_RETRY_INTERVAL);
            }
        }
    }
}

struct State {
    workspace: Workspace,
    lifecycle: Lifecycle,
    hierarchical_symbols: bool,
    diagnostics: Diagnostics,
}

impl State {
    fn new(config: Option<PathBuf>) -> Self {
        Self {
            workspace: Workspace {
                explicit_config: config,
                ..Workspace::default()
            },
            lifecycle: Lifecycle::AwaitingInitialize,
            hierarchical_symbols: false,
            diagnostics: Diagnostics::default(),
        }
    }

    fn exit_code(&self) -> i32 {
        i32::from(self.lifecycle != Lifecycle::Shutdown)
    }
}

fn send(writer: &mut impl Write, value: Value) -> Result<(), String> {
    protocol::write_message(writer, &value).map_err(|e| e.to_string())
}
fn send_error(writer: &mut impl Write, id: Value, code: i32, message: &str) -> Result<(), String> {
    send(
        writer,
        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
    )
}
