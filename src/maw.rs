use anyhow::Result;
use bincode;
use chrono;
use fnv::FnvHashMap;
use futures::stream::StreamExt;
use futures::{future, pin_mut};
use lazy_static::lazy_static;
use log;
use regex::Regex;
use reqwest;
use sled;
use soup::prelude::*;
use std::collections::HashSet;
use stork;
use stork_http;
use stork_http::filters;
use url::Url;

mod settings;
mod structures;

use future::join_all;
use serde::Serialize;
use settings::Settings;

lazy_static! {
    static ref RE_DATE: Regex = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    static ref RE_ID_SLUG: Regex = Regex::new(r"^\d{8,}$").unwrap();
    static ref INVALID_TEXT: Regex = Regex::new(r"\s{2,}|(\n)|(\r)|(\t)|(\\)").unwrap();
}

#[tokio::main]
async fn main() -> Result<()> {
    simple_logger::init_with_level(log::Level::Info).unwrap();
    let cfg = Settings::new().unwrap();
    let sled_config = sled::Config::default()
        .use_compression(cfg.db_compression_enabled)
        .path(&cfg.database_name);
    let sled_db = sled_config.open().unwrap();
    log::info!(
        "Sled info:\nwas recovered? {:?}\ndisk size: {:?}\nlength: {:?}",
        sled_db.was_recovered(),
        sled_db.size_on_disk(),
        sled_db.len()
    );
    let http_client = reqwest::Client::new();
    let example_domain = cfg.crawl_domains.first().unwrap();
    let (content_set, dir_pages) = crawl_site(example_domain).await;
    log::info!(
        "N content pages: {}\nN directory pages: {}",
        content_set.len(),
        dir_pages.len()
    );
    let text_data_future: Vec<Result<(String, String)>> =
        join_all(content_set.iter().map(|url| dl_page(&http_client, url))).await;
    log::info!("Pages extracted");
    let text_data: FnvHashMap<String, structures::NewsData> = text_data_future
        .iter()
        .filter_map(|x| x.as_ref().ok())
        .filter_map(|(k, v)| {
            let v2 = extract_page_contents(v).ok();
            match v2 {
                Some(x) => Some((k.to_owned(), x)),
                None => None,
            }
        })
        .collect();
    log::info!("Pages parsed");
    batch_write(&sled_db, text_data).unwrap();
    log::info!("Pages written to database!");
    Ok(())
}

async fn crawl_site(root_url: &str) -> (HashSet<Url>, HashSet<Url>) {
    let site_stream = stork_http::HttpStorkable::new(root_url.parse().unwrap())
        .with_filters(
            stork::FilterSet::default()
                .add_filter(filters::DomainFilter::new("www.abc.net.au"))
                .add_filter(filters::PathFilter::new(
                    filters::FilterType::StartsWith,
                    "/news",
                )),
        )
        .exec();
    let mut pages = HashSet::with_capacity(500);
    let mut directories = HashSet::with_capacity(300);
    pin_mut!(site_stream);
    while let Some(page_link) = site_stream.next().await {
        // stream return an asynchronous 'stream' (duh) of storkables
        // for each one that is a page and not a directory
        // grab the contents and process it
        // every one that is a directory/subsection of the page
        // re-submit it for "storking"
        let page_url = page_link.unwrap().val().url().clone();
        // log::info!("Crawling page: {}", page_url);
        let path_segments_vec: Vec<&str> = page_url
            .path_segments()
            .map(|c| c.collect())
            .unwrap_or_default();
        let is_story_content = verify_path(&path_segments_vec);
        if is_story_content {
            // log::info!("Page was story content");
            pages.insert(page_url);
        } else {
            // log::info!("Page was directory content");
            directories.insert(page_url);
        };
    }
    (pages, directories)
}

fn verify_path(path: &[&str]) -> bool {
    match path {
        [] => {
            log::error!("Empty path! How did this even happen?");
            false
        }
        [a, b, _, d] => {
            let pred1 = a == &"news";
            let pred2 = RE_DATE.is_match(b);
            let pred4 = RE_ID_SLUG.is_match(d);
            pred1 && pred2 && pred4
        }
        _ => false,
    }
}

async fn dl_page(client: &reqwest::Client, url: &Url) -> Result<(String, String)> {
    // log::info!("Scraping page: {}", url);
    let id = url
        .path_segments()
        .map(|x| x.last())
        .flatten()
        // .map(|x| x.parse::<u32>().unwrap())
        .unwrap();
    // log::info!("URL ID: {}", id);
    Ok((
        id.into(),
        client.get(url.as_str()).send().await?.text().await?,
    ))
}

fn extract_page_contents(raw_html: &str) -> Result<structures::NewsData> {
    // log::info!("Extracting content from html");
    let t_strfmt = "%FT%X%z";
    let data = Soup::new(&raw_html);
    let tstamp_published = data
        .tag("meta")
        .attr("property", "article:published_time")
        .find()
        .and_then(|x| x.get("content"))
        .as_ref()
        .map(|txt| INVALID_TEXT.replace_all(txt, ""))
        .and_then(|ts| chrono::DateTime::parse_from_str(&ts, &t_strfmt).ok());
    let tstamp_updated = data
        .tag("meta")
        .attr("property", "article:modified_time")
        .find()
        .and_then(|x| x.get("content"))
        .as_ref()
        .map(|txt| INVALID_TEXT.replace_all(txt, ""))
        .and_then(|ts| chrono::DateTime::parse_from_str(&ts, &t_strfmt).ok());
    let metadata_tags: Vec<String> = data
        .tag("meta")
        .attr("property", "article:tag")
        .find_all()
        .map(|n| n.get("content").expect("Couldn't get content"))
        .collect();
    let title = data
        .tag("meta")
        .attr("name", "title")
        .find()
        .expect("Couldn't find the stuff")
        .get("content")
        .unwrap_or_default();
    let byline = data
        .tag("meta")
        .attr("name", "description")
        .find()
        .map(|x| x.text());
    let mut text = data
        .tag("p")
        .find_all()
        .map(|t| t.text())
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    text = INVALID_TEXT.replace_all(&text, "").to_string();
    // println!();
    // log::info!(
    //     "published: {:?}\nupdated: {:?}",
    //     tstamp_published,
    //     tstamp_updated
    // );
    // log::info!("tags: {:?}", metadata_tags);
    // log::info!("title: {}", title);
    // log::info!("First ~200 chars of content: {:?}", &text.get(0..250));
    Ok(structures::NewsData {
        title,
        authors: None,
        timestamp_published: tstamp_published,
        timestamp_updated: tstamp_updated,
        byline,
        content: text,
        tags: metadata_tags,
        version: 1,
    })
}

fn encode(input: impl Serialize) -> Result<Vec<u8>> {
    Ok(bincode::serialize(&input)?)
}

fn batch_write(
    database: &sled::Db,
    dataset: FnvHashMap<String, structures::NewsData>,
) -> Result<()> {
    let mut batch = sled::Batch::default();
    let _: Vec<()> = dataset
        .iter()
        .map(|(k, v)| (k, encode(v)))
        .map(|(k, v)| batch.insert(k.as_bytes(), v.unwrap().as_slice()))
        .collect();
    match database.apply_batch(batch) {
        Ok(x) => log::info!("Great success: {:?}", x),
        Err(e) => log::error!("Fucking failure. {:?}", e),
    }
    // Ok(database.apply_batch(batch)?)
    Ok(())
}
