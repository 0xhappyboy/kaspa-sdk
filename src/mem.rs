/// This module is used to monitor and analyze the behavior of the memory transaction pool.
use crate::KaspaClient;
use crate::types::{KaspaError, MempoolEntry, Result, Transaction};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

/// mem pool abstract
///
/// # Example
/// ```
/// let client = KaspaClient::new("");
/// let mut mem_pool = MemPool::new(client);
///
/// let mem_pool_analysis = mem_pool.analyze_mempool().await?;
/// ```
pub struct MemPool {
    client: KaspaClient,
    history: VecDeque<MemPoolSnapshot>,
    max_history_size: usize,
}

#[derive(Debug, Clone)]
pub struct MemPoolSnapshot {
    pub timestamp: Instant,
    pub total_transactions: usize,
    pub total_size: u64, // total byte size
    pub total_fee: u64,
    pub entries: Vec<MempoolEntry>,
}

#[derive(Debug, Clone)]
pub struct MempoolAnalysis {
    pub total_transactions: usize,
    pub total_fee: u64,
    pub average_fee: f64,
    pub median_fee: f64,
    pub fee_distribution: HashMap<String, usize>,
    pub top_addresses: Vec<(String, u64)>,
    pub orphan_count: usize,
    pub congestion_level: CongestionLevel,
    pub estimated_confirmation_times: HashMap<String, Duration>, // Estimated Confirmation Time
    pub rbf_signals: Vec<ReplaceByFeeSignal>,                    // RBF replacement signal
    pub cpfp_candidates: Vec<CpfpCandidate>,                     // CPFP Candidate Transactions
}

/// Trading pool congestion
#[derive(Debug, Clone, PartialEq)]
pub enum CongestionLevel {
    Idle,   // idle
    Normal, // normal
    High,   // congestion
    SHigh,  // extreme congestion
}

#[derive(Debug, Clone)]
pub struct ReplaceByFeeSignal {
    pub original_tx_id: String,
    pub replacement_tx_id: String,
    pub fee_increase: u64,
    pub fee_increase_percent: f64,
}

#[derive(Debug, Clone)]
pub struct CpfpCandidate {
    pub parent_tx_id: String,
    pub child_tx_id: String,
    pub combined_fee: u64,
    pub fee_increase: u64,
}

#[derive(Debug, Clone)]
pub struct TransactionAnomaly {
    pub transaction_id: String,
    pub anomaly_type: AnomalyBehaviorType,
    pub confidence: f64, // 0.0 - 1.0
    pub description: String,
}

/// Abnormal behavior type
#[derive(Debug, Clone)]
pub enum AnomalyBehaviorType {
    HighValueSuddenMovement, // High-value sudden movement
    PossibleWalletSweep,     // Possible wallet cleanup
    MixingPattern,           // Coin mixing
    DustSpam,                // Dust attack
    FeeSpike,                // Abnormally high rates
    TimeLockExploitation,    // Time lock utilization
}

const MAX_SNAPSHOT_SIZE: usize = 100;

impl MemPool {
    /// create a new mempool
    ///
    /// # Example
    /// ```
    /// let client = KaspaClient::new("");
    /// let analyzer = MemPool::new(client);
    /// ```
    pub fn new(client: KaspaClient) -> Self {
        Self {
            client,
            history: VecDeque::new(),
            max_history_size: MAX_SNAPSHOT_SIZE,
        }
    }

    /// get mem pool
    ///
    /// # Example
    /// ```
    /// let analysis = mempool.get_mempool().await?;
    /// println!("Total transactions: {}", analysis.total_transactions);
    /// println!("Congestion level: {:?}", analysis.congestion_level);
    /// ```
    pub async fn get_mempool(&mut self) -> Result<MempoolAnalysis> {
        let entries = self.client.get_mempool_entries(Some(true)).await?;
        // save snapshot
        let snapshot = MemPoolSnapshot {
            timestamp: Instant::now(),
            total_transactions: entries.len(),
            total_size: self.cal_total_size(&entries),
            total_fee: entries.iter().map(|e| e.fee).sum(),
            entries: entries.clone(),
        };
        self.add_snapshot(snapshot);
        let mut analysis = MempoolAnalysis {
            total_transactions: entries.len(),
            total_fee: 0,
            average_fee: 0.0,
            median_fee: 0.0,
            fee_distribution: HashMap::new(),
            top_addresses: Vec::new(),
            orphan_count: 0,
            congestion_level: CongestionLevel::Normal,
            estimated_confirmation_times: HashMap::new(),
            rbf_signals: Vec::new(),
            cpfp_candidates: Vec::new(),
        };
        let mut address_volumes: HashMap<String, u64> = HashMap::new();
        let mut fee_ranges: HashMap<String, usize> = HashMap::new();
        let mut all_fees: Vec<u64> = Vec::new();
        for entry in &entries {
            analysis.total_fee += entry.fee;
            all_fees.push(entry.fee);
            if entry.is_orphan {
                analysis.orphan_count += 1;
            }
            self.analyze_transaction_io(&entry.transaction, &mut address_volumes);
            let fee_range = self.categorize_fee(entry.fee);
            *fee_ranges.entry(fee_range).or_insert(0) += 1;
        }
        // Calculate statistical indicators
        analysis.average_fee = if !entries.is_empty() {
            analysis.total_fee as f64 / entries.len() as f64
        } else {
            0.0
        };
        analysis.median_fee = self.cal_median(&all_fees);
        analysis.fee_distribution = fee_ranges;
        analysis.top_addresses = self.get_top_addresses(address_volumes, 10);
        analysis.congestion_level = self.assess_congestion_level(&entries);
        analysis.estimated_confirmation_times = self.estimate_confirmation_times(&entries);
        analysis.rbf_signals = self.detect_rbf_signals().await?;
        analysis.cpfp_candidates = self.detect_cpfp_candidates(&entries).await?;
        Ok(analysis)
    }

    /// detect RBF signals
    async fn detect_rbf_signals(&self) -> Result<Vec<ReplaceByFeeSignal>> {
        let mut signals = Vec::new();
        if self.history.len() >= 2 {
            let current = self.history.back().unwrap();
            let previous = self.history.get(self.history.len() - 2).unwrap();
            for current_entry in &current.entries {
                for previous_entry in &previous.entries {
                    if self
                        .have_common_inputs(&current_entry.transaction, &previous_entry.transaction)
                    {
                        if current_entry.fee > previous_entry.fee {
                            let fee_increase = current_entry.fee - previous_entry.fee;
                            let fee_increase_percent =
                                (fee_increase as f64 / previous_entry.fee as f64) * 100.0;
                            signals.push(ReplaceByFeeSignal {
                                original_tx_id: todo!(),
                                replacement_tx_id: todo!(),
                                fee_increase,
                                fee_increase_percent,
                            });
                        }
                    }
                }
            }
        }
        Ok(signals)
    }

    /// detect cpfp candidates
    async fn detect_cpfp_candidates(&self, entries: &[MempoolEntry]) -> Result<Vec<CpfpCandidate>> {
        let mut candidates = Vec::new();
        let dependency_graph = self.build_trade_depend_graph(entries).await?;
        for (parent_id, children) in dependency_graph {
            if let Some(parent_entry) = entries
                .iter()
                .find(|e| self.get_transaction_id(e) == parent_id)
            {
                for child_id in children {
                    if let Some(child_entry) = entries
                        .iter()
                        .find(|e| self.get_transaction_id(e) == child_id)
                    {
                        let combined_fee = parent_entry.fee + child_entry.fee;
                        let fee_increase = child_entry.fee;

                        if fee_increase > 1000 {
                            candidates.push(CpfpCandidate {
                                parent_tx_id: parent_id.clone(),
                                child_tx_id: child_id.clone(),
                                combined_fee,
                                fee_increase,
                            });
                        }
                    }
                }
            }
        }
        Ok(candidates)
    }

    /// build trade depend graph
    async fn build_trade_depend_graph(
        &self,
        entries: &[MempoolEntry],
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        for entry in entries {
            let tx_id = self.get_transaction_id(entry);
            for input in &entry.transaction.inputs {
                let parent_tx_id = input.previous_outpoint.transaction_id.clone();
                if entries
                    .iter()
                    .any(|e| self.get_transaction_id(e) == parent_tx_id)
                {
                    graph
                        .entry(parent_tx_id)
                        .or_insert_with(Vec::new)
                        .push(tx_id.clone());
                }
            }
        }
        Ok(graph)
    }

    /// Detects transaction anomalies
    ///
    /// # Example
    /// ```
    /// let anomalies = mempool.detect_transaction_anomalies().await?;
    /// for anomaly in anomalies {
    ///     println!("Anomaly: {} - {}", anomaly.transaction_id, anomaly.description);
    /// }
    /// ```
    pub async fn detect_transaction_anomalies(&self) -> Result<Vec<TransactionAnomaly>> {
        let entries = self.client.get_mempool_entries(Some(true)).await?;
        let mut anomalies = Vec::new();
        for entry in entries {
            let tx_anomalies = self.analyze_transaction_anomalies(&entry).await;
            anomalies.extend(tx_anomalies);
        }
        Ok(anomalies)
    }

    /// analyze transaction anomalies
    async fn analyze_transaction_anomalies(&self, entry: &MempoolEntry) -> Vec<TransactionAnomaly> {
        let mut anomalies = Vec::new();
        let transaction = &entry.transaction;
        if self.is_high_value_sudden_movement(transaction) {
            anomalies.push(TransactionAnomaly {
                transaction_id: self.get_transaction_id(entry),
                anomaly_type: AnomalyBehaviorType::HighValueSuddenMovement,
                confidence: 0.7,
                description: "Detecting sudden movement of high-value assets".to_string(),
            });
        }
        if self.is_possible_wallet_sweep(transaction) {
            anomalies.push(TransactionAnomaly {
                transaction_id: self.get_transaction_id(entry),
                anomaly_type: AnomalyBehaviorType::PossibleWalletSweep,
                confidence: 0.8,
                description: "Possible wallet cleanup detected".to_string(),
            });
        }
        if self.is_mixing_pattern(transaction) {
            anomalies.push(TransactionAnomaly {
                transaction_id: self.get_transaction_id(entry),
                anomaly_type: AnomalyBehaviorType::MixingPattern,
                confidence: 0.6,
                description: "Possible coin mixing pattern detected".to_string(),
            });
        }
        if self.is_dust_spam(transaction) {
            anomalies.push(TransactionAnomaly {
                transaction_id: self.get_transaction_id(entry),
                anomaly_type: AnomalyBehaviorType::DustSpam,
                confidence: 0.9,
                description: "Dust attack detected".to_string(),
            });
        }
        if self.is_fee_spike(entry) {
            anomalies.push(TransactionAnomaly {
                transaction_id: self.get_transaction_id(entry),
                anomaly_type: AnomalyBehaviorType::FeeSpike,
                confidence: 0.85,
                description: "Abnormally high rates detected".to_string(),
            });
        }
        anomalies
    }

    /// predict mempool evolution
    ///
    /// # Example
    /// ```
    /// let prediction = mempool.predict_mempool_evolution(Duration::from_secs(3600)).await?;
    /// println!("Predicted transactions in 1 hour: {}", prediction.predicted_transaction_count);
    /// println!("Recommended fee: {}", prediction.recommended_fee);
    /// ```
    pub async fn predict_mempool_evolution(
        &self,
        time_horizon: Duration,
    ) -> Result<MempoolPrediction> {
        if self.history.len() < 5 {
            return Err(KaspaError::Custom("Not enough data".to_string()));
        }
        let current = self.history.back().unwrap();
        let growth_rate = self.cal_growth_rate();
        let predicted_tx_count = (current.total_transactions as f64 * (1.0 + growth_rate)) as usize;
        let predicted_fee = (current.total_fee as f64 * (1.0 + growth_rate)) as u64;
        Ok(MempoolPrediction {
            timestamp: Instant::now() + time_horizon,
            predicted_transaction_count: predicted_tx_count,
            predicted_total_fee: predicted_fee,
            confidence_interval: (0.7, 0.9), // confidence interval
            recommended_fee: self.cal_recommended_fee(current),
        })
    }

    // Calculate recommended fee
    fn cal_recommended_fee(&self, snapshot: &MemPoolSnapshot) -> u64 {
        let fees: Vec<u64> = snapshot.entries.iter().map(|e| e.fee).collect();
        if fees.is_empty() {
            return 1000; // default fee
        }
        let mut sorted_fees = fees.clone();
        sorted_fees.sort();
        let index = (sorted_fees.len() as f64 * 0.75) as usize;
        sorted_fees[index.min(sorted_fees.len() - 1)]
    }

    /// mock mempool stress
    ///
    /// # Example
    /// ```
    /// let stress_test = mempool.mock_mempool_stress(100.0, Duration::from_secs(300)).await?;
    /// println!("Final size: {}", stress_test.final_size);
    /// println!("Dropped transactions: {}", stress_test.total_dropped);
    /// ```
    pub async fn mock_mempool_stress(
        &self,
        incoming_tx_rate: f64,
        duration: Duration,
    ) -> Result<StressTestResult> {
        let mut mock_mempool = self.history.back().unwrap().clone();
        let mut total_dropped = 0;
        let mut total_added = 0;
        let interval = Duration::from_secs(1);
        let iterations = duration.as_secs();
        for _ in 0..iterations {
            // Simulate new transaction arrival
            let new_txs = (incoming_tx_rate * interval.as_secs_f64()) as usize;
            total_added += new_txs;
            // Simulate block confirmation (remove transactions)
            let confirmed_txs = self.mock_block_confirmation(&mock_mempool);
            mock_mempool
                .entries
                .retain(|e| !confirmed_txs.contains(&self.get_transaction_id(e)));
            // Simulating transaction drops due to memory limitations
            let mempool_limit = 50_000;
            if mock_mempool.entries.len() + new_txs > mempool_limit {
                let to_drop = (mock_mempool.entries.len() + new_txs) - mempool_limit;
                total_dropped += to_drop;
                mock_mempool.entries.sort_by(|a, b| b.fee.cmp(&a.fee));
                mock_mempool.entries.truncate(mempool_limit - new_txs);
            }
            for i in 0..new_txs {
                let simulated_entry = MempoolEntry {
                    fee: 1000 + (i % 1000) as u64, // mock fee 1000-1999
                    transaction: Transaction {
                        version: 0,
                        inputs: vec![],
                        outputs: vec![],
                        lock_time: 0,
                        subnetwork_id: "".to_string(),
                        gas: 0,
                        payload: "".to_string(),
                        verbose_data: None,
                    },
                    is_orphan: false,
                };
                mock_mempool.entries.push(simulated_entry);
            }
        }
        Ok(StressTestResult {
            initial_size: self.history.back().unwrap().total_transactions,
            final_size: mock_mempool.entries.len(),
            total_added,
            total_dropped,
            average_confirmation_time: Duration::from_secs(10),
            congestion_level: self.assess_congestion_level(&mock_mempool.entries),
        })
    }

    // calculate total size
    fn cal_total_size(&self, entries: &[MempoolEntry]) -> u64 {
        entries
            .iter()
            .map(|e| e.transaction.outputs.len() * 100)
            .sum::<usize>() as u64
    }

    fn add_snapshot(&mut self, snapshot: MemPoolSnapshot) {
        self.history.push_back(snapshot);
        if self.history.len() > self.max_history_size {
            self.history.pop_front();
        }
    }

    fn cal_median(&self, values: &[u64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        let mut sorted = values.to_vec();
        sorted.sort();
        let mid = sorted.len() / 2;
        if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) as f64 / 2.0
        } else {
            sorted[mid] as f64
        }
    }

    fn assess_congestion_level(&self, entries: &[MempoolEntry]) -> CongestionLevel {
        let tx_count = entries.len();
        match tx_count {
            0..=1000 => CongestionLevel::Idle,
            1001..=5000 => CongestionLevel::Normal,
            5001..=20000 => CongestionLevel::High,
            _ => CongestionLevel::SHigh,
        }
    }

    fn estimate_confirmation_times(&self, entries: &[MempoolEntry]) -> HashMap<String, Duration> {
        let mut estimates = HashMap::new();
        if entries.is_empty() {
            return estimates;
        }
        let mut fees: Vec<u64> = entries.iter().map(|e| e.fee).collect();
        fees.sort();
        let low_fee = fees[fees.len() / 4]; // 25%
        let medium_fee = fees[fees.len() / 2]; // 50%
        let high_fee = fees[(fees.len() * 3) / 4]; // 75%
        estimates.insert("low".to_string(), Duration::from_secs(60 * 60));
        estimates.insert("medium".to_string(), Duration::from_secs(10 * 60));
        estimates.insert("high".to_string(), Duration::from_secs(2 * 60));
        estimates.insert("very_high".to_string(), Duration::from_secs(30));
        estimates
    }

    /// calculate growth rate
    fn cal_growth_rate(&self) -> f64 {
        if self.history.len() < 2 {
            return 0.0;
        }
        let current = self.history.back().unwrap();
        let previous = self.history.front().unwrap();
        let time_diff = current
            .timestamp
            .duration_since(previous.timestamp)
            .as_secs_f64();
        let tx_diff = current.total_transactions as i64 - previous.total_transactions as i64;
        (tx_diff as f64 / previous.total_transactions as f64) / time_diff
    }

    /// mock  block confirmation
    fn mock_block_confirmation(&self, snapshot: &MemPoolSnapshot) -> HashSet<String> {
        // mock block confirmation: select high-fee transaction
        let mut entries = snapshot.entries.clone();
        entries.sort_by(|a, b| b.fee.cmp(&a.fee));
        let to_confirm = entries
            .iter()
            .take(1000) // Assume that 1000 transactions are confirmed per block
            .map(|e| self.get_transaction_id(e))
            .collect();
        to_confirm
    }

    fn is_high_value_sudden_movement(&self, transaction: &Transaction) -> bool {
        let total_value: u64 = transaction.outputs.iter().map(|o| o.value).sum();
        total_value > 1_000_000_000 // 1000 KAS
    }

    fn is_possible_wallet_sweep(&self, transaction: &Transaction) -> bool {
        transaction.inputs.len() > 10 && transaction.outputs.len() <= 2
    }

    fn is_mixing_pattern(&self, transaction: &Transaction) -> bool {
        if transaction.outputs.len() < 5 {
            return false;
        }
        let values: Vec<u64> = transaction.outputs.iter().map(|o| o.value).collect();
        let first_value = values[0];
        values.iter().all(|&v| v == first_value) && first_value > 0
    }

    fn is_dust_spam(&self, transaction: &Transaction) -> bool {
        let dust_outputs = transaction
            .outputs
            .iter()
            .filter(|o| o.value < 1000)
            .count();
        dust_outputs > 10 && dust_outputs == transaction.outputs.len()
    }

    fn is_fee_spike(&self, entry: &MempoolEntry) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let current_median = self.cal_median(
            &self
                .history
                .back()
                .unwrap()
                .entries
                .iter()
                .map(|e| e.fee)
                .collect::<Vec<_>>(),
        );
        entry.fee as f64 > current_median * 10.0
    }

    fn have_common_inputs(&self, tx1: &Transaction, tx2: &Transaction) -> bool {
        let inputs1: HashSet<String> = tx1
            .inputs
            .iter()
            .map(|i| i.previous_outpoint.transaction_id.clone())
            .collect();
        let inputs2: HashSet<String> = tx2
            .inputs
            .iter()
            .map(|i| i.previous_outpoint.transaction_id.clone())
            .collect();
        !inputs1.is_disjoint(&inputs2)
    }

    fn get_transaction_id(&self, entry: &MempoolEntry) -> String {
        if let Some(verbose_data) = &entry.transaction.verbose_data {
            if !verbose_data.transaction_id.is_empty() {
                return verbose_data.transaction_id.clone();
            }
        }
        let tx = &entry.transaction;
        let tx_data = format!(
            "{}-{}-{}-{}-{}",
            tx.version,
            tx.inputs.len(),
            tx.outputs.len(),
            tx.lock_time,
            tx.gas
        );
        format!("tx_{:x}", seahash::hash(tx_data.as_bytes()))
    }

    fn analyze_transaction_io(
        &self,
        transaction: &Transaction,
        address_volumes: &mut HashMap<String, u64>,
    ) {
        for output in &transaction.outputs {
            if let Some(verbose_data) = &output.verbose_data {
                let address = &verbose_data.script_public_key_address;
                let volume = address_volumes.entry(address.clone()).or_insert(0);
                *volume += output.value;
            }
        }
    }

    fn categorize_fee(&self, fee: u64) -> String {
        match fee {
            0..=100 => "0-100".to_string(),
            101..=1000 => "101-1,000".to_string(),
            1001..=10000 => "1,001-10,000".to_string(),
            10001..=100000 => "10,001-100,000".to_string(),
            _ => "100,000+".to_string(),
        }
    }

    fn get_top_addresses(&self, volumes: HashMap<String, u64>, top_n: usize) -> Vec<(String, u64)> {
        let mut sorted: Vec<(String, u64)> = volumes.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.into_iter().take(top_n).collect()
    }
}

#[derive(Debug, Clone)]
pub struct MempoolPrediction {
    pub timestamp: Instant,
    pub predicted_transaction_count: usize,
    pub predicted_total_fee: u64,
    pub confidence_interval: (f64, f64),
    pub recommended_fee: u64,
}

#[derive(Debug, Clone)]
pub struct StressTestResult {
    pub initial_size: usize,
    pub final_size: usize,
    pub total_added: usize,
    pub total_dropped: usize,
    pub average_confirmation_time: Duration,
    pub congestion_level: CongestionLevel,
}
