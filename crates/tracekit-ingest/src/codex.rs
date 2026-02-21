/// Codex (ChatGPT Codex) session adapter.
/// Format: ~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl
/// Each line: {"timestamp": "...", "type": "session_meta"|"response_item"|"event_msg", "payload": {...}}
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use tracekit_core::*;
use walkdir::WalkDir;

use super::default_root;

pub fn discover_sessions() -> Result<Vec<CanonicalSession>> {
    let root = match default_root(Agent::Codex) {
        Some(r) => r,
        None => return Ok(Vec::new()),
    };

    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    for entry in WalkDir::new(&root)
        .min_depth(4) // YYYY/MM/DD/rollout-*.jsonl
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("rollout-") {
            continue;
        }

        match probe_session(path) {
            Ok(s) => sessions.push(s),
            Err(_) => {}
        }
    }

    Ok(sessions)
}

fn probe_session(path: &Path) -> Result<CanonicalSession> {
    let content = std::fs::read_to_string(path)?;
    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut model: Option<String> = None;
    let mut message_count = 0usize;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match kind {
            "session_meta" => {
                let payload = record.get("payload").unwrap_or(&Value::Null);
                session_id = payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                cwd = payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(ts) = payload.get("timestamp").and_then(|v| v.as_str()) {
                    started_at = ts.parse().ok();
                }
                if let Some(mp) = payload.get("model_provider").and_then(|v| v.as_str()) {
                    model = Some(mp.to_string());
                }
            }
            "response_item" => {
                let payload = record.get("payload").unwrap_or(&Value::Null);
                let ptype = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ptype {
                    // Modern Codex rollout format: message items with explicit role.
                    "message" => {
                        let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                        if matches!(role, "user" | "assistant") {
                            message_count += 1;
                        }
                    }
                    // Legacy/alternative message item types.
                    "user_message" | "agent_message" | "output_text" => {
                        message_count += 1;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    // Fallback: derive session_id from filename
    let session_id = session_id.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split('-').last())
            .unwrap_or("unknown")
            .to_string()
    });

    Ok(CanonicalSession {
        session_id,
        source_agent: Agent::Codex,
        source_path: path.to_path_buf(),
        cwd,
        title: None,
        started_at,
        ended_at: None,
        model,
        message_count,
        total_cost_usd: None,
        total_input_tokens: 0,
        total_output_tokens: 0,
    })
}

pub fn parse_session(session: &CanonicalSession) -> Result<ParsedSession> {
    let content = std::fs::read_to_string(&session.source_path)
        .with_context(|| format!("reading {}", session.source_path.display()))?;

    let mut messages = Vec::new();
    let mut seq = 0usize;

    // We build a virtual "assistant turn" by accumulating function_call and output blocks
    // between user messages. Codex doesn't have clean turn boundaries, so we group by
    // "agent_message" / task_complete events.

    let mut current_tool_calls: Vec<CanonicalTool> = Vec::new();
    let mut pending_calls: HashMap<String, String> = HashMap::new(); // call_id -> tool_name
    let mut current_ts: Option<DateTime<Utc>> = None;
    let mut in_turn = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let ts: Option<DateTime<Utc>> = record
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok());

        match kind {
            "session_meta" => {
                // Beginning of session — synthesize a system message
            }

            "response_item" => {
                let payload = record.get("payload").unwrap_or(&Value::Null);
                let ptype = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match ptype {
                    "user_message" => {
                        // Flush current assistant turn if any
                        if in_turn {
                            flush_assistant_turn(
                                &mut messages,
                                &mut seq,
                                session,
                                &mut current_tool_calls,
                                current_ts,
                            );
                            in_turn = false;
                        }
                        // Add user message
                        seq += 1;
                        messages.push(CanonicalMessage {
                            message_id: format!("user-{}", seq),
                            session_id: session.session_id.clone(),
                            parent_id: None,
                            sequence: seq,
                            role: Role::User,
                            model: None,
                            ts,
                            usage: None,
                            tool_calls: Vec::new(),
                            is_sidechain: false,
                            finish_reason: None,
                        });
                        in_turn = true;
                        current_ts = ts;
                    }

                    "function_call" => {
                        in_turn = true;
                        if current_ts.is_none() {
                            current_ts = ts;
                        }
                        let call_id = payload
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = payload
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = payload
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let args_summary = extract_codex_args(args, &name);

                        pending_calls.insert(call_id.clone(), name.clone());
                        current_tool_calls.push(CanonicalTool {
                            tool_name: name,
                            call_id,
                            status: ToolStatus::Unknown,
                            error_class: None,
                            error_message: None,
                            args_summary,
                            output_summary: None,
                            duration_ms: None,
                        });
                    }

                    "function_call_output" => {
                        let call_id = payload
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let output = payload.get("output").and_then(|v| v.as_str()).unwrap_or("");

                        let is_error = output_looks_like_error(output);
                        let status = if is_error {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Success
                        };

                        // Update the pending tool call
                        for tool in current_tool_calls.iter_mut() {
                            if tool.call_id == call_id {
                                tool.status = status;
                                if is_error {
                                    tool.error_class = Some("exec_error".to_string());
                                    tool.error_message = Some(output.chars().take(200).collect());
                                } else {
                                    tool.output_summary = Some(output.chars().take(100).collect());
                                }
                                break;
                            }
                        }
                    }

                    "agent_message" | "task_complete" => {
                        // End of this assistant turn
                        if in_turn || !current_tool_calls.is_empty() {
                            flush_assistant_turn(
                                &mut messages,
                                &mut seq,
                                session,
                                &mut current_tool_calls,
                                current_ts,
                            );
                            in_turn = false;
                            current_ts = None;
                        }
                    }

                    "custom_tool_call" => {
                        // Similar to function_call
                        in_turn = true;
                        let call_id = payload
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = payload
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("custom_tool")
                            .to_string();
                        pending_calls.insert(call_id.clone(), name.clone());
                        current_tool_calls.push(CanonicalTool {
                            tool_name: name,
                            call_id,
                            status: ToolStatus::Unknown,
                            error_class: None,
                            error_message: None,
                            args_summary: None,
                            output_summary: None,
                            duration_ms: None,
                        });
                    }

                    "custom_tool_call_output" => {
                        let call_id = payload
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let output = payload
                            .get("output")
                            .and_then(|v| {
                                v.as_str()
                                    .map(|s| s.to_string())
                                    .or_else(|| serde_json::to_string(v).ok())
                            })
                            .unwrap_or_default();
                        let is_error = output_looks_like_error(&output);

                        for tool in current_tool_calls.iter_mut() {
                            if tool.call_id == call_id {
                                tool.status = if is_error {
                                    ToolStatus::Error
                                } else {
                                    ToolStatus::Success
                                };
                                if is_error {
                                    tool.error_class = Some("exec_error".to_string());
                                    tool.error_message = Some(output.chars().take(200).collect());
                                }
                                break;
                            }
                        }
                    }

                    _ => {}
                }
            }

            "event_msg" => {
                // token_count, workspace-write, etc. — not much useful per-call data here
            }

            _ => {}
        }
    }

    // Flush any remaining turn
    if in_turn || !current_tool_calls.is_empty() {
        flush_assistant_turn(
            &mut messages,
            &mut seq,
            session,
            &mut current_tool_calls,
            current_ts,
        );
    }

    Ok(ParsedSession {
        session: session.clone(),
        messages,
    })
}

fn flush_assistant_turn(
    messages: &mut Vec<CanonicalMessage>,
    seq: &mut usize,
    session: &CanonicalSession,
    tool_calls: &mut Vec<CanonicalTool>,
    ts: Option<DateTime<Utc>>,
) {
    *seq += 1;
    messages.push(CanonicalMessage {
        message_id: format!("asst-{}", *seq),
        session_id: session.session_id.clone(),
        parent_id: None,
        sequence: *seq,
        role: Role::Assistant,
        model: session.model.clone(),
        ts,
        usage: None, // Codex rollout files don't include per-call token counts
        tool_calls: std::mem::take(tool_calls),
        is_sidechain: false,
        finish_reason: None,
    });
}

fn extract_codex_args(args_json: &str, tool_name: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args_json).ok()?;

    // For exec_command, use cmd
    if tool_name == "exec_command" || tool_name == "shell" {
        if let Some(s) = v.get("cmd").or(v.get("command")).and_then(|x| x.as_str()) {
            return Some(s.chars().take(100).collect());
        }
    }

    // Try common path keys
    for key in &["path", "file_path", "pattern", "file", "query"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            return Some(s.chars().take(100).collect());
        }
    }

    // First string value
    if let Some(obj) = v.as_object() {
        for (_, val) in obj {
            if let Some(s) = val.as_str() {
                if s.len() > 2 {
                    return Some(s.chars().take(100).collect());
                }
            }
        }
    }
    None
}

fn output_looks_like_error(output: &str) -> bool {
    let lower = output.to_lowercase();
    // Check for common error indicators
    lower.contains("error")
        && (lower.contains("exit code") || lower.contains("failed") || lower.contains("not found"))
        || lower.starts_with("error:")
        || lower.contains("command not found")
        || lower.contains("permission denied")
        || lower.contains("no such file or directory")
        || (lower.contains("process exited with code") && !lower.contains("code 0"))
}
