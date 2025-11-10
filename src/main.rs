use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct EventsData {
    events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Event {
    id: u8,
    name: String,
    cost: u8,
    base_score: u8,
    size: u8,
    requirements: Vec<String>,
    penalty_scores: Vec<u8>,
}

impl Event {
    fn format_size(&self) -> &str {
        match self.size {
            0 => "[]",
            1 => "[=]",
            2 => "[==]",
            _ => "[???]",
        }
    }

    fn format_requirements(&self) -> String {
        let mut counts: HashMap<&str, u32> = HashMap::new();
        for req in &self.requirements {
            *counts.entry(req.as_str()).or_insert(0) += 1;
        }

        let mut items: Vec<String> = counts
            .iter()
            .map(|(asset, count)| format!("{}: {}", asset, count))
            .collect();
        items.sort();
        items.join(", ")
    }

    fn format_penalties(&self) -> String {
        self.penalty_scores
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn main() {
    const YAML_CONTENT: &str = include_str!("../data/events.yml");

    let data: EventsData = serde_yml::from_str(YAML_CONTENT).expect("Failed to parse YAML");

    for event in data.events {
        println!(
            "{:2}  {:28}  ${:2}  {:4}  {:2} ({:21})  {}",
            event.id,
            event.name,
            event.cost,
            event.format_size(),
            event.base_score,
            event.format_penalties(),
            event.format_requirements()
        );
    }
}
