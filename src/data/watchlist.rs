use super::Counter;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchlistGroup {
    pub id: u64,
    pub name: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Watchlist {
    pub group_id: Option<u64>,
    pub counters: Vec<Counter>,
    pub groups: Vec<WatchlistGroup>,
    pub hidden: bool,
    pub sort_by: (u8, u8, bool),
}

impl Watchlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_group_id(&mut self, id: u64) {
        self.group_id = Some(id);
    }

    pub fn set_counters(&mut self, counters: Vec<Counter>) {
        self.counters = counters;
    }

    pub fn counters(&self) -> &[Counter] {
        &self.counters
    }
    pub fn load(&mut self, counters: Vec<Counter>) {
        // Use HashSet to deduplicate
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();

        for counter in counters {
            if seen.insert(counter.clone()) {
                deduped.push(counter);
            }
        }

        self.counters = deduped;
    }
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }
    pub fn set_sortby(&mut self, sortby: (u8, u8, bool)) {
        self.sort_by = sortby;
    }
    pub fn refresh(&mut self) {
        self.counters.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    }
    pub fn groups(&self) -> &[WatchlistGroup] {
        &self.groups
    }
    pub fn set_groups(&mut self, groups: Vec<WatchlistGroup>) {
        self.groups = groups;
    }
    pub fn group(&self) -> Option<&WatchlistGroup> {
        let group_id = self.group_id?;
        self.groups.iter().find(|g| g.id == group_id)
    }
}
