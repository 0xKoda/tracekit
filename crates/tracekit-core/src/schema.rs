use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Opencode,
    Codex,
    Pi,
    Kodo,
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Agent::Claude => write!(f, "claude"),
            Agent::Opencode => write!(f, "opencode"),
            Agent::Codex => write!(f, "codex"),
            Agent::Pi => write!(f, "pi"),
            Agent::Kodo => write!(f, "kodo"),
        }
    }
}

impl std::str::FromStr for Agent {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" => Ok(Agent::Claude),
            "opencode" => Ok(Agent::Opencode),
            "codex" => Ok(Agent::Codex),
            "pi" => Ok(Agent::Pi),
            "kodo" => Ok(Agent::Kodo),
            _ => Err(anyhow::anyhow!("Unknown agent: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalSession {
    pub session_id: String,
    pub source_agent: Agent,
    pub source_path: PathBuf,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub message_count: usize,
    pub total_cost_usd: Option<f64>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

impl CanonicalSession {
    pub fn duration_secs(&self) -> Option<i64> {
        match (self.started_at, self.ended_at) {
            (Some(s), Some(e)) => Some((e - s).num_seconds()),
            _ => None,
        }
    }

    pub fn effective_cost(&self) -> Option<f64> {
        self.total_cost_usd
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalMessage {
    pub message_id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub sequence: usize,
    pub role: Role,
    pub model: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub usage: Option<CanonicalUsage>,
    pub tool_calls: Vec<CanonicalTool>,
    pub is_sidechain: bool,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::System => write!(f, "system"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Directly observed cost from source (e.g. OpenCode provides this)
    pub cost_observed_usd: Option<f64>,
    /// Estimated cost from token counts × model pricing
    pub cost_estimated_usd: Option<f64>,
    pub latency_ms: Option<u64>,
}

impl CanonicalUsage {
    pub fn effective_cost(&self) -> Option<f64> {
        self.cost_observed_usd.or(self.cost_estimated_usd)
    }

    pub fn total_billed_input(&self) -> u64 {
        // Cache reads are billed at ~10% of input price; cache writes at ~25%.
        // For a simple total we count all input tokens.
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalTool {
    pub tool_name: String,
    pub call_id: String,
    pub status: ToolStatus,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub args_summary: Option<String>,
    pub output_summary: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Success,
    Error,
    Unknown,
}

/// A fully parsed session with all messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedSession {
    pub session: CanonicalSession,
    pub messages: Vec<CanonicalMessage>,
}

impl ParsedSession {
    /// Compute aggregate cost across all messages
    pub fn compute_totals(&mut self) {
        let mut total_cost = 0.0_f64;
        let mut has_cost = false;
        let mut total_input = 0u64;
        let mut total_output = 0u64;

        for msg in &self.messages {
            if let Some(ref u) = msg.usage {
                total_input += u.input_tokens;
                total_output += u.output_tokens;
                if let Some(c) = u.effective_cost() {
                    total_cost += c;
                    has_cost = true;
                }
            }
        }

        // Include cache tokens in the input total for display (cache write + read)
        let total_cache: u64 = self
            .messages
            .iter()
            .filter_map(|m| m.usage.as_ref())
            .map(|u| u.cache_read_tokens + u.cache_write_tokens)
            .sum();
        self.session.total_input_tokens = total_input + total_cache;
        self.session.total_output_tokens = total_output;
        if has_cost {
            self.session.total_cost_usd = Some(total_cost);
        }
        self.session.message_count = self.messages.len();

        // Infer timestamps from messages
        let timestamps: Vec<DateTime<Utc>> = self.messages.iter().filter_map(|m| m.ts).collect();
        if !timestamps.is_empty() {
            if self.session.started_at.is_none() {
                self.session.started_at = timestamps.iter().copied().min();
            }
            if self.session.ended_at.is_none() {
                self.session.ended_at = timestamps.iter().copied().max();
            }
        }

        // Pick the most common model
        if self.session.model.is_none() {
            let mut models: Vec<&str> = self
                .messages
                .iter()
                .filter_map(|m| m.model.as_deref())
                .collect();
            if !models.is_empty() {
                models.sort();
                // Most frequent
                let mut best = models[0];
                let mut best_count = 1usize;
                let mut cur = models[0];
                let mut cur_count = 1usize;
                for m in &models[1..] {
                    if *m == cur {
                        cur_count += 1;
                    } else {
                        if cur_count > best_count {
                            best = cur;
                            best_count = cur_count;
                        }
                        cur = m;
                        cur_count = 1;
                    }
                }
                if cur_count > best_count {
                    best = cur;
                }
                self.session.model = Some(best.to_string());
            }
        }
    }
}

/// A finding from the inefficiency detector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub description: String,
    pub evidence: Vec<String>,
    pub wasted_tokens: Option<u64>,
    pub wasted_cost_usd: Option<f64>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    RetryLoop,
    EditCascade,
    ToolFanout,
    RedundantReread,
    ContextBloat,
    ErrorRepromptChurn,
    SubagentOverhead,
}

impl std::fmt::Display for FindingKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingKind::RetryLoop => write!(f, "RETRY_LOOP"),
            FindingKind::EditCascade => write!(f, "EDIT_CASCADE"),
            FindingKind::ToolFanout => write!(f, "TOOL_FANOUT"),
            FindingKind::RedundantReread => write!(f, "REDUNDANT_REREAD"),
            FindingKind::ContextBloat => write!(f, "CONTEXT_BLOAT"),
            FindingKind::ErrorRepromptChurn => write!(f, "ERROR_REPROMPT_CHURN"),
            FindingKind::SubagentOverhead => write!(f, "SUBAGENT_OVERHEAD"),
        }
    }
}

/// Full analysis result for a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub session: CanonicalSession,
    pub findings: Vec<Finding>,
    pub top_expensive_messages: Vec<ExpensiveMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpensiveMessage {
    pub message_id: String,
    pub sequence: usize,
    pub role: Role,
    pub model: Option<String>,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_count: usize,
}
