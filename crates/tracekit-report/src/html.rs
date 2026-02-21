use anyhow::Result;
use tracekit_core::*;

pub fn render_analysis(result: &AnalysisResult) -> Result<String> {
    let s = &result.session;
    let findings_html = render_findings(&result.findings);
    let expensive_html = render_expensive_messages(&result.top_expensive_messages);

    // Total identified waste
    let total_waste: f64 = result
        .findings
        .iter()
        .filter_map(|f| f.wasted_cost_usd)
        .sum();
    let waste_display = if total_waste > 0.0 {
        format!("${:.2}", total_waste)
    } else {
        "—".to_string()
    };
    let waste_class = if total_waste >= 5.0 {
        "danger"
    } else if total_waste > 0.0 {
        "warn"
    } else {
        "muted"
    };

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>tracekit — {session_id}</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=JetBrains+Mono:wght@400;500;700&display=swap" rel="stylesheet">
<style>
  :root {{
    /* Base — deep navy-black, not pure black. Cooler undertone. */
    --bg:        #07080e;
    --surface:   #0d0f1a;
    --surface-2: #121520;
    --border:    #1c2035;
    --border-2:  #252942;

    /* Typography */
    --text:      #dde3f0;
    --text-2:    #8892aa;
    --text-3:    #4a5270;

    /* Accent palette — analogous indigo family */
    --accent:    #6366f1;   /* indigo — primary action */
    --accent-dim:#2e3168;   /* indigo dim — badge bg */

    /* Semantic — complementary triad */
    --success:   #34d399;   /* emerald green — good state */
    --warn:      #f59e0b;   /* amber — caution */
    --danger:    #f87171;   /* rose red — critical */
    --info:      #38bdf8;   /* sky blue — neutral info */

    /* Semantic dim variants (for badge backgrounds) */
    --success-dim: rgba(52,211,153,0.12);
    --warn-dim:    rgba(245,158,11,0.12);
    --danger-dim:  rgba(248,113,113,0.14);
    --info-dim:    rgba(56,189,248,0.10);
    --accent-dim2: rgba(99,102,241,0.10);

    --font-ui:   'Inter', system-ui, sans-serif;
    --font-mono: 'JetBrains Mono', 'Fira Code', monospace;
    --radius:    6px;
    --radius-lg: 10px;
  }}

  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  html {{ font-size: 14px; }}
  body {{
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-ui);
    min-height: 100vh;
    line-height: 1.5;
  }}

  /* ── Header ─────────────────────────────────────────── */
  .header {{
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    padding: 1rem 2rem;
    display: flex;
    align-items: center;
    gap: 0.875rem;
  }}
  .header-logo {{
    font-family: var(--font-mono);
    font-size: 1rem;
    font-weight: 700;
    color: var(--accent);
    letter-spacing: -0.01em;
  }}
  .header-sep {{ color: var(--border-2); }}
  .badge {{
    display: inline-flex;
    align-items: center;
    padding: 0.15rem 0.5rem;
    border-radius: 4px;
    font-family: var(--font-mono);
    font-size: 0.7rem;
    font-weight: 500;
    letter-spacing: 0.02em;
    background: var(--accent-dim2);
    color: var(--accent);
    border: 1px solid rgba(99,102,241,0.2);
  }}
  .session-id {{
    font-family: var(--font-mono);
    font-size: 0.75rem;
    color: var(--text-3);
  }}

  /* ── Layout ─────────────────────────────────────────── */
  .container {{
    max-width: 1080px;
    margin: 0 auto;
    padding: 1.75rem 2rem;
  }}

  /* ── KPI Bento Grid ──────────────────────────────────── */
  .kpi-grid {{
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 0.75rem;
    margin-bottom: 1.5rem;
  }}
  .kpi {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 1.125rem 1.25rem;
    position: relative;
    overflow: hidden;
  }}
  .kpi::before {{
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(135deg, rgba(255,255,255,0.015) 0%, transparent 60%);
    pointer-events: none;
  }}
  .kpi-label {{
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-3);
    margin-bottom: 0.5rem;
  }}
  .kpi-value {{
    font-family: var(--font-mono);
    font-size: 1.5rem;
    font-weight: 700;
    line-height: 1;
    color: var(--text);
  }}
  .kpi-value.accent  {{ color: var(--accent); }}
  .kpi-value.success {{ color: var(--success); }}
  .kpi-value.warn    {{ color: var(--warn); }}
  .kpi-value.danger  {{ color: var(--danger); }}
  .kpi-value.info    {{ color: var(--info); }}
  .kpi-value.muted   {{ color: var(--text-2); }}

  /* Waste KPI — emphasis card */
  .kpi.kpi-waste {{
    border-color: rgba(248,113,113,0.25);
    background: linear-gradient(135deg, rgba(248,113,113,0.06) 0%, var(--surface) 60%);
  }}

  /* ── Sections ────────────────────────────────────────── */
  .section {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    margin-bottom: 1rem;
    overflow: hidden;
  }}
  .section-header {{
    padding: 0.75rem 1.25rem;
    border-bottom: 1px solid var(--border);
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-3);
    background: var(--surface-2);
  }}

  /* ── Meta table ──────────────────────────────────────── */
  .meta-grid {{
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 0;
    padding: 0.25rem 0;
  }}
  .meta-grid dt, .meta-grid dd {{
    padding: 0.35rem 1.25rem;
    font-size: 0.8rem;
    line-height: 1.4;
  }}
  .meta-grid dt {{
    color: var(--text-3);
    font-weight: 500;
  }}
  .meta-grid dd {{
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    word-break: break-all;
  }}

  /* ── Data table ──────────────────────────────────────── */
  table {{ width: 100%; border-collapse: collapse; }}
  th, td {{
    padding: 0.5rem 1.25rem;
    text-align: left;
    border-bottom: 1px solid var(--border);
    font-size: 0.82rem;
  }}
  th {{
    font-size: 0.65rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-3);
    background: var(--surface-2);
  }}
  tr:last-child td {{ border-bottom: none; }}
  tbody tr:hover td {{ background: rgba(99,102,241,0.04); }}
  td.mono {{ font-family: var(--font-mono); font-size: 0.78rem; }}
  td.success {{ color: var(--success); font-family: var(--font-mono); }}
  td.warn    {{ color: var(--warn);    font-family: var(--font-mono); }}
  td.danger  {{ color: var(--danger);  font-family: var(--font-mono); }}
  td.info    {{ color: var(--info);    font-family: var(--font-mono); }}

  /* ── Findings ────────────────────────────────────────── */
  .finding {{
    padding: 0.875rem 1.25rem;
    border-bottom: 1px solid var(--border);
    display: grid;
    gap: 0.3rem;
  }}
  .finding:last-child {{ border-bottom: none; }}
  .finding-top {{
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.5rem;
  }}
  .finding-kind {{
    display: inline-block;
    padding: 0.1rem 0.45rem;
    border-radius: 3px;
    font-family: var(--font-mono);
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    background: var(--danger-dim);
    color: var(--danger);
    border: 1px solid rgba(248,113,113,0.18);
    flex-shrink: 0;
  }}
  .finding-desc {{
    font-size: 0.87rem;
    color: var(--text);
    flex: 1;
  }}
  .waste-pill {{
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.1rem 0.5rem;
    border-radius: 20px;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    font-weight: 700;
    background: var(--warn-dim);
    color: var(--warn);
    border: 1px solid rgba(245,158,11,0.2);
    white-space: nowrap;
  }}
  .finding-meta {{
    font-size: 0.72rem;
    color: var(--text-3);
  }}
  .finding-evidence {{
    font-family: var(--font-mono);
    font-size: 0.72rem;
    color: var(--text-3);
    padding-left: 0.5rem;
  }}
  .finding-evidence::before {{ content: '· '; color: var(--border-2); }}
  .no-findings {{
    padding: 1.25rem;
    color: var(--success);
    font-size: 0.875rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }}
  .no-findings::before {{ content: '✓'; font-weight: 700; }}

  /* ── Footer ──────────────────────────────────────────── */
  footer {{
    text-align: center;
    padding: 2rem;
    color: var(--text-3);
    font-size: 0.72rem;
    font-family: var(--font-mono);
  }}
</style>
</head>
<body>
<div class="header">
  <span class="header-logo">tracekit</span>
  <span class="header-sep">/</span>
  <span class="badge">{agent}</span>
  <span class="session-id">{session_id}</span>
</div>
<div class="container">

  <div class="kpi-grid">
    <div class="kpi">
      <div class="kpi-label">Total Cost</div>
      <div class="kpi-value success">{total_cost}</div>
    </div>
    <div class="kpi kpi-waste">
      <div class="kpi-label">Identified Waste</div>
      <div class="kpi-value {waste_class}">{waste_display}</div>
    </div>
    <div class="kpi">
      <div class="kpi-label">Messages</div>
      <div class="kpi-value info">{message_count}</div>
    </div>
    <div class="kpi">
      <div class="kpi-label">Input Tokens</div>
      <div class="kpi-value">{input_tokens}</div>
    </div>
    <div class="kpi">
      <div class="kpi-label">Output Tokens</div>
      <div class="kpi-value">{output_tokens}</div>
    </div>
    <div class="kpi">
      <div class="kpi-label">Duration</div>
      <div class="kpi-value warn">{duration}</div>
    </div>
    <div class="kpi">
      <div class="kpi-label">Findings</div>
      <div class="kpi-value {findings_color}">{findings_count}</div>
    </div>
  </div>

  <div class="section">
    <div class="section-header">Session</div>
    <dl class="meta-grid">
      <dt>Agent</dt><dd>{agent}</dd>
      <dt>Model</dt><dd>{model}</dd>
      <dt>CWD</dt><dd>{cwd}</dd>
      <dt>Started</dt><dd>{started_at}</dd>
      <dt>Source</dt><dd>{source_path}</dd>
    </dl>
  </div>

  <div class="section">
    <div class="section-header">Top Expensive Turns</div>
    {expensive_html}
  </div>

  <div class="section">
    <div class="section-header">Inefficiency Findings</div>
    {findings_html}
  </div>

</div>
<footer>tracekit · {timestamp}</footer>
</body>
</html>"#,
        session_id = &s.session_id,
        agent = s.source_agent,
        total_cost = fmt_cost_html(s.total_cost_usd),
        waste_display = waste_display,
        waste_class = waste_class,
        message_count = s.message_count,
        input_tokens = fmt_tokens(s.total_input_tokens),
        output_tokens = fmt_tokens(s.total_output_tokens),
        duration = fmt_duration(s.duration_secs()),
        findings_count = result.findings.len(),
        findings_color = if result.findings.is_empty() {
            "success"
        } else {
            "danger"
        },
        model = html_escape(s.model.as_deref().unwrap_or("-")),
        cwd = html_escape(s.cwd.as_deref().unwrap_or("-")),
        started_at = fmt_ts(s.started_at),
        source_path = html_escape(&s.source_path.display().to_string()),
        findings_html = findings_html,
        expensive_html = expensive_html,
        timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    ))
}

pub fn render_aggregate(results: &[AnalysisResult]) -> Result<String> {
    let total_cost: f64 = results
        .iter()
        .filter_map(|r| r.session.total_cost_usd)
        .sum();
    let total_msgs: usize = results.iter().map(|r| r.session.message_count).sum();
    let total_findings: usize = results.iter().map(|r| r.findings.len()).sum();
    let total_waste: f64 = results
        .iter()
        .flat_map(|r| r.findings.iter())
        .filter_map(|f| f.wasted_cost_usd)
        .sum();

    let sessions_html = results
        .iter()
        .map(|r| {
            let s = &r.session;
            let session_waste: f64 = r.findings.iter().filter_map(|f| f.wasted_cost_usd).sum();
            format!(
                r#"<tr>
              <td>{}</td>
              <td class="mono">{}</td>
              <td class="success">{}</td>
              <td class="danger">{}</td>
              <td>{}</td>
              <td>{}</td>
              <td>{}</td>
            </tr>"#,
                s.source_agent,
                truncate(&s.session_id, 36),
                fmt_cost_html(s.total_cost_usd),
                if session_waste > 0.0 {
                    format!("~${:.2}", session_waste)
                } else {
                    "—".to_string()
                },
                html_escape(s.cwd.as_deref().unwrap_or("-")),
                fmt_ts(s.started_at),
                s.message_count,
            )
        })
        .collect::<String>();

    Ok(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>tracekit — Aggregate Report</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=JetBrains+Mono:wght@400;500;700&display=swap" rel="stylesheet">
<style>
  :root {{
    --bg:#07080e; --surface:#0d0f1a; --surface-2:#121520;
    --border:#1c2035; --border-2:#252942;
    --text:#dde3f0; --text-2:#8892aa; --text-3:#4a5270;
    --accent:#6366f1; --success:#34d399; --warn:#f59e0b;
    --danger:#f87171; --info:#38bdf8;
    --font-ui:'Inter',system-ui,sans-serif;
    --font-mono:'JetBrains Mono','Fira Code',monospace;
    --radius:6px; --radius-lg:10px;
  }}
  *{{box-sizing:border-box;margin:0;padding:0}}
  html{{font-size:14px}}
  body{{background:var(--bg);color:var(--text);font-family:var(--font-ui);min-height:100vh;line-height:1.5}}
  .header{{background:var(--surface);border-bottom:1px solid var(--border);padding:1rem 2rem;display:flex;align-items:center;gap:.875rem}}
  .header-logo{{font-family:var(--font-mono);font-size:1rem;font-weight:700;color:var(--accent)}}
  .container{{max-width:1200px;margin:0 auto;padding:1.75rem 2rem}}
  .kpi-grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:.75rem;margin-bottom:1.5rem}}
  .kpi{{background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);padding:1.125rem 1.25rem}}
  .kpi-label{{font-size:.65rem;font-weight:600;text-transform:uppercase;letter-spacing:.1em;color:var(--text-3);margin-bottom:.5rem}}
  .kpi-value{{font-family:var(--font-mono);font-size:1.5rem;font-weight:700;line-height:1}}
  .kpi.kpi-waste{{border-color:rgba(248,113,113,.25);background:linear-gradient(135deg,rgba(248,113,113,.06) 0%,var(--surface) 60%)}}
  .section{{background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);margin-bottom:1rem;overflow:hidden}}
  .section-header{{padding:.75rem 1.25rem;border-bottom:1px solid var(--border);font-size:.65rem;font-weight:600;text-transform:uppercase;letter-spacing:.1em;color:var(--text-3);background:var(--surface-2)}}
  table{{width:100%;border-collapse:collapse}}
  th,td{{padding:.5rem 1.25rem;text-align:left;border-bottom:1px solid var(--border);font-size:.82rem}}
  th{{font-size:.65rem;font-weight:600;text-transform:uppercase;letter-spacing:.08em;color:var(--text-3);background:var(--surface-2)}}
  tr:last-child td{{border-bottom:none}}
  tbody tr:hover td{{background:rgba(99,102,241,.04)}}
  td.mono{{font-family:var(--font-mono);font-size:.78rem}}
  td.success{{color:var(--success);font-family:var(--font-mono)}}
  td.danger{{color:var(--danger);font-family:var(--font-mono)}}
  footer{{text-align:center;padding:2rem;color:var(--text-3);font-size:.72rem;font-family:var(--font-mono)}}
</style>
</head>
<body>
<div class="header"><span class="header-logo">tracekit</span><span style="color:var(--border-2)">/</span><span style="color:var(--text-3);font-size:.8rem">aggregate report</span></div>
<div class="container">
  <div class="kpi-grid">
    <div class="kpi"><div class="kpi-label">Total Cost</div><div class="kpi-value" style="color:var(--success)">${total_cost:.4}</div></div>
    <div class="kpi kpi-waste"><div class="kpi-label">Identified Waste</div><div class="kpi-value" style="color:var(--danger)">~${total_waste:.2}</div></div>
    <div class="kpi"><div class="kpi-label">Sessions</div><div class="kpi-value" style="color:var(--info)">{session_count}</div></div>
    <div class="kpi"><div class="kpi-label">Messages</div><div class="kpi-value">{total_msgs}</div></div>
    <div class="kpi"><div class="kpi-label">Findings</div><div class="kpi-value" style="color:var(--warn)">{total_findings}</div></div>
  </div>
  <div class="section">
    <div class="section-header">Sessions</div>
    <table>
      <thead><tr>
        <th>Agent</th><th>Session ID</th><th>Cost</th><th>Waste</th>
        <th>CWD</th><th>Started</th><th>Messages</th>
      </tr></thead>
      <tbody>{sessions_html}</tbody>
    </table>
  </div>
</div>
<footer>tracekit · {timestamp}</footer>
</body>
</html>"#,
        total_cost = total_cost,
        total_waste = total_waste,
        session_count = results.len(),
        total_msgs = total_msgs,
        total_findings = total_findings,
        sessions_html = sessions_html,
        timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    ))
}

fn render_findings(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return r#"<div class="no-findings">No inefficiencies detected</div>"#.to_string();
    }

    findings
        .iter()
        .map(|f| {
            let evidence_html = f
                .evidence
                .iter()
                .take(5)
                .map(|e| format!(r#"<div class="finding-evidence">{}</div>"#, html_escape(e)))
                .collect::<String>();

            let waste_html = f
                .wasted_cost_usd
                .filter(|&c| c > 0.0)
                .map(|c| {
                    format!(
                        r#"<span class="waste-pill">~{} wasted</span>"#,
                        fmt_cost_html(Some(c))
                    )
                })
                .unwrap_or_default();

            format!(
                r#"<div class="finding">
              <div class="finding-top">
                <span class="finding-kind">{kind}</span>
                <span class="finding-desc">{desc}</span>
                {waste}
              </div>
              <div class="finding-meta">confidence {conf:.0}%</div>
              {evidence}
            </div>"#,
                kind = f.kind,
                desc = html_escape(&f.description),
                waste = waste_html,
                conf = f.confidence * 100.0,
                evidence = evidence_html,
            )
        })
        .collect()
}

fn render_expensive_messages(messages: &[ExpensiveMessage]) -> String {
    if messages.is_empty() {
        return r#"<div style="padding:1.25rem;color:var(--text-3);font-size:.85rem">No cost data available.</div>"#.to_string();
    }

    let rows = messages
        .iter()
        .map(|m| {
            format!(
                r#"<tr>
              <td class="mono">{}</td>
              <td class="success">{}</td>
              <td class="mono">{}</td>
              <td class="mono">{}</td>
              <td class="mono">{}</td>
            </tr>"#,
                m.sequence,
                fmt_cost_html(Some(m.cost_usd)),
                fmt_tokens(m.input_tokens),
                fmt_tokens(m.output_tokens),
                m.tool_count,
            )
        })
        .collect::<String>();

    format!(
        r#"<table>
          <thead><tr>
            <th>Turn</th><th>Cost</th><th>Billed Input</th><th>Output</th><th>Tools</th>
          </tr></thead>
          <tbody>{}</tbody>
        </table>"#,
        rows
    )
}

fn fmt_cost_html(cost: Option<f64>) -> String {
    match cost {
        Some(c) => format!("${:.4}", c),
        None => "—".to_string(),
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
        None => "—".to_string(),
        Some(s) if s < 60 => format!("{}s", s),
        Some(s) if s < 3600 => format!("{}m{}s", s / 60, s % 60),
        Some(s) => format!("{}h{}m", s / 3600, (s % 3600) / 60),
    }
}

fn fmt_ts(ts: Option<chrono::DateTime<chrono::Utc>>) -> String {
    match ts {
        Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
        None => "—".to_string(),
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
