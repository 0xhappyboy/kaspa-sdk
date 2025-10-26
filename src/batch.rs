/// Batch processing module.
use crate::KaspaClient;
use crate::types::{Block, Result, Transaction, UtxoEntry};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct BatchProcessor {
    client: KaspaClient,
    block_cache: HashMap<String, Block>,
    transaction_cache: HashMap<String, Transaction>,
    utxo_cache: HashMap<String, Vec<UtxoEntry>>,
    cache_ttl: Duration,
    last_cleanup: Instant,
}

impl BatchProcessor {
    pub fn new(client: KaspaClient) -> Self {
        Self {
            client,
            block_cache: HashMap::new(),
            transaction_cache: HashMap::new(),
            utxo_cache: HashMap::new(),
            cache_ttl: Duration::from_secs(300),
            last_cleanup: Instant::now(),
        }
    }

    /// Get blocks in batches.
    pub async fn get_blocks_batch(
        &mut self,
        hashes: &[String],
        include_transactions: bool,
    ) -> Result<Vec<Block>> {
        self.clean_cache();
        let mut result = Vec::new();
        let mut missing_hashes = Vec::new();
        for hash in hashes {
            if let Some(block) = self.block_cache.get(hash) {
                result.push(block.clone());
            } else {
                missing_hashes.push(hash.clone());
            }
        }
        if !missing_hashes.is_empty() {
            for hash in missing_hashes {
                match self.client.get_block(&hash, include_transactions).await {
                    Ok(block) => {
                        let block_hash = if let Some(verbose_data) = &block.verbose_data {
                            verbose_data.hash.clone()
                        } else {
                            continue;
                        };
                        self.block_cache.insert(block_hash.clone(), block.clone());
                        result.push(block);
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(result)
    }

    /// Get balance in batches
    pub async fn get_balances_batch(
        &mut self,
        addresses: &[String],
    ) -> Result<HashMap<String, u64>> {
        let mut balances = HashMap::new();
        for chunk in addresses.chunks(50) {
            let chunk_balances = self
                .client
                .get_balances_by_addresses(chunk.iter().map(|s| s.as_str()).collect())
                .await?;
            for balance in chunk_balances {
                balances.insert(balance.address, balance.balance);
            }
        }
        Ok(balances)
    }

    /// clean cache
    fn clean_cache(&mut self) {
        if self.last_cleanup.elapsed() > self.cache_ttl {
            self.block_cache.clear();
            self.transaction_cache.clear();
            self.utxo_cache.clear();
            self.last_cleanup = Instant::now();
        }
    }
}
