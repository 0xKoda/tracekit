pub mod analyze;
pub mod capture;
pub mod list;
pub mod report;

use anyhow::Result;
use tracekit_core::Agent;

/// Parse an agent filter string into a list of agents.
pub fn parse_agents(agent: &str) -> Result<Vec<Agent>> {
    match agent.to_lowercase().as_str() {
        "all" => Ok(vec![Agent::Claude, Agent::Opencode, Agent::Codex]),
        other => {
            let a: Agent = other.parse()?;
            Ok(vec![a])
        }
    }
}

/// Parse an ISO 8601 datetime string.
pub fn parse_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    s.parse::<chrono::DateTime<chrono::Utc>>()
        .or_else(|_| {
            // Try date-only
            let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
            Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc())
        })
        .map_err(|e: anyhow::Error| e)
}
