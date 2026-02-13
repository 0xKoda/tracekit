use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;

mod commands;
use commands::{analyze, capture, list, report};

#[derive(Parser)]
#[command(
    name = "tracekit",
    version = "0.1.0",
    author,
    about = "Analyze coding-agent session traces for token/cost inefficiencies",
    long_about = r#"tracekit imports session traces from coding agents (Claude Code, OpenCode, Codex),
identifies inefficient token/cost usage patterns, and outputs actionable optimization reports.

Supported agents: claude, opencode, codex, pi, kodo, all

Quick start:
  tracekit list sessions                        # list all sessions across agents
  tracekit analyze recent --limit 5             # analyze 5 most recent sessions
  tracekit analyze expensive --top 10           # find 10 most expensive sessions
  tracekit report session --session-id <id>     # full report for one session
  tracekit report aggregate --format html       # HTML report across all sessions"#
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Discover and cache traces from coding agents
    Capture(capture::CaptureArgs),

    /// List sessions from one or more agents
    List(list::ListArgs),

    /// Analyze sessions for inefficiencies and cost
    Analyze(analyze::AnalyzeArgs),

    /// Generate reports (terminal/JSON/HTML)
    Report(report::ReportArgs),
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("{}: {:#}", "error".red().bold(), e);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Capture(args) => capture::run(args),
        Commands::List(args) => list::run(args),
        Commands::Analyze(args) => analyze::run(args),
        Commands::Report(args) => report::run(args),
    }
}
