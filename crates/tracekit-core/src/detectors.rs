use crate::schema::*;

/// Run all detectors on a parsed session and return findings.
pub fn detect_inefficiencies(parsed: &ParsedSession) -> Vec<Finding> {
    let mut findings = Vec::new();
    let msgs = &parsed.messages;

    findings.extend(detect_retry_loops(msgs));
    findings.extend(detect_edit_cascades(msgs));
    findings.extend(detect_tool_fanout(msgs));
    findings.extend(detect_redundant_rereads(msgs));
    findings.extend(detect_context_bloat(msgs));
    findings.extend(detect_error_reprompt_churn(msgs));
    findings.extend(detect_subagent_overhead(msgs));

    // Sort by wasted cost descending
    findings.sort_by(|a, b| {
        let ca = a.wasted_cost_usd.unwrap_or(0.0);
        let cb = b.wasted_cost_usd.unwrap_or(0.0);
        cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal)
    });

    findings
}

/// Detect tool calls that fail and are immediately retried (same tool, similar args).
fn detect_retry_loops(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Collect all tool calls with errors across assistant messages
    let mut error_tools: Vec<(usize, &CanonicalTool)> = Vec::new();
    for msg in msgs {
        if msg.role == Role::Assistant {
            for tool in &msg.tool_calls {
                if tool.status == ToolStatus::Error {
                    error_tools.push((msg.sequence, tool));
                }
            }
        }
    }

    // For each errored tool, check if same tool appears in next 1-2 assistant messages
    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    let mut retry_groups: Vec<Vec<(usize, String)>> = Vec::new();
    // Track which (turn, tool_name) pairs have already been reported as part of a chain
    let mut reported: std::collections::HashSet<(usize, String)> = std::collections::HashSet::new();

    for (i, amsg) in assistant_msgs.iter().enumerate() {
        let error_calls: Vec<&CanonicalTool> = amsg.tool_calls.iter()
            .filter(|t| t.status == ToolStatus::Error)
            .collect();

        if error_calls.is_empty() {
            continue;
        }

        for err_tool in &error_calls {
            let key = (amsg.sequence, err_tool.tool_name.clone());
            if reported.contains(&key) {
                continue; // already part of a reported chain
            }

            let mut chain = vec![(amsg.sequence, err_tool.tool_name.clone())];

            // Look at next 5 assistant messages to find the full chain
            for next in assistant_msgs.iter().skip(i + 1).take(5) {
                let retry = next.tool_calls.iter().any(|t| t.tool_name == err_tool.tool_name);
                if retry {
                    chain.push((next.sequence, err_tool.tool_name.clone()));
                } else {
                    break;
                }
            }

            if chain.len() >= 2 {
                // Mark all turns in this chain as reported
                for item in &chain {
                    reported.insert(item.clone());
                }
                retry_groups.push(chain);
            }
        }
    }

    for group in retry_groups {
        let tool_name = group[0].1.clone();
        let count = group.len();
        let evidence: Vec<String> = group.iter()
            .map(|(seq, name)| format!("turn {}: {}", seq, name))
            .collect();

        findings.push(Finding {
            kind: FindingKind::RetryLoop,
            description: format!(
                "{} retried {} times after failure",
                tool_name, count - 1
            ),
            evidence,
            wasted_tokens: None,
            wasted_cost_usd: None,
            confidence: 0.85,
        });
    }

    findings
}

/// Detect repeated failed Edit/Write/Patch calls on the same file.
fn detect_edit_cascades(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let edit_tools = ["edit", "write", "str_replace_based_edit", "apply_patch",
                      "str_replace_editor", "replace_in_file"];

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    // Group consecutive edit tool calls per file path
    let mut file_edits: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();

    for amsg in &assistant_msgs {
        for tool in &amsg.tool_calls {
            let name_lower = tool.tool_name.to_lowercase();
            let is_edit = edit_tools.iter().any(|e| name_lower.contains(e));
            if is_edit && tool.status == ToolStatus::Error {
                if let Some(ref args) = tool.args_summary {
                    file_edits.entry(args.clone()).or_default().push(amsg.sequence);
                }
            }
        }
    }

    for (path, seqs) in &file_edits {
        if seqs.len() >= 2 {
            findings.push(Finding {
                kind: FindingKind::EditCascade,
                description: format!(
                    "Failed edit on '{}' repeated {} times",
                    truncate(path, 60),
                    seqs.len()
                ),
                evidence: seqs.iter().map(|s| format!("turn {}", s)).collect(),
                wasted_tokens: None,
                wasted_cost_usd: None,
                confidence: 0.80,
            });
        }
    }

    findings
}

/// Detect many adjacent calls to the same tool (could be batched).
fn detect_tool_fanout(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let batch_threshold = 4usize;

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    for amsg in &assistant_msgs {
        // Count tools by name within a single message turn
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for tool in &amsg.tool_calls {
            *counts.entry(tool.tool_name.as_str()).or_default() += 1;
        }
        for (name, count) in counts {
            if count >= batch_threshold {
                findings.push(Finding {
                    kind: FindingKind::ToolFanout,
                    description: format!(
                        "{} calls to '{}' in one turn — consider batching",
                        count, name
                    ),
                    evidence: vec![format!("turn {}", amsg.sequence)],
                    wasted_tokens: None,
                    wasted_cost_usd: None,
                    confidence: 0.70,
                });
            }
        }
    }

    findings
}

/// Detect the same file/resource being read multiple times with no writes in between.
fn detect_redundant_rereads(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();
    let read_tools = ["read", "cat", "view", "open", "read_file"];
    let write_tools = ["write", "edit", "str_replace", "apply_patch", "replace_in_file",
                       "create_file", "delete_file"];

    // Track last-written time per resource
    let mut last_written: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut read_count: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    for amsg in &assistant_msgs {
        for tool in &amsg.tool_calls {
            let name_lower = tool.tool_name.to_lowercase();
            let is_read = read_tools.iter().any(|r| name_lower.contains(r));
            let is_write = write_tools.iter().any(|w| name_lower.contains(w));

            if let Some(ref key) = tool.args_summary {
                if is_write {
                    last_written.insert(key.clone(), amsg.sequence);
                    read_count.remove(key);
                } else if is_read {
                    let last_write = last_written.get(key).copied().unwrap_or(0);
                    let reads = read_count.entry(key.clone()).or_default();
                    let all_after_write = reads.iter().all(|&s| s > last_write);
                    if all_after_write {
                        reads.push(amsg.sequence);
                    } else {
                        *reads = vec![amsg.sequence];
                    }
                }
            }
        }
    }

    for (path, seqs) in &read_count {
        if seqs.len() >= 3 {
            findings.push(Finding {
                kind: FindingKind::RedundantReread,
                description: format!(
                    "'{}' read {} times with no intervening write",
                    truncate(path, 60),
                    seqs.len()
                ),
                evidence: seqs.iter().map(|s| format!("turn {}", s)).collect(),
                wasted_tokens: None,
                wasted_cost_usd: None,
                confidence: 0.75,
            });
        }
    }

    findings
}

/// Detect unusually high input-token spikes (context bloat).
fn detect_context_bloat(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Compute average input tokens across assistant messages
    let input_counts: Vec<u64> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| m.usage.as_ref().map(|u| u.input_tokens))
        .collect();

    if input_counts.len() < 3 {
        return findings;
    }

    let mean: f64 = input_counts.iter().sum::<u64>() as f64 / input_counts.len() as f64;

    // Flag any message with >3x average input tokens
    let threshold = (mean * 3.0) as u64;
    let bloat_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .filter(|m| {
            m.usage.as_ref()
                .map(|u| u.input_tokens > threshold && u.input_tokens > 20_000)
                .unwrap_or(false)
        })
        .collect();

    for msg in bloat_msgs {
        let tokens = msg.usage.as_ref().unwrap().input_tokens;
        let excess = tokens.saturating_sub(mean as u64);
        findings.push(Finding {
            kind: FindingKind::ContextBloat,
            description: format!(
                "Turn {} has {} input tokens ({:.0}x avg {:.0}) — likely context bloat",
                msg.sequence,
                tokens,
                tokens as f64 / mean,
                mean
            ),
            evidence: vec![format!("turn {}: {} input tokens", msg.sequence, tokens)],
            wasted_tokens: Some(excess),
            wasted_cost_usd: None,
            confidence: 0.65,
        });
    }

    findings
}

/// Detect repeated error→reprompt cycles without new corrective context.
fn detect_error_reprompt_churn(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Look for sequences: assistant (with error tool) → user → assistant (same error) → ...
    let mut consecutive_errors = 0usize;
    let mut error_start_seq = 0usize;
    let mut error_end_seq = 0usize;
    let mut prev_error_tools: Vec<String> = Vec::new();
    let mut reported_churn: std::collections::HashSet<usize> = std::collections::HashSet::new();

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    for amsg in &assistant_msgs {
        let error_tools: Vec<String> = amsg.tool_calls.iter()
            .filter(|t| t.status == ToolStatus::Error)
            .map(|t| t.tool_name.clone())
            .collect();

        if !error_tools.is_empty() {
            let same_error = !prev_error_tools.is_empty()
                && error_tools.iter().any(|e| prev_error_tools.contains(e));

            if same_error {
                consecutive_errors += 1;
                error_end_seq = amsg.sequence;
            } else {
                consecutive_errors = 1;
                error_start_seq = amsg.sequence;
                error_end_seq = amsg.sequence;
            }
            prev_error_tools = error_tools;
        } else {
            // Flush: only report once per churn chain
            if consecutive_errors >= 3 && !reported_churn.contains(&error_start_seq) {
                reported_churn.insert(error_start_seq);
                findings.push(Finding {
                    kind: FindingKind::ErrorRepromptChurn,
                    description: format!(
                        "Same error repeated {} times (turns {}-{}) without resolution",
                        consecutive_errors, error_start_seq, error_end_seq
                    ),
                    evidence: vec![format!("turns {}-{}", error_start_seq, error_end_seq)],
                    wasted_tokens: None,
                    wasted_cost_usd: None,
                    confidence: 0.80,
                });
            }
            consecutive_errors = 0;
            prev_error_tools.clear();
        }
    }

    // Flush at end
    if consecutive_errors >= 3 && !reported_churn.contains(&error_start_seq) {
        findings.push(Finding {
            kind: FindingKind::ErrorRepromptChurn,
            description: format!(
                "Same error repeated {} times (turns {}-{}) without resolution",
                consecutive_errors, error_start_seq, error_end_seq
            ),
            evidence: vec![format!("turns {}-{}", error_start_seq, error_end_seq)],
            wasted_tokens: None,
            wasted_cost_usd: None,
            confidence: 0.80,
        });
    }

    findings
}

/// Detect sidechain/subagent usage that adds overhead.
fn detect_subagent_overhead(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let sidechain_count = msgs.iter().filter(|m| m.is_sidechain).count();
    if sidechain_count == 0 {
        return Vec::new();
    }

    let sidechain_cost: f64 = msgs.iter()
        .filter(|m| m.is_sidechain)
        .filter_map(|m| m.usage.as_ref()?.effective_cost())
        .sum();

    let sidechain_tokens: u64 = msgs.iter()
        .filter(|m| m.is_sidechain)
        .filter_map(|m| m.usage.as_ref())
        .map(|u| u.input_tokens + u.output_tokens)
        .sum();

    vec![Finding {
        kind: FindingKind::SubagentOverhead,
        description: format!(
            "{} sidechain/subagent messages — check if tasks could be inlined",
            sidechain_count
        ),
        evidence: vec![
            format!("{} subagent turns, {} tokens, ${:.4} cost",
                sidechain_count, sidechain_tokens, sidechain_cost)
        ],
        wasted_tokens: Some(sidechain_tokens / 4), // rough estimate of overhead
        wasted_cost_usd: if sidechain_cost > 0.0 { Some(sidechain_cost * 0.25) } else { None },
        confidence: 0.50,
    }]
}

/// Build top-N expensive messages list
pub fn top_expensive_messages(parsed: &ParsedSession, top_n: usize) -> Vec<ExpensiveMessage> {
    let mut messages: Vec<ExpensiveMessage> = parsed.messages.iter()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| {
            let u = m.usage.as_ref()?;
            let cost = u.effective_cost()?;
            Some(ExpensiveMessage {
                message_id: m.message_id.clone(),
                sequence: m.sequence,
                role: m.role,
                model: m.model.clone(),
                cost_usd: cost,
                input_tokens: u.input_tokens + u.cache_read_tokens + u.cache_write_tokens,
                output_tokens: u.output_tokens,
                tool_count: m.tool_calls.len(),
            })
        })
        .collect();

    messages.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
    messages.truncate(top_n);
    messages
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
