pub mod claude;
pub mod codex;
pub mod opencode;

use anyhow::Result;
use std::path::PathBuf;
use tracekit_core::{Agent, CanonicalSession, ParsedSession};

/// Discover all sessions for the given agent(s).
pub fn discover_sessions(
    agents: &[Agent],
    since: Option<chrono::DateTime<chrono::Utc>>,
    until: Option<chrono::DateTime<chrono::Utc>>,
    cwd_filter: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<CanonicalSession>> {
    let mut sessions = Vec::new();

    for agent in agents {
        let found = match agent {
            Agent::Claude => claude::discover_sessions()?,
            Agent::Opencode => opencode::discover_sessions()?,
            Agent::Codex => codex::discover_sessions()?,
            Agent::Pi => Vec::new(),   // TODO
            Agent::Kodo => Vec::new(), // TODO
        };
        sessions.extend(found);
    }

    // Apply filters
    if let Some(since) = since {
        sessions.retain(|s| s.started_at.map(|t| t >= since).unwrap_or(true));
    }
    if let Some(until) = until {
        sessions.retain(|s| s.started_at.map(|t| t <= until).unwrap_or(true));
    }
    if let Some(cwd) = cwd_filter {
        sessions.retain(|s| s.cwd.as_deref().map(|c| c.contains(cwd)).unwrap_or(false));
    }

    // Sort newest first
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    if let Some(n) = limit {
        sessions.truncate(n);
    }

    Ok(sessions)
}

/// Find a specific session by ID across all agents.
pub fn find_session(session_id: &str, agents: &[Agent]) -> Result<Option<CanonicalSession>> {
    let sessions = discover_sessions(agents, None, None, None, None)?;
    Ok(sessions
        .into_iter()
        .find(|s| s.session_id.starts_with(session_id)))
}

/// Fully parse a session (load all messages, compute totals).
pub fn parse_session(session: &CanonicalSession) -> Result<ParsedSession> {
    let mut parsed = match session.source_agent {
        Agent::Claude => claude::parse_session(session)?,
        Agent::Opencode => opencode::parse_session(session)?,
        Agent::Codex => codex::parse_session(session)?,
        _ => ParsedSession {
            session: session.clone(),
            messages: Vec::new(),
        },
    };
    parsed.compute_totals();
    Ok(parsed)
}

/// Resolve the default root path for an agent.
pub fn default_root(agent: Agent) -> Option<PathBuf> {
    let home = dirs_next();
    match agent {
        Agent::Claude => home.map(|h| h.join(".claude").join("projects")),
        Agent::Opencode => home.map(|h| {
            h.join(".local")
                .join("share")
                .join("opencode")
                .join("storage")
        }),
        Agent::Codex => home.map(|h| h.join(".codex").join("sessions")),
        Agent::Pi => home.map(|h| h.join(".pi").join("agent").join("sessions")),
        Agent::Kodo => home.map(|h| h.join(".kodo").join("sessions")),
    }
}

fn dirs_next() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Shorten a path for display purposes
pub fn short_path(path: &std::path::Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let s = path.to_string_lossy();
    if !home.is_empty() && s.starts_with(&home) {
        format!("~{}", &s[home.len()..])
    } else {
        s.to_string()
    }
}
