use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::Parser;
use goose::{
    config::GooseConfiguration,
    prelude::{Scenario, Transaction, TransactionFunction},
    GooseAttack, GooseError,
};
use mimalloc::MiMalloc;
use regex::Regex;
use url::Url;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// TODO error handling

struct Category {
    name: String,
    regex: Option<Regex>,
    nregex: Option<Regex>,
}

impl Category {
    pub fn is_match(&self, url: &str) -> bool {
        if let Some(regex) = &self.regex {
            regex.is_match(url)
        } else if let Some(nregex) = &self.nregex {
            !nregex.is_match(url)
        } else {
            true
        }
    }
}

/// Parse categories from a file
fn parse_category(path: impl AsRef<Path>) -> Vec<Category> {
    let str = std::fs::read_to_string(path).unwrap();
    let mut json = json::parse(&str).unwrap();
    json.members_mut()
        .map(|j| Category {
            name: j["name"].take_string().unwrap(),
            regex: j["regex"].as_str().map(|r| Regex::new(r).unwrap()),
            nregex: j["nregex"].as_str().map(|r| Regex::new(r).unwrap()),
        })
        .collect()
}

/// Create a goose task from a list of urls
fn task_from_urls(urls: Vec<String>, name: &str) -> Transaction {
    let closure: TransactionFunction = {
        Arc::new(move |user| {
            let urls = urls.clone();
            Box::pin(async move {
                let mut _size = 0;
                for url in urls.iter() {
                    let result = user.get(url).await?;
                    if let Ok(mut response) = result.response {
                        while let Ok(Some(chunk)) = response.chunk().await {
                            _size += chunk.len();
                        }
                    };
                }

                Ok(())
            })
        })
    };

    Transaction::new(closure).set_name(name)
}

/// HAR benchmark: benchmark endpoints from har files
#[derive(Parser, Debug)]
struct Cmd {
    #[clap(default_value = "./load-test")]
    /// path to har dir
    dir: PathBuf,
    #[clap(long, default_value = "localhost:8080")]
    /// Host to benchmark
    host: String,
    #[clap(short, long, default_value = "./report.hml")]
    /// Report file path
    report: String,
    #[clap(short = 't', default_value = "./")]
    /// Report file path
    run_time: String,
    #[clap(long, default_value = "false")]
    /// Disable automatic gzip decoding
    no_gzip: bool,
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    let cmd = Cmd::parse();
    // Load categories
    let categories = parse_category(cmd.dir.join("categories.json"));
    let mut conf = GooseConfiguration::default();
    conf.run_time = cmd.run_time;
    conf.host = cmd.host;
    conf.report_file = cmd.report;
    conf.no_gzip = cmd.no_gzip;
    let mut attack = GooseAttack::initialize_with_config(conf)?;
    // Load every har file
    for result in std::fs::read_dir(cmd.dir).unwrap() {
        let har_entry = result.unwrap();
        let name = har_entry.file_name().to_string_lossy().to_string();
        if name == "categories.json" {
            continue;
        }
        println!("Load har file: {}", name);
        let str = std::fs::read_to_string(har_entry.path()).unwrap();
        let har = json::parse(&str).unwrap();
        let mut urls: Vec<Vec<String>> = categories.iter().map(|_| Vec::new()).collect();
        // Load every request urls
        for entry in har["log"]["entries"].members() {
            let url = Url::parse(entry["request"]["url"].as_str().unwrap()).unwrap();
            let path = url.path();
            let pos = categories.iter().position(|c| c.is_match(path));
            if let Some(pos) = pos {
                urls[pos].push(path.to_string());
            } else {
                println!("Skipped path {}", path);
            }
        }
        let mut scenario = Scenario::new(&name);
        for (c, urls) in categories.iter().zip(urls.into_iter()) {
            scenario = scenario.register_transaction(task_from_urls(urls, &c.name))
        }
        attack = attack.register_scenario(scenario);
    }
    attack.execute().await?;

    Ok(())
}
