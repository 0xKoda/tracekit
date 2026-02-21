/// Claude Code session adapter.
/// Format: ~/.claude/projects/**/<session-uuid>.jsonl
/// Each line is a JSON record with "type" field.
/// Subagent files live in <session-uuid>/subagents/agent-<id>.jsonl.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracekit_core::*;
use walkdir::WalkDir;

use super::default_root;

// ── raw record types ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RawRecord {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(flatten)]
    rest: Value,
}

pub fn discover_sessions() -> Result<Vec<CanonicalSession>> {
    let root = match default_root(Agent::Claude) {
        Some(r) => r,
        None => return Ok(Vec::new()),
    };

    if !root.exists() {
        return Ok(Vec::new());
    }

    // Group JSONL files by session_id (the UUID filename, excluding subagent paths)
    // Session files: <project>/<uuid>.jsonl
    // Subagent files: <project>/<uuid>/subagents/agent-*.jsonl (handled during parse)
    let mut session_paths: HashMap<String, PathBuf> = HashMap::new();

    for entry in WalkDir::new(&root)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
            // Skip subagent files at this level (they're agent-* not uuid-*)
            if name.starts_with("agent-") {
                continue;
            }
            session_paths.insert(name.to_string(), path.to_path_buf());
        }
    }

    let mut sessions = Vec::new();
    for (session_id, path) in session_paths {
        match probe_session(&session_id, &path) {
            Ok(s) => sessions.push(s),
            Err(_) => {} // skip unparseable sessions
        }
    }

    Ok(sessions)
}

/// Quick scan — read only first ~20 records to extract metadata.
fn probe_session(session_id: &str, path: &Path) -> Result<CanonicalSession> {
    let content = std::fs::read_to_string(path)?;
    let mut cwd: Option<String> = None;
    let mut started_at: Option<DateTime<Utc>> = None;
    let mut model: Option<String> = None;
    let mut message_count = 0usize;

    for line in content.lines().take(50) {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match kind {
            "user" => {
                message_count += 1;
                if cwd.is_none() {
                    cwd = record
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if started_at.is_none() {
                    if let Some(ts) = record.get("timestamp").and_then(|v| v.as_str()) {
                        started_at = ts.parse().ok();
                    }
                }
            }
            "assistant" => {
                message_count += 1;
                if model.is_none() {
                    model = record
                        .pointer("/message/model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            _ => {}
        }
    }

    Ok(CanonicalSession {
        session_id: session_id.to_string(),
        source_agent: Agent::Claude,
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
    let mut messages = Vec::new();
    let mut seq = 0usize;

    parse_jsonl_file(
        &session.source_path,
        session,
        &mut messages,
        &mut seq,
        false,
    )?;

    // Also load subagent files
    let subagent_dir = session.source_path.with_extension("").join("subagents");
    if subagent_dir.exists() {
        for entry in WalkDir::new(&subagent_dir)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                let _ = parse_jsonl_file(path, session, &mut messages, &mut seq, true);
            }
        }
    }

    // Sort by sequence
    messages.sort_by_key(|m| m.sequence);

    Ok(ParsedSession {
        session: session.clone(),
        messages,
    })
}

fn parse_jsonl_file(
    path: &Path,
    session: &CanonicalSession,
    messages: &mut Vec<CanonicalMessage>,
    seq: &mut usize,
    is_sidechain: bool,
) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    // We need to pair tool_use calls with their tool_result responses.
    // Tool uses appear in assistant messages, results in the following user message.
    let mut pending_tools: HashMap<String, CanonicalTool> = HashMap::new();

    for (line_no, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warn: {}:{}: parse error: {}",
                    path.display(),
                    line_no + 1,
                    e
                );
                continue;
            }
        };

        let kind = record.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match kind {
            "assistant" => {
                *seq += 1;
                let cur_seq = *seq;
                let ts = record
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok());

                let model = record
                    .pointer("/message/model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let parent_id = record
                    .get("parentUuid")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let msg_id = record
                    .pointer("/message/id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Usage
                let usage = extract_claude_usage(&record, model.as_deref());

                // Tool calls from content blocks
                let mut tool_calls: Vec<CanonicalTool> = Vec::new();
                if let Some(content_arr) = record
                    .pointer("/message/content")
                    .and_then(|v| v.as_array())
                {
                    for block in content_arr {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                            let tool_id = block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let tool_name = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args_summary = extract_args_key(block.get("input"));

                            let tool = CanonicalTool {
                                tool_name: tool_name.clone(),
                                call_id: tool_id.clone(),
                                status: ToolStatus::Unknown,
                                error_class: None,
                                error_message: None,
                                args_summary,
                                output_summary: None,
                                duration_ms: None,
                            };
                            pending_tools.insert(tool_id, tool.clone());
                            tool_calls.push(tool);
                        }
                    }
                }

                let sidechain_flag = record
                    .get("isSidechain")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                messages.push(CanonicalMessage {
                    message_id: msg_id,
                    session_id: session.session_id.clone(),
                    parent_id,
                    sequence: cur_seq,
                    role: Role::Assistant,
                    model,
                    ts,
                    usage,
                    tool_calls,
                    is_sidechain: is_sidechain || sidechain_flag,
                    finish_reason: record
                        .pointer("/message/stop_reason")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }

            "user" => {
                // Check for tool_result blocks — update pending tool statuses
                if let Some(content_arr) = record
                    .pointer("/message/content")
                    .and_then(|v| v.as_array())
                {
                    for block in content_arr {
                        if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let is_error = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            if let Some(pending) = pending_tools.get(&tool_use_id) {
                                let status = if is_error {
                                    ToolStatus::Error
                                } else {
                                    ToolStatus::Success
                                };
                                let err_msg = if is_error {
                                    extract_content_text(block.get("content"))
                                        .map(|s| s.chars().take(200).collect())
                                } else {
                                    None
                                };

                                // Update the tool status in the last assistant message that has this tool
                                for msg in messages.iter_mut().rev() {
                                    let mut updated = false;
                                    for tool in msg.tool_calls.iter_mut() {
                                        if tool.call_id == tool_use_id {
                                            tool.status = status;
                                            tool.error_message = err_msg.clone();
                                            if is_error {
                                                tool.error_class = Some("tool_error".to_string());
                                            }
                                            updated = true;
                                            break;
                                        }
                                    }
                                    if updated {
                                        break;
                                    }
                                }
                                let _ = pending; // suppress warning
                                pending_tools.remove(&tool_use_id);
                            }
                        }
                    }
                }

                *seq += 1;
                let ts = record
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok());

                messages.push(CanonicalMessage {
                    message_id: record
                        .pointer("/message/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("user")
                        .to_string(),
                    session_id: session.session_id.clone(),
                    parent_id: record
                        .get("parentUuid")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    sequence: *seq,
                    role: Role::User,
                    model: None,
                    ts,
                    usage: None,
                    tool_calls: Vec::new(),
                    is_sidechain,
                    finish_reason: None,
                });
            }

            _ => {}
        }
    }

    Ok(())
}

fn extract_claude_usage(record: &Value, model: Option<&str>) -> Option<CanonicalUsage> {
    let usage = record.pointer("/message/usage")?;

    let input_tokens = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_write = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let cost_estimated = model.and_then(|m| {
        tracekit_core::estimate_cost(m, input_tokens, output_tokens, cache_read, cache_write)
    });

    Some(CanonicalUsage {
        input_tokens,
        output_tokens,
        reasoning_tokens: 0,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cost_observed_usd: None,
        cost_estimated_usd: cost_estimated,
        latency_ms: None,
    })
}

fn extract_args_key(input: Option<&Value>) -> Option<String> {
    let v = input?;
    // Try common path/file keys
    for key in &[
        "file_path",
        "path",
        "pattern",
        "command",
        "query",
        "notebook_path",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    // Fallback: first string value
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

fn extract_content_text(content: Option<&Value>) -> Option<String> {
    let v = content?;
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        for item in arr {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(s) = item.get("text").and_then(|t| t.as_str()) {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}
