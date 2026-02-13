use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;
use tracekit_core::{detect_inefficiencies, top_expensive_messages, AnalysisResult};
use tracekit_ingest as ingest;
use tracekit_report::{html as html_report, json as jreport, terminal};

use super::{parse_agents, parse_datetime};

#[derive(Args)]
pub struct AnalyzeArgs {
    #[command(subcommand)]
    pub subcommand: AnalyzeSubcommand,
}

#[derive(Subcommand)]
pub enum AnalyzeSubcommand {
    /// Analyze a specific session by ID
    Session {
        /// Session ID (prefix match)
        #[arg(long)]
        session_id: String,

        /// Agent hint for faster lookup
        #[arg(long, default_value = "all")]
        agent: String,

        /// Optimization target: cost, latency, reliability
        #[arg(long, default_value = "cost")]
        optimize_for: String,

        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Analyze N most recent sessions
    Recent {
        /// Agent filter
        #[arg(long, default_value = "all")]
        agent: String,

        /// Number of sessions to analyze
        #[arg(long, default_value = "10")]
        limit: usize,

        /// Only sessions after this time
        #[arg(long)]
        since: Option<String>,

        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Find and analyze the most expensive sessions
    Expensive {
        /// Agent filter
        #[arg(long, default_value = "all")]
        agent: String,

        /// How many top sessions to show
        #[arg(long, default_value = "10")]
        top: usize,

        /// Only sessions after this time
        #[arg(long)]
        since: Option<String>,

        /// Output format: table, json
        #[arg(long, default_value = "table")]
        format: String,
    },
}

fn analyze_session_by_id(session_id: &str, agent: &str, top_n: usize) -> Result<AnalysisResult> {
    let agents = parse_agents(agent)?;
    let session = ingest::find_session(session_id, &agents)?
        .ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", session_id))?;

    eprintln!("{} Parsing session {}...", "→".cyan(), &session.session_id[..8.min(session.session_id.len())]);
    let parsed = ingest::parse_session(&session)?;
    let findings = detect_inefficiencies(&parsed);
    let top_expensive = top_expensive_messages(&parsed, top_n);

    Ok(AnalysisResult {
        session: parsed.session,
        findings,
        top_expensive_messages: top_expensive,
    })
}

pub fn run(args: AnalyzeArgs) -> Result<()> {
    match args.subcommand {
        AnalyzeSubcommand::Session {
            session_id,
            agent,
            optimize_for: _,
            format,
        } => {
            let result = analyze_session_by_id(&session_id, &agent, 10)?;
            match format.as_str() {
                "json" => println!("{}", jreport::render_analysis(&result)?),
                "html" => {
                    let content = html_report::render_analysis(&result)?;
                    let out = format!("report-{}.html", &session_id[..8.min(session_id.len())]);
                    std::fs::write(&out, &content)?;
                    eprintln!("{} Written to {}", "✓".green(), out);
                    // Also print summary to terminal
                    terminal::print_analysis(&result);
                }
                _ => terminal::print_analysis(&result),
            }
        }

        AnalyzeSubcommand::Recent {
            agent,
            limit,
            since,
            format,
        } => {
            let agents = parse_agents(&agent)?;
            let since_dt = since.as_deref().map(parse_datetime).transpose()?;
            let sessions = ingest::discover_sessions(&agents, since_dt, None, None, Some(limit))?;

            if sessions.is_empty() {
                println!("{}", "No sessions found.".yellow());
                return Ok(());
            }

            eprintln!("{} Analyzing {} sessions...", "→".cyan(), sessions.len());

            let results: Vec<AnalysisResult> = sessions.iter().map(|s| {
                let parsed = match ingest::parse_session(s) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("  {} {}: {}", "!".yellow(), s.session_id, e);
                        return AnalysisResult {
                            session: s.clone(),
                            findings: Vec::new(),
                            top_expensive_messages: Vec::new(),
                        };
                    }
                };
                let findings = detect_inefficiencies(&parsed);
                let top = top_expensive_messages(&parsed, 3);
                AnalysisResult {
                    session: parsed.session,
                    findings,
                    top_expensive_messages: top,
                }
            }).collect();

            match format.as_str() {
                "json" => println!("{}", jreport::render_aggregate(&results)?),
                _ => terminal::print_aggregate(&results),
            }
        }

        AnalyzeSubcommand::Expensive {
            agent,
            top,
            since,
            format,
        } => {
            let agents = parse_agents(&agent)?;
            let since_dt = since.as_deref().map(parse_datetime).transpose()?;

            // We need to parse all sessions to find cost, then take top N
            let sessions = ingest::discover_sessions(&agents, since_dt, None, None, None)?;

            if sessions.is_empty() {
                println!("{}", "No sessions found.".yellow());
                return Ok(());
            }

            eprintln!("{} Analyzing {} sessions...", "→".cyan(), sessions.len());

            let mut results: Vec<AnalysisResult> = sessions.iter().filter_map(|s| {
                let parsed = ingest::parse_session(s).ok()?;
                let findings = detect_inefficiencies(&parsed);
                let top_msgs = top_expensive_messages(&parsed, 5);
                Some(AnalysisResult {
                    session: parsed.session,
                    findings,
                    top_expensive_messages: top_msgs,
                })
            }).collect();

            // Sort by cost descending
            results.sort_by(|a, b| {
                b.session.total_cost_usd.unwrap_or(0.0)
                    .partial_cmp(&a.session.total_cost_usd.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            results.truncate(top);

            match format.as_str() {
                "json" => println!("{}", jreport::render_aggregate(&results)?),
                _ => terminal::print_expensive_sessions(&results, top),
            }
        }
    }
    Ok(())
}
