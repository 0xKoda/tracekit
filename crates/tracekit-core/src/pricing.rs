/// Model pricing catalog (USD per 1M tokens, as of early 2026).
/// Prices are (input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok).
/// cache_read/write may be None if not applicable.

#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

impl ModelPrice {
    const fn new(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Self {
        Self {
            input_per_mtok: input,
            output_per_mtok: output,
            cache_read_per_mtok: cache_read,
            cache_write_per_mtok: cache_write,
        }
    }

    pub fn estimate_cost(&self, input: u64, output: u64, cache_read: u64, cache_write: u64) -> f64 {
        let m = 1_000_000.0_f64;
        (input as f64 / m) * self.input_per_mtok
            + (output as f64 / m) * self.output_per_mtok
            + (cache_read as f64 / m) * self.cache_read_per_mtok
            + (cache_write as f64 / m) * self.cache_write_per_mtok
    }
}

/// Look up price by model ID string (case-insensitive prefix match).
pub fn lookup_price(model_id: &str) -> Option<ModelPrice> {
    let m = model_id.to_lowercase();
    // Claude models
    if m.contains("claude-opus-4") || m.contains("claude-4-opus") {
        return Some(ModelPrice::new(15.0, 75.0, 1.50, 3.75));
    }
    if m.contains("claude-sonnet-4") || m.contains("claude-4-sonnet") || m.contains("claude-4-5") || m.contains("claude-sonnet-4-5") {
        return Some(ModelPrice::new(3.0, 15.0, 0.30, 3.75));
    }
    if m.contains("claude-haiku-4") || m.contains("claude-4-haiku") || m.contains("haiku-4-5") {
        return Some(ModelPrice::new(0.80, 4.0, 0.08, 1.0));
    }
    if m.contains("claude-3-5-sonnet") || m.contains("claude-3.5-sonnet") {
        return Some(ModelPrice::new(3.0, 15.0, 0.30, 3.75));
    }
    if m.contains("claude-3-5-haiku") || m.contains("claude-3.5-haiku") {
        return Some(ModelPrice::new(0.80, 4.0, 0.08, 1.0));
    }
    if m.contains("claude-3-opus") {
        return Some(ModelPrice::new(15.0, 75.0, 1.50, 3.75));
    }
    if m.contains("claude-3-sonnet") {
        return Some(ModelPrice::new(3.0, 15.0, 0.30, 3.75));
    }
    if m.contains("claude-3-haiku") {
        return Some(ModelPrice::new(0.25, 1.25, 0.03, 0.31));
    }
    if m.contains("claude") {
        // Unknown Claude — use Sonnet pricing as safe default
        return Some(ModelPrice::new(3.0, 15.0, 0.30, 3.75));
    }
    // OpenAI models
    if m.contains("gpt-5") {
        return Some(ModelPrice::new(10.0, 40.0, 2.50, 10.0));
    }
    if m.contains("o3-mini") || m.contains("o4-mini") {
        return Some(ModelPrice::new(1.10, 4.40, 0.275, 1.10));
    }
    if m.contains("o3") || m.contains("o4") {
        return Some(ModelPrice::new(10.0, 40.0, 2.50, 10.0));
    }
    if m.contains("gpt-4o-mini") {
        return Some(ModelPrice::new(0.15, 0.60, 0.075, 0.15));
    }
    if m.contains("gpt-4o") {
        return Some(ModelPrice::new(2.50, 10.0, 1.25, 2.50));
    }
    if m.contains("gpt-4") {
        return Some(ModelPrice::new(30.0, 60.0, 7.50, 30.0));
    }
    if m.contains("gpt-3.5") {
        return Some(ModelPrice::new(0.50, 1.50, 0.50, 0.50));
    }
    // Moonshot / Kimi
    if m.contains("kimi") || m.contains("moonshot") {
        return Some(ModelPrice::new(0.15, 2.50, 0.04, 0.15));
    }
    // Google
    if m.contains("gemini-2.0-flash") {
        return Some(ModelPrice::new(0.10, 0.40, 0.025, 0.10));
    }
    if m.contains("gemini-2") {
        return Some(ModelPrice::new(1.25, 5.0, 0.31, 1.25));
    }
    if m.contains("gemini-1.5-pro") {
        return Some(ModelPrice::new(1.25, 5.0, 0.31, 1.25));
    }
    if m.contains("gemini-1.5-flash") {
        return Some(ModelPrice::new(0.075, 0.30, 0.02, 0.075));
    }
    None
}

pub fn estimate_cost(
    model_id: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> Option<f64> {
    let price = lookup_price(model_id)?;
    Some(price.estimate_cost(input_tokens, output_tokens, cache_read_tokens, cache_write_tokens))
}
