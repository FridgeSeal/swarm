use chrono::{offset, DateTime};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct DataSubmission<'a> {
    pub key: &'a str,
    pub value: &'a str,
}

#[derive(Serialize, Debug, PartialEq, Eq, Hash)]
pub struct NewsData {
    pub title: String,
    pub authors: Option<Vec<String>>,
    pub timestamp_published: Option<DateTime<offset::FixedOffset>>,
    pub timestamp_updated: Option<DateTime<offset::FixedOffset>>,
    pub byline: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub version: u32,
}
