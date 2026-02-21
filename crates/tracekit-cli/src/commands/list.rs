use anyhow::Result;
use clap::{Args, Subcommand};
use tracekit_ingest as ingest;
use tracekit_report::terminal;

use super::{parse_agents, parse_datetime};

#[derive(Args)]
pub struct ListArgs {
    #[command(subcommand)]
    pub subcommand: ListSubcommand,
}

#[derive(Subcommand)]
pub enum ListSubcommand {
    /// List sessions
    Sessions {
        /// Agent filter: claude, opencode, codex, all
        #[arg(long, default_value = "all")]
        agent: String,

        /// Only sessions after this time (ISO 8601, e.g. 2026-01-01)
        #[arg(long)]
        since: Option<String>,

        /// Only sessions before this time (ISO 8601)
        #[arg(long)]
        until: Option<String>,

        /// Filter by CWD substring
        #[arg(long)]
        cwd: Option<String>,

        /// Filter by model ID substring
        #[arg(long)]
        model_id: Option<String>,

        /// Limit results
        #[arg(long)]
        limit: Option<usize>,

        /// Sort by: date (default), cost, messages, agent
        #[arg(long, default_value = "date")]
        sort: String,

        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },
}

pub fn run(args: ListArgs) -> Result<()> {
    match args.subcommand {
        ListSubcommand::Sessions {
            agent,
            since,
            until,
            cwd,
            model_id,
            limit,
            sort,
            format,
        } => {
            let agents = parse_agents(&agent)?;
            let since_dt = since.as_deref().map(parse_datetime).transpose()?;
            let until_dt = until.as_deref().map(parse_datetime).transpose()?;

            let mut sessions = ingest::discover_sessions(
                &agents,
                since_dt,
                until_dt,
                cwd.as_deref(),
                None, // apply limit after sort
            )?;

            // Model filter (post-discovery)
            if let Some(mid) = &model_id {
                let mid_lower = mid.to_lowercase();
                sessions.retain(|s| {
                    s.model
                        .as_ref()
                        .map(|m| m.to_lowercase().contains(&mid_lower))
                        .unwrap_or(false)
                });
            }

            // Sort
            match sort.as_str() {
                "messages" | "msgs" => {
                    sessions.sort_by(|a, b| b.message_count.cmp(&a.message_count));
                }
                "cost" => {
                    sessions.sort_by(|a, b| {
                        b.total_cost_usd
                            .unwrap_or(0.0)
                            .partial_cmp(&a.total_cost_usd.unwrap_or(0.0))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                "agent" => {
                    sessions.sort_by(|a, b| {
                        a.source_agent.to_string().cmp(&b.source_agent.to_string())
                    });
                }
                _ => {} // "date" — already sorted newest-first by discover_sessions
            }

            if let Some(n) = limit {
                sessions.truncate(n);
            }

            match format.as_str() {
                "json" => {
                    println!("{}", tracekit_report::json::render_session_list(&sessions)?);
                }
                _ => {
                    terminal::print_session_list(&sessions);
                }
            }
        }
    }
    Ok(())
}
