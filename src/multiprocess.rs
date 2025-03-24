//! Multiprocess support for rust-prometheus
//!
//! This module provides a simple implementation of multiprocess metric aggregation inspired by the
//! Prometheus Python client. It currently supports aggregating counter metrics across multiple processes.

use crate::core::{Collector, Desc};
use crate::proto;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

/// A collector that aggregates metrics from multiple processes
/// by reading files from the directory specified in the
/// `PROMETHEUS_MULTIPROC_DIR` environment variable.
///
/// This implementation currently supports only counter metrics in a simple file format.
/// Each file in the multiprocess directory should contain lines in the following format:
///
///   metric_name: value
///
/// Lines starting with '#' are considered comments and ignored.
///
/// For a more complete implementation, support for gauges, histograms, and summaries can be added.
pub struct MultiProcessCollector {
    multiproc_dir: PathBuf,
}

impl MultiProcessCollector {
    /// Creates a new `MultiProcessCollector`.
    ///
    /// This function will panic if the environment variable `PROMETHEUS_MULTIPROC_DIR` is not set.
    pub fn new() -> Self {
        let dir = env::var("PROMETHEUS_MULTIPROC_DIR")
            .expect("PROMETHEUS_MULTIPROC_DIR is not set")
            .into();
        Self { multiproc_dir: dir }
    }

    /// Helper function to parse a single file from the multiprocess directory.
    /// It expects each non-comment line to be in the format: "metric_name: value".
    fn parse_file(&self, path: &PathBuf) -> HashMap<String, f64> {
        let mut metrics = HashMap::new();
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(idx) = line.find(':') {
                    let key = line[..idx].trim().to_string();
                    let value_str = line[idx + 1..].trim();
                    if let Ok(value) = value_str.parse::<f64>() {
                        // Sum values if the key is already present
                        *metrics.entry(key).or_insert(0.0) += value;
                    }
                }
            }
        }
        metrics
    }
}

impl Collector for MultiProcessCollector {
    fn desc(&self) -> Vec<&Desc> {
        // As this is a dynamic collector, we don't have static descriptors.
        Vec::new()
    }

    fn collect(&self) -> Vec<proto::MetricFamily> {
        let mut aggregated: HashMap<String, f64> = HashMap::new();

        if let Ok(entries) = fs::read_dir(&self.multiproc_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // Only process files
                if path.is_file() {
                    let file_metrics = self.parse_file(&path);
                    for (key, value) in file_metrics {
                        *aggregated.entry(key).or_insert(0.0) += value;
                    }
                }
            }
        }

        // Convert aggregated results into MetricFamily protos for counter metrics.
        let mut mfs = Vec::new();
        for (name, value) in aggregated.into_iter() {
            let mut counter = proto::Counter::default();
            counter.set_value(value);

            let mut metric = proto::Metric::default();
            metric.set_counter(counter);

            let mut mf = proto::MetricFamily::default();
            mf.set_name(name);
            mf.set_help("Aggregated multiprocess counter metric".to_string());
            mf.set_field_type(proto::MetricType::COUNTER);
            mf.mut_metric().push(metric);
            mfs.push(mf);
        }

        mfs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto_ext::MessageFieldExt;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_parse_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test_metric.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        writeln!(file, "# a comment\ntest_counter: 5.0\ntest_counter: 2.5").unwrap();

        let collector = MultiProcessCollector {
            multiproc_dir: dir.path().to_path_buf(),
        };
        let metrics = collector.parse_file(&file_path);
        assert_eq!(metrics.get("test_counter"), Some(&7.5));
    }

    #[test]
    fn test_collect() {
        let dir = tempdir().unwrap();
        // Create two metric files
        let file1 = dir.path().join("m1.txt");
        let file2 = dir.path().join("m2.txt");
        fs::write(&file1, "metric_a: 3.0\nmetric_b: 1.0").unwrap();
        fs::write(&file2, "metric_a: 2.0\nmetric_b: 4.0").unwrap();

        let collector = MultiProcessCollector {
            multiproc_dir: dir.path().to_path_buf(),
        };
        let mfs = collector.collect();
        let mut results = std::collections::HashMap::new();
        for mf in mfs {
            if mf.get_field_type() == proto::MetricType::COUNTER {
                if let Some(metric) = mf.get_metric().get(0) {
                    results.insert(mf.name().to_string(), metric.get_counter().get_value());
                }
            }
        }

        assert_eq!(results.get("metric_a"), Some(&5.0));
        assert_eq!(results.get("metric_b"), Some(&5.0));
    }
}
