use crate::KaspaClient;
use crate::types::{KaspaError, Result};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Network monitoring for Kaspa node
///
/// # Examples
/// ```
/// #[tokio::main]
/// async fn main() {
///     let client = KaspaClient::new("");
///     let mut monitor_arc = Arc::new(NetworkMonitor::new(client));
///     tokio::spawn(async move {
///         monitor_arc.clone().start_monitoring().await.unwrap();
///     });
///     loop {
///         let report = monitor.generate_health_report();
///         println!("Network healthy: {}", report.is_healthy);
///         tokio::time::sleep(Duration::from_secs(60)).await;
///     }
/// }
/// ```
pub struct NetworkMonitor {
    client: KaspaClient,
    metrics: NetworkMetrics,
    history: VecDeque<NetworkSnapshot>,
    max_history_size: usize,
}

#[derive(Debug, Clone)]
pub struct NetworkMetrics {
    pub block_rate: f64,       // block/s
    pub transaction_rate: f64, // trade/s
    pub peer_count: usize,
    pub network_hashrate: f64, // network hashrate(computing power)
    pub difficulty: f64,
}

pub struct NetworkSnapshot {
    pub timestamp: Instant,
    pub block_count: u64,
    pub mempool_size: usize,
    pub peer_count: usize,
}

impl NetworkMonitor {
    /// create new network monitor
    ///
    /// # Examples
    /// ```
    /// let client = KaspaClient::new("http://localhost:16110");
    /// let monitor = NetworkMonitor::new(client);
    /// ```
    pub fn new(client: KaspaClient) -> Self {
        Self {
            client,
            metrics: NetworkMetrics::default(),
            history: VecDeque::new(),
            max_history_size: 1000,
        }
    }

    /// start monitoring
    ///
    /// # Examples
    /// ```rust
    /// let mut monitor = NetworkMonitor::new(client);
    /// tokio::spawn(async move {
    ///     monitor.start_monitoring().await.unwrap();
    /// });
    /// ```
    pub async fn start_monitoring(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.update_metrics().await {
                println!("Monitoring error: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    async fn update_metrics(&mut self) -> Result<()> {
        let snapshot = self.create_snapshot().await?;
        self.history.push_back(snapshot);
        if self.history.len() > self.max_history_size {
            self.history.pop_front();
        }
        self.calculate_metrics().await?;
        Ok(())
    }

    /// create snapshot
    async fn create_snapshot(&self) -> Result<NetworkSnapshot> {
        let info = self.client.get_info().await?;
        let mempool_entries = self.client.get_mempool_entries(None).await?;
        Ok(NetworkSnapshot {
            timestamp: Instant::now(),
            block_count: info.block_count,
            mempool_size: mempool_entries.len(),
            peer_count: self.client.get_connected_peer_info().await.unwrap().len(),
        })
    }

    async fn calculate_metrics(&mut self) -> Result<()> {
        if self.history.len() < 2 {
            return Ok(());
        }
        let latest = self.history.back().unwrap();
        let previous = self.history.front().unwrap();
        let time_diff = latest
            .timestamp
            .duration_since(previous.timestamp)
            .as_secs_f64();
        let block_diff = latest.block_count - previous.block_count;
        self.metrics.block_rate = block_diff as f64 / time_diff.max(1.0);
        self.metrics.peer_count = latest.peer_count;
        self.metrics.network_hashrate = self.estimate_network_hashrate().await?;
        Ok(())
    }

    async fn estimate_network_hashrate(&self) -> Result<f64> {
        // Estimating network computing power based on difficulty and block time
        let hashrate_response = self
            .client
            .estimate_network_hashes_per_second(Some(100))
            .await?;
        hashrate_response
            .network_hashes_per_second
            .parse::<f64>()
            .map_err(|e| KaspaError::Custom(format!("Failed to parse hashrate: {}", e)))
    }

    /// Generate health health report
    ///
    /// # Examples
    /// ```
    /// let monitor = NetworkMonitor::new(client);
    /// let report = monitor.generate_health_report();
    /// if !report.is_healthy {
    ///     println!("ALERT: {}", report.recommendation);
    ///     // Send alert notification...
    /// }
    /// println!("Block rate: {:.2} B/s", report.metrics.block_rate);
    /// println!("Peers: {}", report.metrics.peer_count);
    /// ```
    pub fn generate_health_report(&self) -> NetworkHealthReport {
        NetworkHealthReport {
            metrics: self.metrics.clone(),
            is_healthy: self.metrics.block_rate > 0.1,
            recommendation: if self.metrics.peer_count < 5 {
                "Low peer count, consider adding more connections".to_string()
            } else {
                "Network healthy".to_string()
            },
        }
    }
}

impl Default for NetworkMetrics {
    fn default() -> Self {
        Self {
            block_rate: 0.0,
            transaction_rate: 0.0,
            peer_count: 0,
            network_hashrate: 0.0,
            difficulty: 0.0,
        }
    }
}

pub struct DetailedNetworkMetrics {
    pub inbound_peers: usize,
    pub outbound_peers: usize,
    pub average_ping: f64,
    pub user_agents: HashMap<String, usize>, // User agent statistics
    pub protocol_versions: HashMap<u32, usize>, // Protocol version statistics
}

pub struct NetworkHealthReport {
    pub metrics: NetworkMetrics,
    pub is_healthy: bool,
    pub recommendation: String,
}
pub struct DetailedNetworkReport {
    pub basic_metrics: NetworkMetrics,
    pub peer_metrics: DetailedNetworkMetrics,
    pub health_status: bool,
    pub recommendations: Vec<String>,
    pub timestamp: Instant,
}

impl DetailedNetworkReport {
    pub fn display(&self) {
        println!("=== Kaspa Network Report ===");
        println!(
            "Health Status: {}",
            if self.health_status {
                "✅ Healthy"
            } else {
                "❌ Unhealthy"
            }
        );
        println!(
            "Block Rate: {:.2} blocks/sec",
            self.basic_metrics.block_rate
        );
        println!(
            "Transaction Rate: {:.2} tx/sec",
            self.basic_metrics.transaction_rate
        );
        println!(
            "Network Hashrate: {:.2} H/s",
            self.basic_metrics.network_hashrate
        );
        println!(
            "Connected Peers: {} (In: {}, Out: {})",
            self.basic_metrics.peer_count,
            self.peer_metrics.inbound_peers,
            self.peer_metrics.outbound_peers
        );
        println!("Average Ping: {:.2} ms", self.peer_metrics.average_ping);

        if !self.recommendations.is_empty() {
            println!("Recommendations:");
            for rec in &self.recommendations {
                println!("  - {}", rec);
            }
        }
    }
}
