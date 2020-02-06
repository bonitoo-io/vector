use crate::event::metric::{Metric, MetricKind, MetricValue};
use chrono::{DateTime, Utc};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;

pub enum Field {
    /// string
    String(String),
    /// float
    Float(f64),
    /// unsigned integer
    UnsignedInt(u32),
}

fn encode_events(events: Vec<Metric>, namespace: &str) -> Vec<String> {
    events
        .into_iter()
        .filter_map(|event| {
            let fullname = encode_namespace(namespace, &event.name);
            let ts = encode_timestamp(event.timestamp);
            let tags = event.tags.clone();
            match event.value {
                MetricValue::Counter { value } => {
                    let fields = to_fields(value);

                    Some(vec![influx_line_protocol(
                        fullname,
                        "counter",
                        tags,
                        Some(fields),
                        ts,
                    )])
                }
                MetricValue::Gauge { value } => {
                    let fields = to_fields(value);

                    Some(vec![influx_line_protocol(
                        fullname,
                        "gauge",
                        tags,
                        Some(fields),
                        ts,
                    )])
                }
                MetricValue::Set { values } => {
                    let fields = to_fields(values.len() as f64);

                    Some(vec![influx_line_protocol(
                        fullname,
                        "set",
                        tags,
                        Some(fields),
                        ts,
                    )])
                }
                MetricValue::AggregatedHistogram {
                    buckets,
                    counts,
                    count,
                    sum,
                } => {
                    let mut fields: HashMap<String, Field> = buckets
                        .iter()
                        .zip(counts.iter())
                        .map(|pair| (format!("bucket_{}", pair.0), Field::UnsignedInt(*pair.1)))
                        .collect();
                    fields.insert("count".to_owned(), Field::UnsignedInt(count));
                    fields.insert("sum".to_owned(), Field::Float(sum));

                    Some(vec![influx_line_protocol(
                        fullname,
                        "histogram",
                        tags,
                        Some(fields),
                        ts,
                    )])
                }
                MetricValue::AggregatedSummary {
                    quantiles,
                    values,
                    count,
                    sum,
                } => {
                    let mut fields: HashMap<String, Field> = quantiles
                        .iter()
                        .zip(values.iter())
                        .map(|pair| (format!("quantile_{}", pair.0), Field::Float(*pair.1)))
                        .collect();
                    fields.insert("count".to_owned(), Field::UnsignedInt(count));
                    fields.insert("sum".to_owned(), Field::Float(sum));

                    Some(vec![influx_line_protocol(
                        fullname,
                        "summary",
                        tags,
                        Some(fields),
                        ts,
                    )])
                }
                MetricValue::Distribution {
                    values,
                    sample_rates,
                } => {
                    let fields = encode_distribution(&values, &sample_rates);
                    Some(vec![influx_line_protocol(
                        fullname,
                        "distribution",
                        tags,
                        fields,
                        ts,
                    )])
                }
            }
        })
        .flatten()
        .filter(|lp| !lp.is_empty())
        .collect()
}

fn encode_distribution(values: &[f64], counts: &[u32]) -> Option<HashMap<String, Field>> {
    if values.len() != counts.len() {
        return None;
    }

    let mut samples = Vec::new();
    for (v, c) in values.iter().zip(counts.iter()) {
        for _ in 0..*c {
            samples.push(*v);
        }
    }

    if samples.is_empty() {
        return None;
    }

    if samples.len() == 1 {
        let val = samples[0];
        return Some(
            vec![
                ("min".to_owned(), Field::Float(val)),
                ("max".to_owned(), Field::Float(val)),
                ("median".to_owned(), Field::Float(val)),
                ("avg".to_owned(), Field::Float(val)),
                ("sum".to_owned(), Field::Float(val)),
                ("count".to_owned(), Field::Float(1.0)),
                ("quantile_0.95".to_owned(), Field::Float(val)),
            ]
            .into_iter()
            .collect(),
        );
    }

    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let length = samples.len() as f64;
    let min = samples.first().unwrap();
    let max = samples.last().unwrap();

    let p50 = samples[(0.50 * length - 1.0).round() as usize];
    let p95 = samples[(0.95 * length - 1.0).round() as usize];

    let sum = samples.iter().sum();
    let avg = sum / length;

    let fields: HashMap<String, Field> = vec![
        ("min".to_owned(), Field::Float(*min)),
        ("max".to_owned(), Field::Float(*max)),
        ("median".to_owned(), Field::Float(p50)),
        ("avg".to_owned(), Field::Float(avg)),
        ("sum".to_owned(), Field::Float(sum)),
        ("count".to_owned(), Field::Float(length)),
        ("quantile_0.95".to_owned(), Field::Float(p95)),
    ]
    .into_iter()
    .collect();

    Some(fields)
}

fn influx_line_protocol(
    measurement: String,
    metric_type: &str,
    tags: Option<HashMap<String, String>>,
    fields: Option<HashMap<String, Field>>,
    timestamp: i64,
) -> String {
    let mut line_protocol = vec![encode_key(measurement)];

    // Tags
    let mut unwrapped_tags = tags.unwrap_or(HashMap::new());
    unwrapped_tags.insert("metric_type".to_owned(), metric_type.to_owned());
    line_protocol.push(format!(",{}", encode_tags(unwrapped_tags)));

    // Fields
    let unwrapped_fields = fields.unwrap_or(HashMap::new());
    let encoded_fields = encode_fields(unwrapped_fields);
    if encoded_fields.is_empty() {
        return "".to_owned();
    }
    line_protocol.push(format!(" {}", encoded_fields));

    // Timestamp
    line_protocol.push(format!(" {}", timestamp));

    line_protocol.join("")
}

fn encode_key(key: String) -> String {
    key.replace("\\", "\\\\")
        .replace(",", "\\,")
        .replace(" ", "\\ ")
        .replace("=", "\\=")
}

fn encode_tags(tags: HashMap<String, String>) -> String {
    let ordered: Vec<String> = tags
        // sort by key
        .iter()
        .collect::<BTreeMap<_, _>>()
        // map to key=value
        .iter()
        .map(|pair| {
            let key = encode_key(pair.0.to_string());
            let value = encode_key(pair.1.to_string());
            if !key.is_empty() && !value.is_empty() {
                format!("{}={}", key, value)
            } else {
                "".to_string()
            }
        })
        // filter empty
        .filter(|tag_value| !tag_value.is_empty())
        .collect();

    ordered.join(",")
}

fn encode_fields(fields: HashMap<String, Field>) -> String {
    let encoded = fields
        // sort by key
        .iter()
        .collect::<BTreeMap<_, _>>()
        // map to key=value
        .iter()
        .map(|pair| {
            let key = encode_key(pair.0.to_string());
            let value = match pair.1 {
                Field::String(s) => {
                    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"");
                    format!("\"{}\"", escaped)
                }
                Field::Float(f) => f.to_string(),
                Field::UnsignedInt(i) => format!("{}i", i.to_string()),
            };
            if !key.is_empty() && !value.is_empty() {
                format!("{}={}", key, value)
            } else {
                "".to_string()
            }
        })
        .filter(|field_value| !field_value.is_empty())
        .collect::<Vec<String>>();

    encoded.join(",")
}

fn encode_timestamp(timestamp: Option<DateTime<Utc>>) -> i64 {
    if let Some(ts) = timestamp {
        ts.timestamp_nanos()
    } else {
        encode_timestamp(Some(Utc::now()))
    }
}

fn encode_namespace(namespace: &str, name: &str) -> String {
    if !namespace.is_empty() {
        format!("{}.{}", namespace, name)
    } else {
        name.to_string()
    }
}

fn to_fields(value: f64) -> HashMap<String, Field> {
    let fields: HashMap<String, Field> = vec![("value".to_owned(), Field::Float(value))]
        .into_iter()
        .collect();
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::offset::TimeZone;
    use pretty_assertions::assert_eq;

    fn ts() -> DateTime<Utc> {
        Utc.ymd(2018, 11, 14).and_hms_nano(8, 9, 10, 11)
    }

    fn tags() -> HashMap<String, String> {
        vec![
            ("normal_tag".to_owned(), "value".to_owned()),
            ("true_tag".to_owned(), "true".to_owned()),
            ("empty_tag".to_owned(), "".to_owned()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn test_encode_timestamp() {
        let start = Utc::now().timestamp_nanos();
        assert_eq!(encode_timestamp(Some(ts())), 1542182950000000011);
        assert!(encode_timestamp(None) >= start)
    }

    #[test]
    fn test_encode_namespace() {
        assert_eq!(encode_namespace("services", "status"), "services.status");
        assert_eq!(encode_namespace("", "status"), "status")
    }

    #[test]
    fn test_encode_key() {
        assert_eq!(
            encode_key("measurement_name".to_string()),
            "measurement_name"
        );
        assert_eq!(
            encode_key("measurement name".to_string()),
            "measurement\\ name"
        );
        assert_eq!(
            encode_key("measurement=name".to_string()),
            "measurement\\=name"
        );
        assert_eq!(
            encode_key("measurement,name".to_string()),
            "measurement\\,name"
        );
    }

    #[test]
    fn test_encode_tags() {
        assert_eq!(encode_tags(tags()), "normal_tag=value,true_tag=true");

        let tags_to_escape = vec![
            ("tag".to_owned(), "val=ue".to_owned()),
            ("name escape".to_owned(), "true".to_owned()),
            ("value_escape".to_owned(), "value escape".to_owned()),
            ("a_first_place".to_owned(), "10".to_owned()),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            encode_tags(tags_to_escape),
            "a_first_place=10,name\\ escape=true,tag=val\\=ue,value_escape=value\\ escape"
        );
    }

    #[test]
    fn test_encode_fields() {
        let fields = vec![
            (
                "field_string".to_owned(),
                Field::String("string value".to_owned()),
            ),
            (
                "field_string_escape".to_owned(),
                Field::String("string\\val\"ue".to_owned()),
            ),
            ("field_float".to_owned(), Field::Float(123.45)),
            ("escape key".to_owned(), Field::Float(10.0)),
        ]
        .into_iter()
        .collect();

        assert_eq!(encode_fields(fields), "escape\\ key=10,field_float=123.45,field_string=\"string value\",field_string_escape=\"string\\\\val\\\"ue\"");
    }

    #[test]
    fn encode_counter() {
        let events = vec![
            Metric {
                name: "total".into(),
                timestamp: Some(ts()),
                tags: None,
                kind: MetricKind::Incremental,
                value: MetricValue::Counter { value: 1.5 },
            },
            Metric {
                name: "check".into(),
                timestamp: Some(ts()),
                tags: Some(tags()),
                kind: MetricKind::Incremental,
                value: MetricValue::Counter { value: 1.0 },
            },
        ];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec!["ns.total,metric_type=counter value=1.5 1542182950000000011", "ns.check,metric_type=counter,normal_tag=value,true_tag=true value=1 1542182950000000011", ]
        );
    }

    #[test]
    fn encode_gauge() {
        let events = vec![Metric {
            name: "meter".to_owned(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Incremental,
            value: MetricValue::Gauge { value: -1.5 },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec!["ns.meter,metric_type=gauge,normal_tag=value,true_tag=true value=-1.5 1542182950000000011", ]
        );
    }

    #[test]
    fn encode_set() {
        let events = vec![Metric {
            name: "users".into(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Incremental,
            value: MetricValue::Set {
                values: vec!["alice".into(), "bob".into()].into_iter().collect(),
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec!["ns.users,metric_type=set,normal_tag=value,true_tag=true value=2 1542182950000000011", ]
        );
    }

    #[test]
    fn encode_histogram() {
        let events = vec![Metric {
            name: "requests".to_owned(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Absolute,
            value: MetricValue::AggregatedHistogram {
                buckets: vec![1.0, 2.1, 3.0],
                counts: vec![1, 2, 3],
                count: 6,
                sum: 12.5,
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec!["ns.requests,metric_type=histogram,normal_tag=value,true_tag=true bucket_1=1i,bucket_2.1=2i,bucket_3=3i,count=6i,sum=12.5 1542182950000000011", ]
        );
    }

    #[test]
    fn encode_summary() {
        let events = vec![Metric {
            name: "requests_sum".to_owned(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Absolute,
            value: MetricValue::AggregatedSummary {
                quantiles: vec![0.01, 0.5, 0.99],
                values: vec![1.5, 2.0, 3.0],
                count: 6,
                sum: 12.0,
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec!["ns.requests_sum,metric_type=summary,normal_tag=value,true_tag=true count=6i,quantile_0.01=1.5,quantile_0.5=2,quantile_0.99=3,sum=12 1542182950000000011", ]
        );
    }

    #[test]
    fn encode_distribution() {
        let events = vec![
            Metric {
                name: "requests".into(),
                timestamp: Some(ts()),
                tags: Some(tags()),
                kind: MetricKind::Incremental,
                value: MetricValue::Distribution {
                    values: vec![1.0, 2.0, 3.0],
                    sample_rates: vec![3, 3, 2],
                },
            },
            Metric {
                name: "dense_stats".into(),
                timestamp: Some(ts()),
                tags: None,
                kind: MetricKind::Incremental,
                value: MetricValue::Distribution {
                    values: (0..20).into_iter().map(f64::from).collect::<Vec<_>>(),
                    sample_rates: vec![1; 20],
                },
            },
            Metric {
                name: "sparse_stats".into(),
                timestamp: Some(ts()),
                tags: None,
                kind: MetricKind::Incremental,
                value: MetricValue::Distribution {
                    values: (1..5).into_iter().map(f64::from).collect::<Vec<_>>(),
                    sample_rates: (1..5).into_iter().collect::<Vec<_>>(),
                },
            },
        ];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(
            line_protocols,
            vec![
                "ns.requests,metric_type=distribution,normal_tag=value,true_tag=true avg=1.875,count=8,max=3,median=2,min=1,quantile_0.95=3,sum=15 1542182950000000011",
                "ns.dense_stats,metric_type=distribution avg=9.5,count=20,max=19,median=9,min=0,quantile_0.95=18,sum=190 1542182950000000011",
                "ns.sparse_stats,metric_type=distribution avg=3,count=10,max=4,median=3,min=1,quantile_0.95=4,sum=30 1542182950000000011",
            ]
        );
    }

    #[test]
    fn encode_distribution_empty_stats() {
        let events = vec![Metric {
            name: "requests".into(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Incremental,
            value: MetricValue::Distribution {
                values: vec![],
                sample_rates: vec![],
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(line_protocols.len(), 0);
    }

    #[test]
    fn encode_distribution_zero_counts_stats() {
        let events = vec![Metric {
            name: "requests".into(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Incremental,
            value: MetricValue::Distribution {
                values: vec![1.0, 2.0],
                sample_rates: vec![0, 0],
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(line_protocols.len(), 0);
    }

    #[test]
    fn encode_distribution_unequal_stats() {
        let events = vec![Metric {
            name: "requests".into(),
            timestamp: Some(ts()),
            tags: Some(tags()),
            kind: MetricKind::Incremental,
            value: MetricValue::Distribution {
                values: vec![1.0],
                sample_rates: vec![1, 2, 3],
            },
        }];

        let line_protocols = encode_events(events, "ns");
        assert_eq!(line_protocols.len(), 0);
    }
}
