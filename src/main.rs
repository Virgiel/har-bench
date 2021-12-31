use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use goose::{
    prelude::{GooseTask, GooseTaskFunction, GooseTaskSet},
    GooseAttack, GooseError,
};
use mimalloc::MiMalloc;
use regex::Regex;
use url::Url;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// TODO error handling

/// Parse categories from a file
fn parse_category(path: impl AsRef<Path>) -> Vec<(String, Regex)> {
    let str = std::fs::read_to_string(path).unwrap();
    let mut json = json::parse(&str).unwrap();
    json.members_mut()
        .map(|j| {
            (
                j["name"].take_string().unwrap(),
                Regex::new(&j["regex"].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}

/// Create a goose task from a list of urls
fn task_from_urls(urls: Vec<String>, name: &str) -> GooseTask {
    let closure: GooseTaskFunction = {
        Arc::new(move |user| {
            let urls = urls.clone();
            Box::pin(async move {
                for url in urls.iter() {
                    let result = user.get(url).await?;
                    if let Ok(response) = result.response {
                        let _bytes = response.bytes().await?;
                    };
                }

                Ok(())
            })
        })
    };
    GooseTask::new(closure).set_name(name)
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    // TODO make configurable
    let har_dir = "./load-test";
    // Load categories
    let categories = parse_category(PathBuf::from(&har_dir).join("categories.json"));
    let mut attack = GooseAttack::initialize()?;
    // Load every har file
    for result in std::fs::read_dir(har_dir).unwrap() {
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
            let pos = categories
                .iter()
                .position(|(_, pattern)| pattern.is_match(path));
            if let Some(pos) = pos {
                urls[pos].push(path.to_string());
            } else {
                println!("Skipped path {}", path);
            }
        }
        let mut set = GooseTaskSet::new(&name);
        for ((name, _), urls) in categories.iter().zip(urls.into_iter()) {
            set = set.register_task(task_from_urls(urls, name))
        }
        attack = attack.register_taskset(set);
    }
    attack.execute().await?;

    Ok(())
}
