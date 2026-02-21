/// OpenCode session adapter.
/// Storage layout:
///   ~/.local/share/opencode/storage/session/<project_hash>/<ses_*.json>
///   ~/.local/share/opencode/storage/message/<ses_id>/<msg_*.json>
///   ~/.local/share/opencode/storage/part/<msg_id>/<prt_*.json>
use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use tracekit_core::*;
use walkdir::WalkDir;

use super::default_root;

pub fn discover_sessions() -> Result<Vec<CanonicalSession>> {
    let root = match default_root(Agent::Opencode) {
        Some(r) => r,
        None => return Ok(Vec::new()),
    };

    let session_root = root.join("session");
    if !session_root.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();

    for entry in WalkDir::new(&session_root)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match parse_session_file(path, &root) {
            Ok(s) => sessions.push(s),
            Err(_) => {}
        }
    }

    Ok(sessions)
}

#[derive(Debug, Deserialize)]
struct RawSession {
    id: String,
    #[serde(rename = "projectID")]
    project_id: Option<String>,
    directory: Option<String>,
    title: Option<String>,
    time: Option<RawTime>,
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTime {
    created: Option<u64>,
    updated: Option<u64>,
    completed: Option<u64>,
}

fn ms_to_utc(ms: u64) -> DateTime<Utc> {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    Utc.timestamp_opt(secs, nanos)
        .single()
        .unwrap_or_else(Utc::now)
}

fn parse_session_file(path: &std::path::Path, root: &std::path::Path) -> Result<CanonicalSession> {
    let content = std::fs::read_to_string(path)?;
    let raw: RawSession = serde_json::from_str(&content)
        .with_context(|| format!("parsing session {}", path.display()))?;

    let started_at = raw.time.as_ref().and_then(|t| t.created).map(ms_to_utc);

    let ended_at = raw
        .time
        .as_ref()
        .and_then(|t| t.updated.or(t.completed))
        .map(ms_to_utc);

    // Quick scan messages to get message count and model
    let msg_root = root.join("message").join(&raw.id);
    let (message_count, model) = if msg_root.exists() {
        let mut count = 0;
        let mut found_model: Option<String> = None;
        for e in WalkDir::new(&msg_root)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if e.path().extension().and_then(|x| x.to_str()) == Some("json") {
                count += 1;
                if found_model.is_none() {
                    if let Ok(data) = std::fs::read_to_string(e.path()) {
                        if let Ok(v) = serde_json::from_str::<Value>(&data) {
                            if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
                                // Strip provider prefix e.g. "openrouter/moonshotai/kimi-k2.5" -> keep as-is
                                found_model = Some(m.to_string());
                            }
                        }
                    }
                }
            }
        }
        (count, found_model)
    } else {
        (0, None)
    };

    Ok(CanonicalSession {
        session_id: raw.id,
        source_agent: Agent::Opencode,
        source_path: path.to_path_buf(),
        cwd: raw.directory,
        title: raw.title,
        started_at,
        ended_at,
        model,
        message_count,
        total_cost_usd: None,
        total_input_tokens: 0,
        total_output_tokens: 0,
    })
}

pub fn parse_session(session: &CanonicalSession) -> Result<ParsedSession> {
    let root = match default_root(Agent::Opencode) {
        Some(r) => r,
        None => {
            return Ok(ParsedSession {
                session: session.clone(),
                messages: Vec::new(),
            })
        }
    };

    let msg_root = root.join("message").join(&session.session_id);
    let part_root = root.join("part");

    if !msg_root.exists() {
        return Ok(ParsedSession {
            session: session.clone(),
            messages: Vec::new(),
        });
    }

    let mut messages = Vec::new();
    let mut seq = 0usize;

    // Collect all message files
    let mut msg_files: Vec<PathBuf> = WalkDir::new(&msg_root)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|e| e.path().to_path_buf())
        .collect();

    // Sort by filename (which encodes a timestamp-like ID)
    msg_files.sort();

    for msg_path in &msg_files {
        let data = match std::fs::read_to_string(msg_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_id = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let role_str = v.get("role").and_then(|x| x.as_str()).unwrap_or("user");
        let role = match role_str {
            "assistant" => Role::Assistant,
            "user" => Role::User,
            _ => Role::User,
        };
        let model = v
            .get("modelID")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let parent_id = v
            .get("parentID")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());

        let ts = v
            .pointer("/time/created")
            .and_then(|x| x.as_u64())
            .map(ms_to_utc);

        let latency_ms = match (
            v.pointer("/time/created").and_then(|x| x.as_u64()),
            v.pointer("/time/completed").and_then(|x| x.as_u64()),
        ) {
            (Some(s), Some(e)) if e >= s => Some(e - s),
            _ => None,
        };

        // Direct cost/token fields on message (aggregated)
        let cost_observed = v.get("cost").and_then(|x| x.as_f64());
        let direct_usage = extract_opencode_usage(&v, cost_observed, latency_ms, model.as_deref());

        // Load parts for this message
        let msg_part_root = part_root.join(&msg_id);
        let (tool_calls, step_usage) = if msg_part_root.exists() {
            load_parts(&msg_part_root, model.as_deref())?
        } else {
            (Vec::new(), None)
        };

        // Prefer step-finish usage if available (it's per-step), otherwise use message-level
        let usage = step_usage.or(direct_usage);

        seq += 1;
        messages.push(CanonicalMessage {
            message_id: msg_id,
            session_id: session.session_id.clone(),
            parent_id,
            sequence: seq,
            role,
            model,
            ts,
            usage,
            tool_calls,
            is_sidechain: false,
            finish_reason: v
                .get("finish")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
        });
    }

    Ok(ParsedSession {
        session: session.clone(),
        messages,
    })
}

fn extract_opencode_usage(
    v: &Value,
    cost: Option<f64>,
    latency_ms: Option<u64>,
    model: Option<&str>,
) -> Option<CanonicalUsage> {
    let tokens = v.get("tokens")?;
    let input = tokens.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
    let output = tokens.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
    let reasoning = tokens
        .get("reasoning")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_read = tokens
        .pointer("/cache/read")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_write = tokens
        .pointer("/cache/write")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    let cost_estimated = if cost.is_none() {
        model.and_then(|m| tracekit_core::estimate_cost(m, input, output, cache_read, cache_write))
    } else {
        None
    };

    Some(CanonicalUsage {
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cost_observed_usd: cost,
        cost_estimated_usd: cost_estimated,
        latency_ms,
    })
}

fn load_parts(
    part_dir: &PathBuf,
    model: Option<&str>,
) -> Result<(Vec<CanonicalTool>, Option<CanonicalUsage>)> {
    let mut tool_calls = Vec::new();
    let mut step_usage: Option<CanonicalUsage> = None;

    let mut part_files: Vec<PathBuf> = WalkDir::new(part_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|e| e.path().to_path_buf())
        .collect();

    part_files.sort();

    for part_path in &part_files {
        let data = match std::fs::read_to_string(part_path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let part_type = v.get("type").and_then(|x| x.as_str()).unwrap_or("");

        match part_type {
            "step-finish" => {
                let cost = v.get("cost").and_then(|x| x.as_f64());
                if let Some(tokens) = v.get("tokens") {
                    let input = tokens.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
                    let output = tokens.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
                    let reasoning = tokens
                        .get("reasoning")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    let cache_read = tokens
                        .pointer("/cache/read")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    let cache_write = tokens
                        .pointer("/cache/write")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);

                    let cost_estimated = if cost.is_none() {
                        model.and_then(|m| {
                            tracekit_core::estimate_cost(m, input, output, cache_read, cache_write)
                        })
                    } else {
                        None
                    };

                    // Accumulate step-finish costs (there may be multiple per message)
                    if let Some(ref mut existing) = step_usage {
                        existing.input_tokens += input;
                        existing.output_tokens += output;
                        existing.reasoning_tokens += reasoning;
                        existing.cache_read_tokens += cache_read;
                        existing.cache_write_tokens += cache_write;
                        if let Some(c) = cost {
                            *existing.cost_observed_usd.get_or_insert(0.0) += c;
                        }
                    } else {
                        step_usage = Some(CanonicalUsage {
                            input_tokens: input,
                            output_tokens: output,
                            reasoning_tokens: reasoning,
                            cache_read_tokens: cache_read,
                            cache_write_tokens: cache_write,
                            cost_observed_usd: cost,
                            cost_estimated_usd: cost_estimated,
                            latency_ms: None,
                        });
                    }
                }
            }

            "tool" => {
                let call_id = v
                    .get("callID")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = v
                    .get("tool")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let status_str = v
                    .pointer("/state/status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown");
                let status = match status_str {
                    "completed" => ToolStatus::Success,
                    "error" => ToolStatus::Error,
                    _ => ToolStatus::Unknown,
                };

                let args_summary = v.pointer("/state/input").map(|x| extract_opencode_args(x));

                let err_msg = if status == ToolStatus::Error {
                    v.pointer("/state/output")
                        .and_then(|x| x.as_str())
                        .map(|s| s.chars().take(200).collect())
                } else {
                    None
                };

                let duration_ms = match (
                    v.pointer("/state/time/start").and_then(|x| x.as_u64()),
                    v.pointer("/state/time/end").and_then(|x| x.as_u64()),
                ) {
                    (Some(s), Some(e)) if e >= s => Some(e - s),
                    _ => None,
                };

                tool_calls.push(CanonicalTool {
                    tool_name,
                    call_id,
                    status,
                    error_class: if status == ToolStatus::Error {
                        Some("tool_error".to_string())
                    } else {
                        None
                    },
                    error_message: err_msg,
                    args_summary,
                    output_summary: None,
                    duration_ms,
                });
            }

            _ => {}
        }
    }

    Ok((tool_calls, step_usage))
}

fn extract_opencode_args(v: &Value) -> String {
    // Try common fields
    for key in &["file", "path", "command", "query", "pattern", "name"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            return s.chars().take(100).collect();
        }
    }
    // Fallback to compact JSON
    serde_json::to_string(v)
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect()
}
