use anyhow::Result;
use tracekit_core::*;

pub fn render_analysis(result: &AnalysisResult) -> Result<String> {
    let s = &result.session;
    let findings_html = render_findings(&result.findings);
    let expensive_html = render_expensive_messages(&result.top_expensive_messages);
    let _data_json = serde_json::to_string(result)?;

    Ok(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>tracekit — {session_id}</title>
<style>
  :root {{
    --bg: #0f1117; --surface: #1a1d27; --border: #2a2d3a;
    --text: #e2e8f0; --muted: #64748b; --accent: #7c6af7;
    --green: #4ade80; --yellow: #facc15; --red: #f87171;
    --cyan: #22d3ee;
    font-family: 'Berkeley Mono', 'JetBrains Mono', 'Fira Code', monospace;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ background: var(--bg); color: var(--text); min-height: 100vh; }}
  .header {{ background: var(--surface); border-bottom: 1px solid var(--border);
    padding: 1.5rem 2rem; display: flex; align-items: center; gap: 1rem; }}
  .header h1 {{ font-size: 1.25rem; font-weight: 700; color: var(--accent); }}
  .header .agent-badge {{ background: var(--border); padding: 0.2rem 0.6rem;
    border-radius: 4px; font-size: 0.75rem; color: var(--cyan); }}
  .container {{ max-width: 1100px; margin: 0 auto; padding: 2rem; }}
  .kpi-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 1rem; margin-bottom: 2rem; }}
  .kpi {{ background: var(--surface); border: 1px solid var(--border);
    border-radius: 8px; padding: 1.25rem; }}
  .kpi .label {{ font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.1em;
    color: var(--muted); margin-bottom: 0.4rem; }}
  .kpi .value {{ font-size: 1.5rem; font-weight: 700; }}
  .kpi .value.green {{ color: var(--green); }}
  .kpi .value.yellow {{ color: var(--yellow); }}
  .kpi .value.cyan {{ color: var(--cyan); }}
  .section {{ background: var(--surface); border: 1px solid var(--border);
    border-radius: 8px; margin-bottom: 1.5rem; overflow: hidden; }}
  .section-header {{ padding: 0.875rem 1.25rem; border-bottom: 1px solid var(--border);
    font-size: 0.8rem; font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.08em; color: var(--muted); }}
  table {{ width: 100%; border-collapse: collapse; }}
  th, td {{ padding: 0.6rem 1.25rem; text-align: left; border-bottom: 1px solid var(--border);
    font-size: 0.85rem; }}
  th {{ font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.08em;
    color: var(--muted); }}
  tr:last-child td {{ border-bottom: none; }}
  tr:hover td {{ background: rgba(124,106,247,0.06); }}
  .finding {{ padding: 1rem 1.25rem; border-bottom: 1px solid var(--border); }}
  .finding:last-child {{ border-bottom: none; }}
  .finding-kind {{ display: inline-block; background: rgba(248,113,113,0.15);
    color: var(--red); padding: 0.1rem 0.5rem; border-radius: 3px;
    font-size: 0.75rem; font-weight: 700; margin-right: 0.5rem; }}
  .finding-desc {{ font-size: 0.9rem; display: inline; }}
  .finding-meta {{ font-size: 0.75rem; color: var(--muted); margin-top: 0.4rem; }}
  .finding-evidence {{ font-size: 0.75rem; color: var(--muted); margin-top: 0.3rem;
    padding-left: 0.75rem; }}
  .badge-green {{ color: var(--green); }}
  .badge-yellow {{ color: var(--yellow); }}
  .badge-red {{ color: var(--red); }}
  .no-findings {{ padding: 1.25rem; color: var(--green); font-size: 0.9rem; }}
  .meta-table {{ padding: 1rem 1.25rem; }}
  .meta-table tr td:first-child {{ color: var(--muted); width: 140px; font-size: 0.8rem; }}
  .meta-table tr td {{ border: none; padding: 0.25rem 0; }}
  footer {{ text-align: center; padding: 2rem; color: var(--muted); font-size: 0.75rem; }}
</style>
</head>
<body>
<div class="header">
  <h1>tracekit</h1>
  <span class="agent-badge">{agent}</span>
  <span style="color:var(--muted);font-size:0.85rem">{session_id}</span>
</div>
<div class="container">

  <!-- KPIs -->
  <div class="kpi-grid">
    <div class="kpi"><div class="label">Total Cost</div><div class="value green">{total_cost}</div></div>
    <div class="kpi"><div class="label">Messages</div><div class="value cyan">{message_count}</div></div>
    <div class="kpi"><div class="label">Input Tokens</div><div class="value">{input_tokens}</div></div>
    <div class="kpi"><div class="label">Output Tokens</div><div class="value">{output_tokens}</div></div>
    <div class="kpi"><div class="label">Duration</div><div class="value yellow">{duration}</div></div>
    <div class="kpi"><div class="label">Findings</div><div class="value {findings_color}">{findings_count}</div></div>
  </div>

  <!-- Session Metadata -->
  <div class="section">
    <div class="section-header">Session Metadata</div>
    <table class="meta-table">
      <tr><td>Agent</td><td>{agent}</td></tr>
      <tr><td>Model</td><td>{model}</td></tr>
      <tr><td>CWD</td><td>{cwd}</td></tr>
      <tr><td>Started</td><td>{started_at}</td></tr>
      <tr><td>Source</td><td>{source_path}</td></tr>
    </table>
  </div>

  <!-- Expensive Generations -->
  <div class="section">
    <div class="section-header">Top Expensive Generations</div>
    {expensive_html}
  </div>

  <!-- Findings -->
  <div class="section">
    <div class="section-header">Inefficiency Findings</div>
    {findings_html}
  </div>

</div>
<footer>Generated by tracekit · {timestamp}</footer>
</body>
</html>"#,
        session_id = &s.session_id,
        agent = s.source_agent,
        total_cost = fmt_cost_html(s.total_cost_usd),
        message_count = s.message_count,
        input_tokens = fmt_tokens(s.total_input_tokens),
        output_tokens = fmt_tokens(s.total_output_tokens),
        duration = fmt_duration(s.duration_secs()),
        findings_count = result.findings.len(),
        findings_color = if result.findings.is_empty() { "badge-green" } else { "badge-yellow" },
        model = s.model.as_deref().unwrap_or("-"),
        cwd = s.cwd.as_deref().unwrap_or("-"),
        started_at = fmt_ts(s.started_at),
        source_path = s.source_path.display(),
        findings_html = findings_html,
        expensive_html = expensive_html,
        timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    ))
}

pub fn render_aggregate(results: &[AnalysisResult]) -> Result<String> {
    let total_cost: f64 = results.iter()
        .filter_map(|r| r.session.total_cost_usd)
        .sum();
    let total_msgs: usize = results.iter().map(|r| r.session.message_count).sum();
    let total_findings: usize = results.iter().map(|r| r.findings.len()).sum();

    let sessions_html = results.iter().map(|r| {
        let s = &r.session;
        format!(
            r#"<tr>
              <td>{}</td>
              <td style="color:var(--cyan)">{}</td>
              <td style="color:var(--green)">{}</td>
              <td>{}</td>
              <td>{}</td>
              <td>{}</td>
              <td style="color:var(--red)">{}</td>
            </tr>"#,
            s.source_agent,
            truncate(&s.session_id, 36),
            fmt_cost_html(s.total_cost_usd),
            s.cwd.as_deref().unwrap_or("-"),
            fmt_ts(s.started_at),
            s.message_count,
            r.findings.len(),
        )
    }).collect::<String>();

    Ok(format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>tracekit — Aggregate Report</title>
<style>
  :root {{
    --bg: #0f1117; --surface: #1a1d27; --border: #2a2d3a;
    --text: #e2e8f0; --muted: #64748b; --accent: #7c6af7;
    --green: #4ade80; --yellow: #facc15; --red: #f87171;
    --cyan: #22d3ee;
    font-family: 'Berkeley Mono', 'JetBrains Mono', 'Fira Code', monospace;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ background: var(--bg); color: var(--text); }}
  .header {{ background: var(--surface); border-bottom: 1px solid var(--border);
    padding: 1.5rem 2rem; display: flex; align-items: center; gap: 1rem; }}
  .header h1 {{ font-size: 1.25rem; font-weight: 700; color: var(--accent); }}
  .container {{ max-width: 1200px; margin: 0 auto; padding: 2rem; }}
  .kpi-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 1rem; margin-bottom: 2rem; }}
  .kpi {{ background: var(--surface); border: 1px solid var(--border);
    border-radius: 8px; padding: 1.25rem; }}
  .kpi .label {{ font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.1em;
    color: var(--muted); margin-bottom: 0.4rem; }}
  .kpi .value {{ font-size: 1.5rem; font-weight: 700; }}
  .kpi .value.green {{ color: var(--green); }}
  .kpi .value.yellow {{ color: var(--yellow); }}
  .section {{ background: var(--surface); border: 1px solid var(--border);
    border-radius: 8px; margin-bottom: 1.5rem; overflow: hidden; }}
  .section-header {{ padding: 0.875rem 1.25rem; border-bottom: 1px solid var(--border);
    font-size: 0.8rem; font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.08em; color: var(--muted); }}
  table {{ width: 100%; border-collapse: collapse; }}
  th, td {{ padding: 0.6rem 1.25rem; text-align: left; border-bottom: 1px solid var(--border);
    font-size: 0.85rem; }}
  th {{ font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.08em;
    color: var(--muted); }}
  tr:last-child td {{ border-bottom: none; }}
  tr:hover td {{ background: rgba(124,106,247,0.06); }}
  footer {{ text-align: center; padding: 2rem; color: var(--muted); font-size: 0.75rem; }}
</style>
</head>
<body>
<div class="header"><h1>tracekit — Aggregate Report</h1></div>
<div class="container">
  <div class="kpi-grid">
    <div class="kpi"><div class="label">Total Cost</div><div class="value green">${total_cost:.4}</div></div>
    <div class="kpi"><div class="label">Sessions</div><div class="value">{session_count}</div></div>
    <div class="kpi"><div class="label">Messages</div><div class="value">{total_msgs}</div></div>
    <div class="kpi"><div class="label">Findings</div><div class="value yellow">{total_findings}</div></div>
  </div>
  <div class="section">
    <div class="section-header">Sessions</div>
    <table>
      <thead><tr>
        <th>Agent</th><th>Session ID</th><th>Cost</th>
        <th>CWD</th><th>Started</th><th>Messages</th><th>Findings</th>
      </tr></thead>
      <tbody>{sessions_html}</tbody>
    </table>
  </div>
</div>
<footer>Generated by tracekit · {timestamp}</footer>
</body>
</html>"#,
        total_cost = total_cost,
        session_count = results.len(),
        total_msgs = total_msgs,
        total_findings = total_findings,
        sessions_html = sessions_html,
        timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    ))
}

fn render_findings(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return r#"<div class="no-findings">✓ No inefficiencies detected.</div>"#.to_string();
    }

    findings.iter().enumerate().map(|(_i, f)| {
        let evidence_html = f.evidence.iter().take(4)
            .map(|e| format!(r#"<div class="finding-evidence">· {}</div>"#, html_escape(e)))
            .collect::<String>();

        let waste_html = f.wasted_cost_usd
            .filter(|&c| c > 0.0)
            .map(|c| format!(r#" <span class="badge-yellow">~{} wasted</span>"#, fmt_cost_html(Some(c))))
            .unwrap_or_default();

        format!(
            r#"<div class="finding">
              <span class="finding-kind">{kind}</span>
              <span class="finding-desc">{desc}</span>{waste}
              <div class="finding-meta">confidence: {conf:.0}%</div>
              {evidence}
            </div>"#,
            kind = f.kind,
            desc = html_escape(&f.description),
            waste = waste_html,
            conf = f.confidence * 100.0,
            evidence = evidence_html,
        )
    }).collect()
}

fn render_expensive_messages(messages: &[ExpensiveMessage]) -> String {
    if messages.is_empty() {
        return r#"<div style="padding:1.25rem;color:var(--muted)">No cost data available.</div>"#.to_string();
    }

    let rows = messages.iter().map(|m| {
        format!(
            r#"<tr>
              <td>{}</td>
              <td style="color:var(--green)">{}</td>
              <td>{}</td>
              <td>{}</td>
              <td>{}</td>
            </tr>"#,
            m.sequence,
            fmt_cost_html(Some(m.cost_usd)),
            fmt_tokens(m.input_tokens),
            fmt_tokens(m.output_tokens),
            m.tool_count,
        )
    }).collect::<String>();

    format!(
        r#"<table>
          <thead><tr>
            <th>Turn</th><th>Cost</th><th>Input Tokens</th><th>Output Tokens</th><th>Tool Calls</th>
          </tr></thead>
          <tbody>{}</tbody>
        </table>"#,
        rows
    )
}

fn fmt_cost_html(cost: Option<f64>) -> String {
    match cost {
        Some(c) => format!("${:.4}", c),
        None => "-".to_string(),
    }
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_duration(secs: Option<i64>) -> String {
    match secs {
        None => "-".to_string(),
        Some(s) if s < 60 => format!("{}s", s),
        Some(s) if s < 3600 => format!("{}m{}s", s / 60, s % 60),
        Some(s) => format!("{}h{}m", s / 3600, (s % 3600) / 60),
    }
}

fn fmt_ts(ts: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match ts {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => "-".to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
