use crate::data::Counter;
use crate::openapi;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PoolItem {
    /// Pool code
    pub code: String,
    /// Pool id
    pub counter_id: Counter,
    /// Quote asset symbol (used in popup rendering)
    pub market: String,
    /// Display name
    pub name: String,
}

impl PartialEq for PoolItem {
    fn eq(&self, other: &Self) -> bool {
        self.counter_id == other.counter_id
    }
}

#[derive(Default, Clone, Debug, Deserialize, Serialize)]
pub struct PoolResult {
    pub product_list: Vec<PoolItem>,
    pub recommend_list: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PoolQuery {
    #[serde(rename = "k")]
    pub keyword: String,
}
pub async fn fetch_pool(query: &PoolQuery) -> Result<PoolResult> {
    let pools = openapi::quote().list_pools().await?;
    let keyword = query.keyword.to_ascii_uppercase();

    let product_list = pools
        .into_iter()
        .filter(|pool| {
            pool.pool_name.to_ascii_uppercase().contains(&keyword)
                || pool
                    .base_asset_symbol
                    .to_ascii_uppercase()
                    .contains(&keyword)
                || pool
                    .quote_asset_symbol
                    .to_ascii_uppercase()
                    .contains(&keyword)
        })
        .map(|pool| PoolItem {
            code: pool.pool_name.clone(),
            counter_id: Counter::new(&pool.pool_name),
            market: pool.quote_asset_symbol.clone(),
            name: pool.display_name(),
        })
        .collect();

    Ok(PoolResult {
        product_list,
        recommend_list: None,
    })
}
