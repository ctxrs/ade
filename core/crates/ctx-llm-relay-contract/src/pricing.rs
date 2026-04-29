use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricingCatalog {
    pub version: String,
    pub model_prices: Vec<ModelPrice>,
}

impl PricingCatalog {
    pub fn find_model_price(&self, provider_id: &str, model_id: &str) -> Option<&ModelPrice> {
        self.model_prices
            .iter()
            .find(|price| price.provider_id == provider_id && price.model_id == model_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelPrice {
    pub pricing_version: String,
    pub provider_id: String,
    pub model_id: String,
    pub currency: String,
    pub input_cost_micros_per_1k_tokens: u64,
    pub output_cost_micros_per_1k_tokens: u64,
    pub request_overhead_input_tokens: u32,
    pub function_schema_overhead_input_tokens: u32,
}
