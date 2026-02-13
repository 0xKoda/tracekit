use colored::Colorize;
use tracekit_core::*;

// ── formatting helpers ────────────────────────────────────────────────────────

pub fn fmt_cost(cost: Option<f64>) -> String {
    match cost {
        Some(c) => format!("${:.4}", c),
        None => "-".to_string(),
    }
}

pub fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn fmt_duration(secs: Option<i64>) -> String {
    match secs {
        None => "-".to_string(),
        Some(s) if s < 60 => format!("{}s", s),
        Some(s) if s < 3600 => format!("{}m{}s", s / 60, s % 60),
        Some(s) => format!("{}h{}m", s / 3600, (s % 3600) / 60),
    }
}

pub fn fmt_ts(ts: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match ts {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => "-".to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

// ── session list ──────────────────────────────────────────────────────────────

pub fn print_session_list(sessions: &[CanonicalSession]) {
    if sessions.is_empty() {
        println!("{}", "No sessions found.".yellow());
        return;
    }

    let col_widths = (8, 38, 32, 17, 5, 10);
    let (w_agent, w_id, w_cwd, w_ts, w_msgs, w_cost) = col_widths;

    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:>w4$}  {:>w5$}",
        "AGENT".bold(),
        "SESSION ID".bold(),
        "CWD".bold(),
        "STARTED".bold(),
        "MSGS".bold(),
        "COST".bold(),
        w0 = w_agent,
        w1 = w_id,
        w2 = w_cwd,
        w3 = w_ts,
        w4 = w_msgs,
        w5 = w_cost,
    );
    println!("{}", "─".repeat(w_agent + w_id + w_cwd + w_ts + w_msgs + w_cost + 10));

    for s in sessions {
        let cwd_display = s.cwd.as_deref()
            .map(|c| {
                let home = std::env::var("HOME").unwrap_or_default();
                if !home.is_empty() && c.starts_with(&home) {
                    format!("~{}", &c[home.len()..])
                } else {
                    c.to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());

        let agent_colored = match s.source_agent {
            Agent::Claude => s.source_agent.to_string().cyan().to_string(),
            Agent::Opencode => s.source_agent.to_string().green().to_string(),
            Agent::Codex => s.source_agent.to_string().yellow().to_string(),
            Agent::Pi => s.source_agent.to_string().magenta().to_string(),
            Agent::Kodo => s.source_agent.to_string().blue().to_string(),
        };

        println!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:>w4$}  {:>w5$}",
            agent_colored,
            truncate(&s.session_id, w_id),
            truncate(&cwd_display, w_cwd),
            fmt_ts(s.started_at),
            s.message_count,
            fmt_cost(s.total_cost_usd),
            w0 = w_agent,
            w1 = w_id,
            w2 = w_cwd,
            w3 = w_ts,
            w4 = w_msgs,
            w5 = w_cost,
        );
    }
    println!("\n{} sessions", sessions.len());
}

// ── analysis result ───────────────────────────────────────────────────────────

pub fn print_analysis(result: &AnalysisResult) {
    let s = &result.session;

    println!("\n{}", "── Session ─────────────────────────────────────────────────────".bold());
    println!("  Agent      : {}", s.source_agent.to_string().cyan());
    println!("  Session ID : {}", s.session_id);
    println!("  Path       : {}", s.source_path.display());
    if let Some(cwd) = &s.cwd {
        println!("  CWD        : {}", cwd);
    }
    if let Some(model) = &s.model {
        println!("  Model      : {}", model);
    }
    println!("  Started    : {}", fmt_ts(s.started_at));
    println!("  Duration   : {}", fmt_duration(s.duration_secs()));
    println!("  Messages   : {}", s.message_count);
    println!("  Input tok  : {}", fmt_tokens(s.total_input_tokens));
    println!("  Output tok : {}", fmt_tokens(s.total_output_tokens));
    println!("  Total cost : {}", fmt_cost(s.total_cost_usd).green().bold().to_string());

    let total_waste: f64 = result.findings.iter()
        .filter_map(|f| f.wasted_cost_usd)
        .sum();
    if total_waste > 0.0 {
        println!("  Identified waste : {}", format!("~${:.2}", total_waste).red().bold().to_string());
    }

    // Top expensive messages
    if !result.top_expensive_messages.is_empty() {
        println!("\n{}", "── Top Expensive Generations ───────────────────────────────────".bold());
        for (i, m) in result.top_expensive_messages.iter().enumerate() {
            println!(
                "  {}. turn {:>4}  {:>10}  in:{:>8}  out:{:>7}  tools:{}",
                i + 1,
                m.sequence,
                fmt_cost(Some(m.cost_usd)).yellow(),
                fmt_tokens(m.input_tokens),
                fmt_tokens(m.output_tokens),
                m.tool_count,
            );
        }
    }

    // Findings
    if result.findings.is_empty() {
        println!("\n{}", "No inefficiency findings.".green());
    } else {
        println!("\n{}", "── Inefficiency Findings ───────────────────────────────────────".bold());
        for (i, f) in result.findings.iter().enumerate() {
            let kind_str = format!("[{}]", f.kind).red().bold().to_string();
            let conf = format!("(conf {:.0}%)", f.confidence * 100.0).dimmed();
            let waste = match f.wasted_cost_usd {
                Some(c) if c > 0.0 => format!(" ~{} wasted", fmt_cost(Some(c))).yellow().to_string(),
                _ => String::new(),
            };
            println!("\n  {}. {} {}{}", i + 1, kind_str, conf, waste);
            println!("     {}", f.description);
            for ev in f.evidence.iter().take(3) {
                println!("       · {}", ev.dimmed());
            }
        }
    }

    println!();
}

// ── aggregate summary ─────────────────────────────────────────────────────────

pub fn print_aggregate(results: &[AnalysisResult]) {
    if results.is_empty() {
        println!("{}", "No results.".yellow());
        return;
    }

    println!("\n{}", "── Aggregate Summary ───────────────────────────────────────────".bold());
    let total_cost: f64 = results.iter()
        .filter_map(|r| r.session.total_cost_usd)
        .sum();
    let total_msgs: usize = results.iter().map(|r| r.session.message_count).sum();
    let total_findings: usize = results.iter().map(|r| r.findings.len()).sum();

    println!("  Sessions analyzed : {}", results.len());
    println!("  Total messages    : {}", total_msgs);
    println!("  Total cost        : {}", fmt_cost(Some(total_cost)).green().bold().to_string());
    println!("  Total findings    : {}", total_findings);

    println!("\n{}", "── Top Sessions by Cost ────────────────────────────────────────".bold());
    let mut sorted: Vec<&AnalysisResult> = results.iter().collect();
    sorted.sort_by(|a, b| {
        b.session.total_cost_usd.unwrap_or(0.0)
            .partial_cmp(&a.session.total_cost_usd.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, r) in sorted.iter().take(10).enumerate() {
        let s = &r.session;
        let cwd_display = s.cwd.as_deref().unwrap_or("-");
        println!(
            "  {}. {:>10}  {:>8}  {}  {}",
            i + 1,
            fmt_cost(s.total_cost_usd).yellow(),
            s.source_agent.to_string().cyan(),
            truncate(&s.session_id, 36),
            truncate(cwd_display, 40).dimmed(),
        );
    }

    // Most common finding types
    let mut finding_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in results {
        for f in &r.findings {
            *finding_counts.entry(f.kind.to_string()).or_default() += 1;
        }
    }
    if !finding_counts.is_empty() {
        println!("\n{}", "── Most Common Inefficiencies ──────────────────────────────────".bold());
        let mut counts: Vec<(String, usize)> = finding_counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        for (kind, count) in counts.iter().take(7) {
            println!("  {:<30}  {}", kind.red(), count);
        }
    }

    println!();
}

pub fn print_expensive_sessions(results: &[AnalysisResult], top_n: usize) {
    let mut sorted: Vec<&AnalysisResult> = results.iter().collect();
    sorted.sort_by(|a, b| {
        b.session.total_cost_usd.unwrap_or(0.0)
            .partial_cmp(&a.session.total_cost_usd.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted.truncate(top_n);

    println!("\n{}", "── Most Expensive Sessions ─────────────────────────────────────".bold());
    for (i, r) in sorted.iter().enumerate() {
        print_analysis(r);
        if i < sorted.len() - 1 {
            println!("{}", "────────────────────────────────────────────────────────────────".dimmed());
        }
    }
}
