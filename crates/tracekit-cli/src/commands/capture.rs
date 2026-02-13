use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;
use tracekit_ingest::{self as ingest};

use super::parse_agents;

#[derive(Args)]
pub struct CaptureArgs {
    #[command(subcommand)]
    pub subcommand: CaptureSubcommand,
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
                println!("  {} {}  {}",
                    s.source_agent.to_string().cyan(),
                    s.session_id,
                    s.cwd.as_deref().unwrap_or("-").dimmed(),
                );
            }
        }
        CaptureSubcommand::Session { agent, session_id } => {
            let agents = parse_agents(&agent)?;
            match ingest::find_session(&session_id, &agents)? {
                Some(s) => {
                    println!("{} Found session", "✓".green());
                    println!("  Agent    : {}", s.source_agent.to_string().cyan());
                    println!("  ID       : {}", s.session_id);
                    println!("  Path     : {}", s.source_path.display());
                    println!("  CWD      : {}", s.cwd.as_deref().unwrap_or("-"));
                    println!("  Started  : {}", s.started_at.map(|t| t.to_string()).unwrap_or_else(|| "-".to_string()));
                }
                None => println!("{} No session found matching '{}'", "✗".red(), session_id),
            }
        }
    }
    Ok(())
}
