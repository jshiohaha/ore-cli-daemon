use aws_config::meta::region::RegionProviderChain;
use aws_sdk_cloudwatch::config::Credentials;
use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};
use aws_sdk_cloudwatch::{Client, Error};
use aws_types::region::Region;

#[derive(Debug)]
struct Metrics {
    stake: Option<f64>,
    change: Option<f64>,
    multiplier: Option<f64>,
    difficulty: Option<u64>,
    timestamp: Option<String>,
    tx_hash: Option<String>,
}

pub async fn create_cloudwatch_client() -> Client {
    let region_provider =
        RegionProviderChain::first_try(std::env::var("AWS_ACCESS_REGION").ok().map(Region::new))
            .or_default_provider()
            .or_else(Region::new("us-east-1"));

    let access_key = std::env::var("AWS_ACCESS_KEY_ID").expect("AWS_ACCESS_KEY_ID must be set");
    let secret_key =
        std::env::var("AWS_SECRET_ACCESS_KEY").expect("AWS_SECRET_ACCESS_KEY must be set");

    let credentials = Credentials::new(access_key, secret_key, None, None, "ore-miner-credentials");

    let config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(credentials)
        .load()
        .await;

    let client = Client::new(&config);
    tracing::info!("Created cloudwatch client");

    return client;
}

fn parse_metrics(line: &str) -> Option<Metrics> {
    let trimmed_line = line.trim();
    tracing::info!("parse metrics from line: {:?}", trimmed_line);

    // let parts: Vec<Vec<&str>> = line
    //      .lines()
    //      .map(|line| line.trim().split_whitespace().collect())
    //      .collect();

    let mut metric = Metrics {
        stake: None,
        change: None,
        multiplier: None,
        difficulty: None,
        timestamp: None,
        tx_hash: None,
    };

    //
    // ["Best hash:", "Timestamp:", "OK"] are not available in the default ore-cli
    // client because those values are output based on a spinner's progress object
    // as opposed to stdout.
    //
    let parts: Vec<&str> = trimmed_line.split_whitespace().collect();
    tracing::info!("parse metrics parts: {:?}", parts);
    if trimmed_line.starts_with("Stake:") {
        tracing::info!("building stake metric: {:?}", parts);
        metric.stake = Some(parts[1].parse().ok()?);
    } else if trimmed_line.starts_with("Change:") {
        tracing::info!("building change metric: {:?}", parts);
        metric.change = Some(parts[1].parse().ok()?);
    } else if trimmed_line.starts_with("Multiplier:") {
        tracing::info!("building multiplier metric: {:?}", parts);
        metric.multiplier = Some(parts[1].trim_end_matches('x').parse().ok()?);
    } else if trimmed_line.starts_with("Best hash:") {
        tracing::info!("building difficulty metric: {:?}", parts);
        metric.difficulty = Some(parts[4].trim_end_matches(r")").parse().ok()?);
    } else if trimmed_line.starts_with("Timestamp:") {
        tracing::info!("building timestamp metric: {:?}", parts);
        metric.timestamp = Some(format!("{}T{}Z", parts[1], parts[2]));
    } else if trimmed_line.starts_with("OK") {
        tracing::info!("building tx_hash metric: {:?}", parts);
        metric.tx_hash = Some(parts[1].trim().to_string());
    }

    Some(metric)
}

async fn send_metrics_to_cloudwatch(client: &Client, metrics: Metrics) -> Result<(), Error> {
    let common_dimensions = vec![Dimension::builder()
        .name("Environment")
        .value("MainnetBeta")
        .build()];

    let mut metric_data: Vec<MetricDatum> = vec![];
    if let Some(stake) = metrics.stake {
        metric_data.push(
            MetricDatum::builder()
                .metric_name("Stake")
                .set_dimensions(Some(common_dimensions.clone()))
                .value(stake)
                .unit(StandardUnit::None)
                .build(),
        );
    }

    if let Some(change) = metrics.change {
        metric_data.push(
            MetricDatum::builder()
                .metric_name("Change")
                .set_dimensions(Some(common_dimensions.clone()))
                .value(change)
                .unit(StandardUnit::None)
                .build(),
        );
    }

    if let Some(multiplier) = metrics.multiplier {
        metric_data.push(
            MetricDatum::builder()
                .metric_name("Multiplier")
                .set_dimensions(Some(common_dimensions.clone()))
                .value(multiplier)
                .unit(StandardUnit::None)
                .build(),
        );
    }

    if let Some(difficulty) = metrics.difficulty {
        metric_data.push(
            MetricDatum::builder()
                .metric_name("Difficulty")
                .set_dimensions(Some(common_dimensions.clone()))
                .value(difficulty as f64)
                .unit(StandardUnit::None)
                .build(),
        );
    }

    if !metric_data.is_empty() {
        client
            .put_metric_data()
            .namespace("OreMining")
            .set_metric_data(Some(metric_data))
            .send()
            .await?;
    } else {
        tracing::info!("No metrics to report to CloudWatch");
    }

    Ok(())
}

pub async fn process_mining_metrics(client: &Client, line: &str) -> Result<(), String> {
    if line.len() > 0 {
        if let Some(metrics) = parse_metrics(line) {
            // // note: could send somewhere else, like a database?
            // tracing::info!(
            //     "[{:?}] tx_hash: {:?}",
            //     metrics.timestamp, metrics.tx_hash
            // );
            tracing::info!("{:?}", metrics);

            return send_metrics_to_cloudwatch(client, metrics)
                .await
                .map_err(|e| format!("Failed to send metrics to CloudWatch: {}", e));
        } else {
            return Err("Failed to parse metrics".to_string());
        }
    }

    Ok(())
}
