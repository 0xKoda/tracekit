use crate::schema::*;
use std::collections::{HashMap, HashSet};

/// Run all detectors on a parsed session and return findings.
pub fn detect_inefficiencies(parsed: &ParsedSession) -> Vec<Finding> {
    let mut findings = Vec::new();
    let msgs = &parsed.messages;

    // Build per-sequence cost lookup for waste estimation
    let cost_map: HashMap<usize, f64> = msgs.iter()
        .filter_map(|m| {
            let cost = m.usage.as_ref()?.effective_cost()?;
            Some((m.sequence, cost))
        })
        .collect();

    findings.extend(detect_retry_loops(msgs, &cost_map));
    findings.extend(detect_edit_cascades(msgs, &cost_map));
    findings.extend(detect_tool_fanout(msgs));
    findings.extend(detect_redundant_rereads(msgs));
    findings.extend(detect_context_bloat(msgs));
    findings.extend(detect_error_reprompt_churn(msgs, &cost_map));
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
fn detect_retry_loops(msgs: &[CanonicalMessage], cost_map: &HashMap<usize, f64>) -> Vec<Finding> {
    let mut findings = Vec::new();

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    let mut reported: HashSet<(usize, String)> = HashSet::new();

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
                continue;
            }

            let mut chain = vec![(amsg.sequence, err_tool.tool_name.clone())];

            for next in assistant_msgs.iter().skip(i + 1).take(5) {
                let retry = next.tool_calls.iter().any(|t| t.tool_name == err_tool.tool_name);
                if retry {
                    chain.push((next.sequence, err_tool.tool_name.clone()));
                } else {
                    break;
                }
            }

            if chain.len() >= 2 {
                for item in &chain {
                    reported.insert(item.clone());
                }

                // Waste = cost of all retry turns (skip first — that was the initial attempt)
                let wasted: f64 = chain[1..].iter()
                    .filter_map(|(seq, _)| cost_map.get(seq))
                    .sum();

                let tool_name = chain[0].1.clone();
                let evidence: Vec<String> = chain.iter()
                    .map(|(seq, name)| format!("turn {}: {}", seq, name))
                    .collect();

                findings.push(Finding {
                    kind: FindingKind::RetryLoop,
                    description: format!(
                        "{} retried {} times after failure",
                        tool_name,
                        chain.len() - 1
                    ),
                    evidence,
                    wasted_tokens: None,
                    wasted_cost_usd: if wasted > 0.0 { Some(wasted) } else { None },
                    confidence: 0.85,
                });
            }
        }
    }

    findings
}

/// Detect repeated failed Edit/Write/Patch calls on the same file.
fn detect_edit_cascades(msgs: &[CanonicalMessage], cost_map: &HashMap<usize, f64>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let edit_tools = ["edit", "write", "str_replace_based_edit", "apply_patch",
                      "str_replace_editor", "replace_in_file"];

    let assistant_msgs: Vec<&CanonicalMessage> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .collect();

    let mut file_edits: HashMap<String, Vec<usize>> = HashMap::new();

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
            // Waste = cost of all repeat turns after the first
            let wasted: f64 = seqs[1..].iter()
                .filter_map(|seq| cost_map.get(seq))
                .sum();

            findings.push(Finding {
                kind: FindingKind::EditCascade,
                description: format!(
                    "Failed edit on '{}' repeated {} times",
                    truncate(path, 60),
                    seqs.len()
                ),
                evidence: seqs.iter().map(|s| format!("turn {}", s)).collect(),
                wasted_tokens: None,
                wasted_cost_usd: if wasted > 0.0 { Some(wasted) } else { None },
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
        let mut counts: HashMap<&str, usize> = HashMap::new();
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

    let mut last_written: HashMap<String, usize> = HashMap::new();
    let mut read_count: HashMap<String, Vec<usize>> = HashMap::new();

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

/// Detect unusually high total-billed-input spikes (context bloat / over-injection).
fn detect_context_bloat(msgs: &[CanonicalMessage]) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Use total_billed_input (input + cache_read + cache_write) as the signal —
    // this catches both massive cache writes (initial injections) and cache reads
    // that spike because the context grew unexpectedly large.
    let billed_counts: Vec<(usize, u64, f64)> = msgs.iter()
        .filter(|m| m.role == Role::Assistant)
        .filter_map(|m| {
            let u = m.usage.as_ref()?;
            let cost = u.effective_cost()?;
            Some((m.sequence, u.total_billed_input(), cost))
        })
        .collect();

    if billed_counts.len() < 3 {
        return findings;
    }

    let mean: f64 = billed_counts.iter().map(|(_, t, _)| *t as f64).sum::<f64>()
        / billed_counts.len() as f64;

    // Flag turns with >2.5x average billed input and a minimum absolute threshold
    let threshold = (mean * 2.5) as u64;

    for (seq, total_billed, cost) in &billed_counts {
        if *total_billed > threshold && *total_billed > 200_000 {
            let excess = total_billed.saturating_sub(mean as u64);
            // Attribute the fraction of cost proportional to excess tokens
            let wasted = if *total_billed > 0 {
                Some(cost * (excess as f64 / *total_billed as f64))
            } else {
                None
            };

            findings.push(Finding {
                kind: FindingKind::ContextBloat,
                description: format!(
                    "Turn {} — {:.1}M billed tokens ({:.1}x avg) — likely context over-injection",
                    seq,
                    *total_billed as f64 / 1_000_000.0,
                    *total_billed as f64 / mean,
                ),
                evidence: vec![format!(
                    "turn {}: {} billed input tokens (${:.4})",
                    seq,
                    fmt_tokens_plain(*total_billed),
                    cost
                )],
                wasted_tokens: Some(excess),
                wasted_cost_usd: wasted,
                confidence: 0.70,
            });
        }
    }

    findings
}

/// Detect repeated error→reprompt cycles without new corrective context.
fn detect_error_reprompt_churn(
    msgs: &[CanonicalMessage],
    cost_map: &HashMap<usize, f64>,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    let mut consecutive_errors = 0usize;
    let mut error_start_seq = 0usize;
    let mut error_end_seq = 0usize;
    let mut churn_seqs: Vec<usize> = Vec::new();
    let mut prev_error_tools: Vec<String> = Vec::new();
    let mut reported_churn: HashSet<usize> = HashSet::new();

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
                churn_seqs.push(amsg.sequence);
            } else {
                consecutive_errors = 1;
                error_start_seq = amsg.sequence;
                error_end_seq = amsg.sequence;
                churn_seqs = vec![amsg.sequence];
            }
            prev_error_tools = error_tools;
        } else {
            if consecutive_errors >= 3 && !reported_churn.contains(&error_start_seq) {
                reported_churn.insert(error_start_seq);
                // Waste = cost of all churn turns beyond the first
                let wasted: f64 = churn_seqs[1..].iter()
                    .filter_map(|seq| cost_map.get(seq))
                    .sum();
                findings.push(Finding {
                    kind: FindingKind::ErrorRepromptChurn,
                    description: format!(
                        "Same error repeated {} times (turns {}-{}) without resolution",
                        consecutive_errors, error_start_seq, error_end_seq
                    ),
                    evidence: vec![format!("turns {}-{}", error_start_seq, error_end_seq)],
                    wasted_tokens: None,
                    wasted_cost_usd: if wasted > 0.0 { Some(wasted) } else { None },
                    confidence: 0.80,
                });
            }
            consecutive_errors = 0;
            prev_error_tools.clear();
            churn_seqs.clear();
        }
    }

    // Flush at end
    if consecutive_errors >= 3 && !reported_churn.contains(&error_start_seq) {
        let wasted: f64 = churn_seqs[1..].iter()
            .filter_map(|seq| cost_map.get(seq))
            .sum();
        findings.push(Finding {
            kind: FindingKind::ErrorRepromptChurn,
            description: format!(
                "Same error repeated {} times (turns {}-{}) without resolution",
                consecutive_errors, error_start_seq, error_end_seq
            ),
            evidence: vec![format!("turns {}-{}", error_start_seq, error_end_seq)],
            wasted_tokens: None,
            wasted_cost_usd: if wasted > 0.0 { Some(wasted) } else { None },
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
        .map(|u| u.total_billed_input() + u.output_tokens)
        .sum();

    vec![Finding {
        kind: FindingKind::SubagentOverhead,
        description: format!(
            "{} sidechain/subagent messages — check if tasks could be inlined",
            sidechain_count
        ),
        evidence: vec![
            format!("{} subagent turns, {} tokens, ${:.4} cost",
                sidechain_count, fmt_tokens_plain(sidechain_tokens), sidechain_cost)
        ],
        wasted_tokens: Some(sidechain_tokens / 4),
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
                input_tokens: u.total_billed_input(),
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

fn fmt_tokens_plain(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
