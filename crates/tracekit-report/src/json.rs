use anyhow::Result;
use tracekit_core::*;

pub fn render_analysis(result: &AnalysisResult) -> Result<String> {
    Ok(serde_json::to_string_pretty(result)?)
}

pub fn render_session_list(sessions: &[CanonicalSession]) -> Result<String> {
    Ok(serde_json::to_string_pretty(sessions)?)
}

pub fn render_aggregate(results: &[AnalysisResult]) -> Result<String> {
    let total_cost: f64 = results
        .iter()
        .filter_map(|r| r.session.total_cost_usd)
        .sum();

    let mut finding_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for r in results {
        for f in &r.findings {
            *finding_counts.entry(f.kind.to_string()).or_default() += 1;
        }
    }

    let summary = serde_json::json!({
        "sessions_analyzed": results.len(),
        "total_cost_usd": total_cost,
        "total_messages": results.iter().map(|r| r.session.message_count).sum::<usize>(),
        "finding_counts": finding_counts,
        "sessions": results,
    });

    Ok(serde_json::to_string_pretty(&summary)?)
}
