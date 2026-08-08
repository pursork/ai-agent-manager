mod cpa;
mod deepseek;

use crate::provider::Provider;
use crate::provider_registry::{ProviderKind, ProviderRecord};
pub use cpa::CpaProvider;
pub use deepseek::{catalog_file_name, DeepSeekProvider};

/// Builds the concrete `Provider` a `ProviderRecord` describes, given its
/// (separately-resolved, from `aam-vault`) plaintext API key.
pub fn build_provider(record: &ProviderRecord, api_key: String) -> Box<dyn Provider> {
    match record.kind {
        ProviderKind::Cpa => Box::new(CpaProvider {
            base_url: record.base_url.clone(),
            model: record.model.clone(),
            api_key,
            supports_websockets: record.supports_websockets,
            reasoning_effort: record.reasoning_effort.clone(),
            plan_reasoning_effort: record.plan_reasoning_effort.clone(),
        }),
        ProviderKind::DeepseekV4Flash => Box::new(DeepSeekProvider::new(
            record.base_url.clone(),
            api_key,
            record.reasoning_effort.clone(),
            record.plan_reasoning_effort.clone(),
        )),
    }
}
