use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use colored::Colorize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracekit_ingest::{self as ingest};

use super::parse_agents;

#[derive(Args)]
pub struct CaptureArgs {
    #[command(subcommand)]
    pub subcommand: CaptureSubcommand,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum InspectMode {
    Analysis,
    Forensic,
}

#[derive(Subcommand)]
pub enum CaptureSubcommand {
    /// Discover all available sessions
    All {
        /// Agent filter: claude, opencode, codex, all
        #[arg(long, default_value = "all")]
        agent: String,
    },
    /// Discover the N most recent sessions
    Recent {
        /// Agent filter
        #[arg(long, default_value = "all")]
        agent: String,
        /// Maximum number of sessions to list
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show details for a single session
    Session {
        /// Agent name
        #[arg(long, default_value = "all")]
        agent: String,
        /// Session ID (prefix match)
        #[arg(long)]
        session_id: String,
        /// Generate a cleaned inspect file for this session
        #[arg(long, default_value_t = false)]
        inspect_file: bool,
        /// Print cleaned inspect output to terminal
        #[arg(long, default_value_t = false)]
        inspect_terminal: bool,
        /// Optional output file path for inspect file
        #[arg(long)]
        inspect_out: Option<PathBuf>,
        /// Inspect rendering mode: analysis (deduped/noise-reduced) or forensic (full)
        #[arg(long, value_enum, default_value_t = InspectMode::Analysis)]
        inspect_mode: InspectMode,
    },
}

pub fn run(args: CaptureArgs) -> Result<()> {
    match args.subcommand {
        CaptureSubcommand::All { agent } => {
            let agents = parse_agents(&agent)?;
            let sessions = ingest::discover_sessions(&agents, None, None, None, None)?;
            println!("{} Discovered {} sessions", "✓".green(), sessions.len());
            for s in &sessions {
                println!("  {} {}", s.source_agent.to_string().cyan(), s.session_id);
            }
        }
        CaptureSubcommand::Recent { agent, limit } => {
            let agents = parse_agents(&agent)?;
            let sessions = ingest::discover_sessions(&agents, None, None, None, Some(limit))?;
            println!("{} Found {} recent sessions", "✓".green(), sessions.len());
            for s in &sessions {
                println!(
                    "  {} {}  {}",
                    s.source_agent.to_string().cyan(),
                    s.session_id,
                    s.cwd.as_deref().unwrap_or("-").dimmed(),
                );
            }
        }
        CaptureSubcommand::Session {
            agent,
            session_id,
            inspect_file,
            inspect_terminal,
            inspect_out,
            inspect_mode,
        } => {
            let agents = parse_agents(&agent)?;
            match ingest::find_session(&session_id, &agents)? {
                Some(s) => {
                    println!("{} Found session", "✓".green());
                    println!("  Agent    : {}", s.source_agent.to_string().cyan());
                    println!("  ID       : {}", s.session_id);
                    println!("  Path     : {}", s.source_path.display());
                    println!("  CWD      : {}", s.cwd.as_deref().unwrap_or("-"));
                    println!(
                        "  Started  : {}",
                        s.started_at
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "-".to_string())
                    );

                    let write_inspect = inspect_file || inspect_out.is_some();
                    if write_inspect || inspect_terminal {
                        let entries = build_inspect_entries(&s)?;
                        let transformed = transform_inspect_entries(&entries, inspect_mode);

                        if write_inspect {
                            let out_path =
                                inspect_out.unwrap_or_else(|| default_inspect_path(&s.session_id));
                            let markdown = render_inspect_markdown(&s, &transformed, inspect_mode);
                            if let Some(parent) = out_path.parent() {
                                if !parent.as_os_str().is_empty() {
                                    std::fs::create_dir_all(parent)?;
                                }
                            }
                            std::fs::write(&out_path, markdown)?;
                            println!("{} Inspect file: {}", "✓".green(), out_path.display());
                        }

                        if inspect_terminal {
                            println!();
                            print_inspect_terminal(&s, &transformed, inspect_mode);
                        }
                    }
                }
                None => println!("{} No session found matching '{}'", "✗".red(), session_id),
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct InspectEntry {
    ts: Option<String>,
    label: String,
    title: String,
    body: Option<String>,
    source_type: String,
    metadata: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct InspectSummary {
    raw_entries: usize,
    rendered_entries: usize,
    dropped_noise: usize,
    dropped_duplicates: usize,
    tool_calls: usize,
    tool_results: usize,
    tool_errors: usize,
    labels: Vec<(String, usize)>,
}

#[derive(Debug, Clone)]
struct InspectRender {
    entries: Vec<InspectEntry>,
    summary: InspectSummary,
}

fn default_inspect_path(session_id: &str) -> PathBuf {
    PathBuf::from("inspect-traces").join(format!("tracekit-inspect-{}.md", session_id))
}

fn build_inspect_entries(session: &tracekit_core::CanonicalSession) -> Result<Vec<InspectEntry>> {
    match session.source_agent {
        tracekit_core::Agent::Claude => inspect_claude(session),
        tracekit_core::Agent::Codex => inspect_codex(session),
        tracekit_core::Agent::Opencode => inspect_opencode(session),
        _ => inspect_generic_jsonl(&session.source_path, &session.source_agent.to_string()),
    }
}

fn inspect_claude(session: &tracekit_core::CanonicalSession) -> Result<Vec<InspectEntry>> {
    let content = std::fs::read_to_string(&session.source_path)?;
    let mut out = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = record
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ts = record
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match kind {
            "user" => {
                if let Some(content) = record.pointer("/message/content") {
                    match content {
                        Value::String(s) => out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "USER".to_string(),
                            title: "User prompt".to_string(),
                            body: Some(limit_text(s, 8000)),
                            source_type: "claude:user".to_string(),
                            metadata: vec![(
                                "is_meta".to_string(),
                                record
                                    .get("isMeta")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                                    .to_string(),
                            )],
                        }),
                        Value::Array(arr) => {
                            for block in arr {
                                let btype = block
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("block");
                                match btype {
                                    "tool_result" => {
                                        let tool_id = block
                                            .get("tool_use_id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("-");
                                        let is_error = block
                                            .get("is_error")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(false);
                                        let body =
                                            extract_text(block.get("content")).or_else(|| {
                                                Some(limit_text(&compact_json(block), 1200))
                                            });
                                        out.push(InspectEntry {
                                            ts: ts.clone(),
                                            label: "TOOL_RESULT".to_string(),
                                            title: format!("Tool result ({})", tool_id),
                                            body,
                                            source_type: "claude:user.tool_result".to_string(),
                                            metadata: vec![(
                                                "is_error".to_string(),
                                                is_error.to_string(),
                                            )],
                                        });
                                    }
                                    "text" => {
                                        if let Some(text) =
                                            block.get("text").and_then(|v| v.as_str())
                                        {
                                            out.push(InspectEntry {
                                                ts: ts.clone(),
                                                label: "USER".to_string(),
                                                title: "User prompt".to_string(),
                                                body: Some(limit_text(text, 8000)),
                                                source_type: "claude:user.text".to_string(),
                                                metadata: vec![],
                                            });
                                        }
                                    }
                                    _ => out.push(InspectEntry {
                                        ts: ts.clone(),
                                        label: "USER".to_string(),
                                        title: format!("User block: {}", btype),
                                        body: Some(limit_text(&compact_json(block), 1200)),
                                        source_type: "claude:user.block".to_string(),
                                        metadata: vec![],
                                    }),
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "assistant" => {
                if let Some(content_arr) = record
                    .pointer("/message/content")
                    .and_then(|v| v.as_array())
                {
                    for block in content_arr {
                        let btype = block
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("block");
                        match btype {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                    out.push(InspectEntry {
                                        ts: ts.clone(),
                                        label: "ASSISTANT".to_string(),
                                        title: "Assistant reply".to_string(),
                                        body: Some(limit_text(text, 8000)),
                                        source_type: "claude:assistant.text".to_string(),
                                        metadata: vec![],
                                    });
                                }
                            }
                            "thinking" | "redacted_thinking" => {
                                let thought = block
                                    .get("thinking")
                                    .or_else(|| block.get("text"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("[thinking present]");
                                out.push(InspectEntry {
                                    ts: ts.clone(),
                                    label: "THINKING".to_string(),
                                    title: "Assistant reasoning".to_string(),
                                    body: Some(limit_text(thought, 8000)),
                                    source_type: "claude:assistant.thinking".to_string(),
                                    metadata: vec![],
                                });
                            }
                            "tool_use" => {
                                let name =
                                    block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                                let tool_id =
                                    block.get("id").and_then(|v| v.as_str()).unwrap_or("-");
                                let args = block
                                    .get("input")
                                    .map(compact_json)
                                    .unwrap_or_else(|| "{}".to_string());
                                out.push(InspectEntry {
                                    ts: ts.clone(),
                                    label: "TOOL_CALL".to_string(),
                                    title: format!("Tool call: {}", name),
                                    body: Some(limit_text(&args, 2000)),
                                    source_type: "claude:assistant.tool_use".to_string(),
                                    metadata: vec![("tool_id".to_string(), tool_id.to_string())],
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            "system" => out.push(InspectEntry {
                ts: ts.clone(),
                label: "SYSTEM".to_string(),
                title: format!(
                    "System event ({})",
                    record
                        .get("subtype")
                        .and_then(|v| v.as_str())
                        .unwrap_or("general")
                ),
                body: Some(limit_text(
                    &compact_json(&redact_record(record.clone())),
                    1500,
                )),
                source_type: "claude:system".to_string(),
                metadata: vec![],
            }),
            "progress" | "file-history-snapshot" => out.push(InspectEntry {
                ts: ts.clone(),
                label: "EVENT".to_string(),
                title: format!("Event: {}", kind),
                body: Some(limit_text(
                    &compact_json(&redact_record(record.clone())),
                    1200,
                )),
                source_type: format!("claude:{}", kind),
                metadata: vec![],
            }),
            _ => {}
        }
    }

    Ok(out)
}

fn inspect_codex(session: &tracekit_core::CanonicalSession) -> Result<Vec<InspectEntry>> {
    let content = std::fs::read_to_string(&session.source_path)?;
    let mut out = Vec::new();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let kind = record
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let ts = record
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match kind {
            "session_meta" => {
                let payload = record.get("payload").cloned().unwrap_or(Value::Null);
                let body = serde_json::json!({
                    "id": payload.get("id"),
                    "cwd": payload.get("cwd"),
                    "originator": payload.get("originator"),
                    "model_provider": payload.get("model_provider"),
                    "cli_version": payload.get("cli_version")
                });
                out.push(InspectEntry {
                    ts: ts.clone(),
                    label: "SYSTEM".to_string(),
                    title: "Session metadata".to_string(),
                    body: Some(limit_text(&compact_json(&body), 1200)),
                    source_type: "codex:session_meta".to_string(),
                    metadata: vec![],
                });
            }
            "response_item" => {
                let payload = record.get("payload").cloned().unwrap_or(Value::Null);
                let ptype = payload
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                match ptype {
                    "message" => {
                        let role = payload
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or("assistant");
                        let text = extract_codex_message_text(&payload).unwrap_or_else(|| {
                            limit_text(&compact_json(&redact_record(payload.clone())), 1200)
                        });
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: role.to_uppercase(),
                            title: format!("{} message", capitalize(role)),
                            body: Some(text),
                            source_type: "codex:response_item.message".to_string(),
                            metadata: vec![],
                        });
                    }
                    "user_message" => {
                        let text = payload
                            .get("content")
                            .and_then(|v| v.as_str())
                            .or_else(|| payload.get("text").and_then(|v| v.as_str()))
                            .unwrap_or("");
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "USER".to_string(),
                            title: "User prompt".to_string(),
                            body: Some(limit_text(text, 8000)),
                            source_type: "codex:response_item.user_message".to_string(),
                            metadata: vec![],
                        });
                    }
                    "reasoning" => {
                        let text = payload
                            .get("summary")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.get("text").and_then(|t| t.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "[reasoning present]".to_string());
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "THINKING".to_string(),
                            title: "Assistant reasoning".to_string(),
                            body: Some(limit_text(&text, 8000)),
                            source_type: "codex:response_item.reasoning".to_string(),
                            metadata: vec![],
                        });
                    }
                    "function_call" | "custom_tool_call" => {
                        let name = payload
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool");
                        let args = payload
                            .get("arguments")
                            .or_else(|| payload.get("input"))
                            .map(|v| {
                                if let Some(s) = v.as_str() {
                                    s.to_string()
                                } else {
                                    compact_json(v)
                                }
                            })
                            .unwrap_or_else(|| "{}".to_string());
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "TOOL_CALL".to_string(),
                            title: format!("Tool call: {}", name),
                            body: Some(limit_text(&args, 2000)),
                            source_type: format!("codex:response_item.{}", ptype),
                            metadata: vec![(
                                "call_id".to_string(),
                                payload
                                    .get("call_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("-")
                                    .to_string(),
                            )],
                        });
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let output = payload
                            .get("output")
                            .map(|v| {
                                if let Some(s) = v.as_str() {
                                    s.to_string()
                                } else {
                                    compact_json(v)
                                }
                            })
                            .unwrap_or_default();
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "TOOL_RESULT".to_string(),
                            title: "Tool output".to_string(),
                            body: Some(limit_text(&output, 4000)),
                            source_type: format!("codex:response_item.{}", ptype),
                            metadata: vec![(
                                "call_id".to_string(),
                                payload
                                    .get("call_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("-")
                                    .to_string(),
                            )],
                        });
                    }
                    _ => {}
                }
            }
            "event_msg" => {
                let payload = record.get("payload").cloned().unwrap_or(Value::Null);
                let ptype = payload
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("event");
                match ptype {
                    "agent_message" => {
                        let text = payload
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "ASSISTANT".to_string(),
                            title: "Assistant reply".to_string(),
                            body: Some(limit_text(text, 8000)),
                            source_type: "codex:event_msg.agent_message".to_string(),
                            metadata: vec![],
                        });
                    }
                    "agent_reasoning" => {
                        let text = payload
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("[reasoning present]");
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "THINKING".to_string(),
                            title: "Assistant reasoning".to_string(),
                            body: Some(limit_text(text, 8000)),
                            source_type: "codex:event_msg.agent_reasoning".to_string(),
                            metadata: vec![],
                        });
                    }
                    "token_count" => {
                        let summary = serde_json::json!({
                            "last": payload.pointer("/info/last_token_usage"),
                            "total": payload.pointer("/info/total_token_usage"),
                        });
                        out.push(InspectEntry {
                            ts: ts.clone(),
                            label: "METRICS".to_string(),
                            title: "Token usage snapshot".to_string(),
                            body: Some(limit_text(&compact_json(&summary), 1200)),
                            source_type: "codex:event_msg.token_count".to_string(),
                            metadata: vec![],
                        });
                    }
                    _ => out.push(InspectEntry {
                        ts: ts.clone(),
                        label: "EVENT".to_string(),
                        title: format!("Event: {}", ptype),
                        body: Some(limit_text(
                            &compact_json(&redact_record(payload.clone())),
                            1200,
                        )),
                        source_type: format!("codex:event_msg.{}", ptype),
                        metadata: vec![],
                    }),
                }
            }
            "turn_context" => {
                let payload = record.get("payload").cloned().unwrap_or(Value::Null);
                let summary = serde_json::json!({
                    "turn_id": payload.get("turn_id"),
                    "cwd": payload.get("cwd"),
                    "model": payload.get("model"),
                    "approval_policy": payload.get("approval_policy"),
                    "sandbox_policy": payload.get("sandbox_policy"),
                });
                out.push(InspectEntry {
                    ts: ts.clone(),
                    label: "CONTEXT".to_string(),
                    title: "Turn context".to_string(),
                    body: Some(limit_text(&compact_json(&summary), 1200)),
                    source_type: "codex:turn_context".to_string(),
                    metadata: vec![],
                });
            }
            _ => {}
        }
    }

    Ok(out)
}

fn inspect_opencode(session: &tracekit_core::CanonicalSession) -> Result<Vec<InspectEntry>> {
    let mut out = Vec::new();
    let session_json = std::fs::read_to_string(&session.source_path)?;
    let session_value: Value = serde_json::from_str(&session_json).unwrap_or(Value::Null);

    out.push(InspectEntry {
        ts: None,
        label: "SYSTEM".to_string(),
        title: "Session metadata".to_string(),
        body: Some(limit_text(
            &compact_json(&redact_record(session_value.clone())),
            1400,
        )),
        source_type: "opencode:session".to_string(),
        metadata: vec![],
    });

    let root = match session.source_path.ancestors().nth(3) {
        Some(p) => p.to_path_buf(),
        None => return Ok(out),
    };

    let message_dir = root.join("message").join(&session.session_id);
    if !message_dir.exists() {
        return Ok(out);
    }

    let mut msg_files: Vec<PathBuf> = std::fs::read_dir(&message_dir)?
        .filter_map(|e| e.ok().map(|x| x.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    msg_files.sort();

    for msg_path in msg_files {
        let raw = match std::fs::read_to_string(&msg_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let msg: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let ts = msg
            .pointer("/time/created")
            .and_then(|v| v.as_u64())
            .map(ms_to_iso);
        let message_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let title = if role == "assistant" {
            "Assistant message"
        } else {
            "User message"
        };

        out.push(InspectEntry {
            ts,
            label: role.to_uppercase(),
            title: title.to_string(),
            body: None,
            source_type: "opencode:message".to_string(),
            metadata: vec![
                ("message_id".to_string(), message_id.to_string()),
                (
                    "model".to_string(),
                    msg.get("modelID")
                        .or_else(|| msg.pointer("/model/modelID"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("-")
                        .to_string(),
                ),
            ],
        });

        let part_dir = root.join("part").join(message_id);
        if !part_dir.exists() {
            continue;
        }
        let mut part_files: Vec<PathBuf> = std::fs::read_dir(&part_dir)?
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
            .collect();
        part_files.sort();

        for part_path in part_files {
            let part_raw = match std::fs::read_to_string(&part_path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let part: Value = match serde_json::from_str(&part_raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let ptype = part.get("type").and_then(|v| v.as_str()).unwrap_or("part");
            let p_ts = part
                .pointer("/time/start")
                .or_else(|| part.pointer("/time/end"))
                .and_then(|v| v.as_u64())
                .map(ms_to_iso);
            match ptype {
                "text" => {
                    let text = part.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    out.push(InspectEntry {
                        ts: p_ts,
                        label: role.to_uppercase(),
                        title: if role == "assistant" {
                            "Assistant text"
                        } else {
                            "User text"
                        }
                        .to_string(),
                        body: Some(limit_text(text, 8000)),
                        source_type: "opencode:part.text".to_string(),
                        metadata: vec![],
                    });
                }
                "reasoning" => {
                    let text = part
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[reasoning present]");
                    out.push(InspectEntry {
                        ts: p_ts,
                        label: "THINKING".to_string(),
                        title: "Assistant reasoning".to_string(),
                        body: Some(limit_text(text, 8000)),
                        source_type: "opencode:part.reasoning".to_string(),
                        metadata: vec![],
                    });
                }
                "tool" => {
                    let tool_name = part.get("tool").and_then(|v| v.as_str()).unwrap_or("tool");
                    let status = part
                        .pointer("/state/status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let input = part
                        .pointer("/state/input")
                        .map(compact_json)
                        .unwrap_or_default();
                    let output = part
                        .pointer("/state/output")
                        .map(compact_json)
                        .unwrap_or_default();
                    out.push(InspectEntry {
                        ts: p_ts.clone(),
                        label: "TOOL_CALL".to_string(),
                        title: format!("Tool: {}", tool_name),
                        body: Some(limit_text(&input, 2000)),
                        source_type: "opencode:part.tool".to_string(),
                        metadata: vec![("status".to_string(), status.to_string())],
                    });
                    out.push(InspectEntry {
                        ts: p_ts,
                        label: "TOOL_RESULT".to_string(),
                        title: format!("Tool result: {}", tool_name),
                        body: Some(limit_text(&output, 2000)),
                        source_type: "opencode:part.tool".to_string(),
                        metadata: vec![("status".to_string(), status.to_string())],
                    });
                }
                "step-finish" => {
                    let summary = serde_json::json!({
                        "reason": part.get("reason"),
                        "cost": part.get("cost"),
                        "tokens": part.get("tokens"),
                    });
                    out.push(InspectEntry {
                        ts: p_ts,
                        label: "METRICS".to_string(),
                        title: "Step finish".to_string(),
                        body: Some(limit_text(&compact_json(&summary), 1200)),
                        source_type: "opencode:part.step-finish".to_string(),
                        metadata: vec![],
                    });
                }
                _ => out.push(InspectEntry {
                    ts: p_ts,
                    label: "EVENT".to_string(),
                    title: format!("Part: {}", ptype),
                    body: Some(limit_text(
                        &compact_json(&redact_record(part.clone())),
                        1200,
                    )),
                    source_type: format!("opencode:part.{}", ptype),
                    metadata: vec![],
                }),
            }
        }
    }

    Ok(out)
}

fn inspect_generic_jsonl(path: &Path, agent_name: &str) -> Result<Vec<InspectEntry>> {
    let mut out = Vec::new();
    let content = std::fs::read_to_string(path)?;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = record
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let kind = record
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("record");
        out.push(InspectEntry {
            ts,
            label: "EVENT".to_string(),
            title: format!("{} record: {}", capitalize(agent_name), kind),
            body: Some(limit_text(
                &compact_json(&redact_record(record.clone())),
                1500,
            )),
            source_type: format!("{}:{}", agent_name, kind),
            metadata: vec![],
        });
    }
    Ok(out)
}

fn transform_inspect_entries(entries: &[InspectEntry], mode: InspectMode) -> InspectRender {
    match mode {
        InspectMode::Forensic => {
            let rendered = entries.to_vec();
            let summary = build_summary(entries.len(), &rendered, 0, 0);
            InspectRender {
                entries: rendered,
                summary,
            }
        }
        InspectMode::Analysis => {
            let mut filtered: Vec<InspectEntry> = Vec::new();
            let mut dropped_noise = 0usize;
            for e in entries {
                if is_noise_entry(e) {
                    dropped_noise += 1;
                } else {
                    filtered.push(e.clone());
                }
            }

            let mut deduped: Vec<InspectEntry> = Vec::new();
            let mut dropped_duplicates = 0usize;
            for e in filtered {
                if is_duplicate_of_last(&deduped, &e) {
                    dropped_duplicates += 1;
                } else {
                    deduped.push(e);
                }
            }

            let summary = build_summary(entries.len(), &deduped, dropped_noise, dropped_duplicates);
            InspectRender {
                entries: deduped,
                summary,
            }
        }
    }
}

fn is_noise_entry(e: &InspectEntry) -> bool {
    if e.label == "DEVELOPER" {
        return true;
    }

    matches!(
        e.source_type.as_str(),
        "codex:event_msg.token_count"
            | "codex:turn_context"
            | "codex:event_msg.agent_reasoning"
            | "codex:event_msg.agent_message"
            | "codex:event_msg.user_message"
            | "claude:file-history-snapshot"
            | "claude:progress"
    )
}

fn is_duplicate_of_last(existing: &[InspectEntry], e: &InspectEntry) -> bool {
    let Some(last) = existing.last() else {
        return false;
    };

    let same_label = last.label == e.label;
    let same_ts = last.ts == e.ts;
    let same_title = last.title == e.title;
    let same_body = normalize_body(last.body.as_deref()) == normalize_body(e.body.as_deref());

    same_label && same_ts && same_title && same_body
}

fn normalize_body(body: Option<&str>) -> String {
    body.unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_summary(
    raw_entries: usize,
    rendered: &[InspectEntry],
    dropped_noise: usize,
    dropped_duplicates: usize,
) -> InspectSummary {
    let mut label_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut tool_calls = 0usize;
    let mut tool_results = 0usize;
    let mut tool_errors = 0usize;

    for e in rendered {
        *label_counts.entry(e.label.clone()).or_default() += 1;
        if e.label == "TOOL_CALL" {
            tool_calls += 1;
        } else if e.label == "TOOL_RESULT" {
            tool_results += 1;
            if e.metadata.iter().any(|(k, v)| {
                (k == "is_error" && v == "true") || (k == "status" && v.contains("error"))
            }) {
                tool_errors += 1;
            }
        }
    }

    let mut labels: Vec<(String, usize)> = label_counts.into_iter().collect();
    labels.sort_by(|a, b| b.1.cmp(&a.1));

    InspectSummary {
        raw_entries,
        rendered_entries: rendered.len(),
        dropped_noise,
        dropped_duplicates,
        tool_calls,
        tool_results,
        tool_errors,
        labels,
    }
}

fn render_inspect_markdown(
    session: &tracekit_core::CanonicalSession,
    rendered: &InspectRender,
    mode: InspectMode,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# tracekit inspect — {}\n\n", session.session_id));
    out.push_str(&format!("- mode: `{}`\n", inspect_mode_str(mode)));
    out.push_str(&format!("- agent: `{}`\n", session.source_agent));
    out.push_str(&format!(
        "- source_path: `{}`\n",
        session.source_path.display()
    ));
    out.push_str(&format!(
        "- cwd: `{}`\n",
        session.cwd.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "- started_at: `{}`\n",
        session
            .started_at
            .map(|t| t.to_string())
            .unwrap_or_else(|| "-".to_string())
    ));
    out.push_str(&format!(
        "- entries: `{}`\n\n",
        rendered.summary.rendered_entries
    ));

    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- raw entries: `{}`\n",
        rendered.summary.raw_entries
    ));
    out.push_str(&format!(
        "- rendered entries: `{}`\n",
        rendered.summary.rendered_entries
    ));
    out.push_str(&format!(
        "- dropped (noise): `{}`\n",
        rendered.summary.dropped_noise
    ));
    out.push_str(&format!(
        "- dropped (duplicates): `{}`\n",
        rendered.summary.dropped_duplicates
    ));
    out.push_str(&format!(
        "- tools: calls=`{}`, results=`{}`, errors=`{}`\n",
        rendered.summary.tool_calls, rendered.summary.tool_results, rendered.summary.tool_errors
    ));
    let labels = rendered
        .summary
        .labels
        .iter()
        .map(|(label, count)| format!("{}={}", label, count))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("- labels: `{}`\n\n", labels));

    for (i, e) in rendered.entries.iter().enumerate() {
        out.push_str(&format!(
            "## {:04}. {} {}{}\n\n",
            i + 1,
            e.label,
            e.title,
            e.ts.as_ref()
                .map(|ts| format!(" ({})", ts))
                .unwrap_or_default()
        ));

        if let Some(body) = &e.body {
            out.push_str("```text\n");
            out.push_str(body);
            out.push_str("\n```\n\n");
        }

        out.push_str(&format!("source: `{}`\n", e.source_type));
        if !e.metadata.is_empty() {
            let meta = e
                .metadata
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("metadata: `{}`\n", meta));
        }
        out.push('\n');
    }
    out
}

fn print_inspect_terminal(
    session: &tracekit_core::CanonicalSession,
    rendered: &InspectRender,
    mode: InspectMode,
) {
    println!(
        "{}",
        "── Session Inspect ─────────────────────────────────────────────".bold()
    );
    println!("  Mode       : {}", inspect_mode_str(mode).cyan());
    println!("  Agent      : {}", session.source_agent.to_string().cyan());
    println!("  Session ID : {}", session.session_id);
    println!(
        "  Entries    : {} shown / {} raw",
        rendered.summary.rendered_entries, rendered.summary.raw_entries
    );
    println!(
        "  Dropped    : {} noise, {} duplicates",
        rendered.summary.dropped_noise, rendered.summary.dropped_duplicates
    );
    println!(
        "  Tools      : {} calls, {} results, {} errors",
        rendered.summary.tool_calls, rendered.summary.tool_results, rendered.summary.tool_errors
    );
    println!();

    for (i, e) in rendered.entries.iter().enumerate() {
        let tag = match e.label.as_str() {
            "USER" => e.label.blue().bold(),
            "ASSISTANT" => e.label.green().bold(),
            "THINKING" => e.label.magenta().bold(),
            "TOOL_CALL" => e.label.yellow().bold(),
            "TOOL_RESULT" => e.label.yellow().bold(),
            "SYSTEM" | "CONTEXT" => e.label.cyan().bold(),
            "METRICS" => e.label.bright_black().bold(),
            _ => e.label.normal(),
        };
        let ts = e.ts.as_deref().unwrap_or("-").dimmed();
        println!(
            "{}  {}  {}  {}",
            format!("[{:04}]", i + 1).dimmed(),
            ts,
            tag,
            e.title.bold()
        );
        if let Some(body) = &e.body {
            for line in body.lines().take(8) {
                println!("  {}", line);
            }
            if body.lines().count() > 8 {
                println!("  {}", "...".dimmed());
            }
        }
        if !e.metadata.is_empty() {
            let meta = e
                .metadata
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  {}", format!("meta: {}", meta).dimmed());
        }
        println!("  {}", format!("src: {}", e.source_type).dimmed());
        println!();
    }
}

fn inspect_mode_str(mode: InspectMode) -> &'static str {
    match mode {
        InspectMode::Analysis => "analysis",
        InspectMode::Forensic => "forensic",
    }
}

fn extract_codex_message_text(payload: &Value) -> Option<String> {
    let arr = payload.get("content")?.as_array()?;
    let mut chunks = Vec::new();
    for item in arr {
        let itype = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match itype {
            "output_text" | "text" => {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    chunks.push(text.to_string());
                }
            }
            _ => {}
        }
    }
    if chunks.is_empty() {
        None
    } else {
        Some(limit_text(&chunks.join("\n"), 8000))
    }
}

fn extract_text(v: Option<&Value>) -> Option<String> {
    let value = v?;
    match value {
        Value::String(s) => Some(s.to_string()),
        Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.as_str() {
                    parts.push(text.to_string());
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

fn ms_to_iso(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nsec = ((ms % 1000) * 1_000_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec)
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| ms.to_string())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn redact_record(mut v: Value) -> Value {
    redact_in_place(&mut v);
    v
}

fn redact_in_place(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for key in [
                "base_instructions",
                "user_instructions",
                "developer_instructions",
                "encrypted_content",
                "signature",
            ] {
                if map.contains_key(key) {
                    map.insert(key.to_string(), Value::String("[omitted]".to_string()));
                }
            }
            for value in map.values_mut() {
                redact_in_place(value);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                redact_in_place(item);
            }
        }
        Value::String(s) => {
            if s.chars().count() > 1000 {
                let mut truncated = String::new();
                for ch in s.chars().take(999) {
                    truncated.push(ch);
                }
                truncated.push('…');
                *s = truncated;
            }
        }
        _ => {}
    }
}

fn limit_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out = String::new();
        for ch in s.chars().take(max.saturating_sub(1)) {
            out.push(ch);
        }
        out.push('…');
        out
    }
}
