use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use tracekit_core::{detect_inefficiencies, top_expensive_messages, AnalysisResult};
use tracekit_ingest as ingest;
use tracekit_report::{html as html_report, json as jreport, terminal};

use super::{parse_agents, parse_datetime};

#[derive(Args)]
pub struct ReportArgs {
    #[command(subcommand)]
    pub subcommand: ReportSubcommand,
}

#[derive(Subcommand)]
pub enum ReportSubcommand {
    /// Generate a report for a single session
    Session {
        /// Session ID (prefix match)
        #[arg(long)]
        session_id: String,

        /// Agent hint
        #[arg(long, default_value = "all")]
        agent: String,

        /// Output format: table, json, html
        #[arg(long, default_value = "table")]
        format: String,

        /// Output file (defaults to stdout for table/json, report.html for html)
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Generate an aggregate report across multiple sessions
    Aggregate {
        /// Agent filter
        #[arg(long, default_value = "all")]
        agent: String,

        /// Only sessions after this time
        #[arg(long)]
        since: Option<String>,

        /// Only sessions before this time
        #[arg(long)]
        until: Option<String>,

        /// Output format: table, json, html
        #[arg(long, default_value = "table")]
        format: String,

        /// Output file
        #[arg(long)]
        out: Option<PathBuf>,

        /// Limit number of sessions included
        #[arg(long)]
        limit: Option<usize>,
    },
}

fn analyze_one(session_id: &str, agent: &str) -> Result<AnalysisResult> {
    let agents = parse_agents(agent)?;
    let session = ingest::find_session(session_id, &agents)?
        .ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", session_id))?;

    eprintln!("{} Parsing {}...", "→".cyan(), &session.session_id[..8.min(session.session_id.len())]);
    let parsed = ingest::parse_session(&session)?;
    let findings = detect_inefficiencies(&parsed);
    let top = top_expensive_messages(&parsed, 10);

    Ok(AnalysisResult {
        session: parsed.session,
        findings,
        top_expensive_messages: top,
    })
}

fn write_or_print(content: &str, out: Option<&PathBuf>, default_file: &str) -> Result<()> {
    match out {
        Some(path) => {
            std::fs::write(path, content)?;
            eprintln!("{} Written to {}", "✓".green(), path.display());
        }
        None if content.starts_with("<!DOCTYPE") => {
            // HTML: write to default file
            let path = PathBuf::from(default_file);
            std::fs::write(&path, content)?;
            eprintln!("{} Written to {}", "✓".green(), path.display());
        }
        None => print!("{}", content),
    }
    Ok(())
}

pub fn run(args: ReportArgs) -> Result<()> {
    match args.subcommand {
        ReportSubcommand::Session {
            session_id,
            agent,
            format,
            out,
        } => {
            let result = analyze_one(&session_id, &agent)?;
            match format.as_str() {
                "json" => {
                    let content = jreport::render_analysis(&result)?;
                    write_or_print(&content, out.as_ref(), "report.json")?;
                }
                "html" => {
                    let content = html_report::render_analysis(&result)?;
                    write_or_print(&content, out.as_ref(), "report.html")?;
                }
                _ => {
                    terminal::print_analysis(&result);
                }
            }
        }

        ReportSubcommand::Aggregate {
            agent,
            since,
            until,
            format,
            out,
            limit,
        } => {
            let agents = parse_agents(&agent)?;
            let since_dt = since.as_deref().map(parse_datetime).transpose()?;
            let until_dt = until.as_deref().map(parse_datetime).transpose()?;

            let sessions = ingest::discover_sessions(&agents, since_dt, until_dt, None, limit)?;

            if sessions.is_empty() {
                println!("{}", "No sessions found.".yellow());
                return Ok(());
            }

            eprintln!("{} Analyzing {} sessions...", "→".cyan(), sessions.len());

            let results: Vec<AnalysisResult> = sessions.iter().filter_map(|s| {
                match ingest::parse_session(s) {
                    Ok(parsed) => {
                        let findings = detect_inefficiencies(&parsed);
                        let top = top_expensive_messages(&parsed, 5);
                        Some(AnalysisResult {
                            session: parsed.session,
                            findings,
                            top_expensive_messages: top,
                        })
                    }
                    Err(e) => {
                        eprintln!("  {} {}: {}", "!".yellow(), s.session_id, e);
                        None
                    }
                }
            }).collect();

            match format.as_str() {
                "json" => {
                    let content = jreport::render_aggregate(&results)?;
                    write_or_print(&content, out.as_ref(), "report.json")?;
                }
                "html" => {
                    let content = html_report::render_aggregate(&results)?;
                    write_or_print(&content, out.as_ref(), "report.html")?;
                }
                _ => {
                    terminal::print_aggregate(&results);
                }
            }
        }
    }
    Ok(())
}
