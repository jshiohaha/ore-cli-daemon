use aws_config::meta::region::RegionProviderChain;
use aws_sdk_cloudwatch::config::Credentials;
use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use aws_sdk_cloudwatch::{Client, Error};
use aws_types::region::Region;

use crate::errors::{OreMinerError, Result};

/// Represents the metrics collected during the mining process.
#[derive(Debug, Clone)]
pub struct Metrics {
    pub stake: Option<f64>,
    pub change: Option<f64>,
    pub multiplier: Option<f64>,
    pub difficulty: Option<u64>,
    pub timestamp: Option<String>,
    pub tx_hash: Option<String>,
}

impl Metrics {
    /// Creates a new instance of Metrics with all fields set to None.
    pub fn new() -> Self {
        Self {
            stake: None,
            change: None,
            multiplier: None,
            difficulty: None,
            timestamp: None,
            tx_hash: None,
        }
    }
}

pub async fn create_cloudwatch_client() -> Result<Client> {
    let region_provider =
        RegionProviderChain::first_try(std::env::var("AWS_ACCESS_REGION").ok().map(Region::new))
            .or_default_provider()
            .or_else(Region::new("us-east-1"));

    let access_key = std::env::var("AWS_ACCESS_KEY_ID")
        .map_err(|_| OreMinerError::EnvVar("AWS_ACCESS_KEY_ID not set".to_string()))?;
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
        .map_err(|_| OreMinerError::EnvVar("AWS_SECRET_ACCESS_KEY not set".to_string()))?;

    let credentials = Credentials::new(access_key, secret_key, None, None, "ore-miner-credentials");

    let config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(credentials)
        .load()
        .await;

    let client = Client::new(&config);
    tracing::info!("Created CloudWatch client");

    Ok(client)
}

fn parse_metrics(line: &str) -> Result<Metrics> {
    let trimmed_line = line.trim();
    tracing::debug!("Parsing metrics from line: {:?}", trimmed_line);

    let mut metrics = Metrics::new();
    let parts: Vec<&str> = trimmed_line.split_whitespace().collect();

    match parts.get(0) {
        Some(&"Stake:") => metrics.stake = parse_float(&parts, 1)?,
        Some(&"Change:") => metrics.change = parse_float(&parts, 1)?,
        Some(&"Multiplier:") => metrics.multiplier = parse_multiplier(&parts)?,
        Some(&"Best") if parts.get(1) == Some(&"hash:") => {
            metrics.difficulty = parse_difficulty(&parts)?
        }
        Some(&"Timestamp:") => metrics.timestamp = parse_timestamp(&parts)?,
        Some(&"OK") => {
            metrics.tx_hash = Some(
                parts
                    .get(1)
                    .ok_or(OreMinerError::ParseError("Missing tx_hash".to_string()))?
                    .to_string(),
            )
        }
        _ => return Err(OreMinerError::ParseError("Unknown metric type".to_string())),
    }

    Ok(metrics)
}

fn parse_float(parts: &[&str], index: usize) -> Result<Option<f64>> {
    parts
        .get(index)
        .ok_or_else(|| OreMinerError::ParseError("Missing value".to_string()))?
        .parse()
        .map(Some)
        .map_err(|e| OreMinerError::ParseError(format!("Failed to parse float: {}", e)))
}

fn parse_multiplier(parts: &[&str]) -> Result<Option<f64>> {
    parts
        .get(1)
        .ok_or_else(|| OreMinerError::ParseError("Missing multiplier value".to_string()))?
        .trim_end_matches('x')
        .parse()
        .map(Some)
        .map_err(|e| OreMinerError::ParseError(format!("Failed to parse multiplier: {}", e)))
}

fn parse_difficulty(parts: &[&str]) -> Result<Option<u64>> {
    parts
        .get(4)
        .ok_or_else(|| OreMinerError::ParseError("Missing difficulty value".to_string()))?
        .trim_end_matches(')')
        .parse()
        .map(Some)
        .map_err(|e| OreMinerError::ParseError(format!("Failed to parse difficulty: {}", e)))
}

fn parse_timestamp(parts: &[&str]) -> Result<Option<String>> {
    if parts.len() < 3 {
        return Err(OreMinerError::ParseError(
            "Invalid timestamp format".to_string(),
        ));
    }
    Ok(Some(format!("{}T{}Z", parts[1], parts[2])))
}

async fn send_metrics_to_cloudwatch(
    client: &Client,
    metrics: &Metrics,
) -> std::result::Result<(), Error> {
    let common_dimensions = vec![Dimension::builder()
        .name("Environment")
        .value("MainnetBeta")
        .build()];

    let metric_data = vec![
        build_metric_datum("Stake", metrics.stake, &common_dimensions),
        build_metric_datum("Change", metrics.change, &common_dimensions),
        build_metric_datum("Multiplier", metrics.multiplier, &common_dimensions),
        build_metric_datum(
            "Difficulty",
            metrics.difficulty.map(|d| d as f64),
            &common_dimensions,
        ),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();

    if !metric_data.is_empty() {
        client
            .put_metric_data()
            .namespace("OreMining")
            .set_metric_data(Some(metric_data.clone()))
            .send()
            .await?;
        tracing::info!("Sent {} metrics to CloudWatch", metric_data.len());
    } else {
        tracing::info!("No metrics to report to CloudWatch");
    }

    Ok(())
}

fn build_metric_datum(
    name: &str,
    value: Option<f64>,
    dimensions: &[Dimension],
) -> Option<MetricDatum> {
    value.map(|v| {
        MetricDatum::builder()
            .metric_name(name)
            .set_dimensions(Some(dimensions.to_vec()))
            .value(v)
            .unit(StandardUnit::None)
            .build()
    })
}

pub async fn process_mining_metrics(client: &Client, line: &str) -> Result<()> {
    if !line.is_empty() {
        let metrics = parse_metrics(line)?;
        tracing::info!("Parsed metrics: {:?}", metrics);

        send_metrics_to_cloudwatch(client, &metrics)
            .await
            .map_err(|e| {
                OreMinerError::CloudWatch(format!("Failed to send metrics to CloudWatch: {}", e))
            })?;
    }

    Ok(())
}
