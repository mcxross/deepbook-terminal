use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use bevy_ecs::{prelude::*, system::CommandQueue};
use crossterm::event::KeyEvent;
use futures::future::BoxFuture;
use ratatui::widgets::TableState;
use tokio::sync::{mpsc, watch};
use tui_input::backend::crossterm::EventHandler;

use crate::utils::cycle;

#[derive(Resource, Component)]
pub struct LocalSearch<T> {
    pub(crate) input: tui_input::Input,
    pub(crate) table: TableState,
    visible: bool,
    items: Vec<T>,
    results: Vec<T>,
    func: fn(&str, &T) -> bool,
}

impl<T> std::fmt::Debug for LocalSearch<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSearch")
            .field("visible", &self.visible)
            .field("input", &self.input.value())
            .field("items", &self.items)
            .finish_non_exhaustive()
    }
}

impl<T> LocalSearch<T>
where
    T: Clone + Send + 'static,
{
    pub fn new(items: Vec<T>, func: fn(&str, &T) -> bool) -> Self {
        Self {
            input: tui_input::Input::default(),
            table: TableState::default(),
            visible: false,
            results: items.clone(),
            items,
            func,
        }
    }

    pub fn visible(&mut self) {
        self.visible = true;
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> (bool, Option<T>) {
        match event {
            key!(Esc) => {
                self.visible = false;
                return (true, None);
            }
            key!(Enter) => {
                if let Some(idx) = self.table.selected() {
                    self.table.select(None);
                    let selected = self.result(idx);
                    self.input.reset();
                    self.results = self.items.clone();
                    self.visible = false;
                    return (true, Some(selected));
                }
            }
            key!(Up) => {
                let idx = cycle::prev_opt(self.table.selected(), self.results.len());
                self.table.select(idx);
            }
            key!(Down) => {
                let idx = cycle::next_opt(self.table.selected(), self.results.len());
                self.table.select(idx);
            }
            _ => {
                let evt = crossterm::event::Event::Key(event);
                if self.input.handle_event(&evt).is_some() {
                    let keyword = self.input.value();
                    self.results = self
                        .items
                        .iter()
                        .filter(|v| keyword.is_empty() || (self.func)(keyword, v))
                        .cloned()
                        .collect();
                }
            }
        }
        (false, None)
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn results(&self) -> &[T] {
        &self.results
    }

    fn result(&self, index: usize) -> T {
        self.results.get(index).cloned().expect("index in range")
    }
}

// ------------

#[derive(Resource, Component)]
pub struct Search<T> {
    pub(crate) input: tui_input::Input,
    pub(crate) table: TableState,
    visible: bool,
    results: Arc<Mutex<Vec<T>>>,
    history: Vec<T>,
    tx: watch::Sender<String>,
}

impl<T> std::fmt::Debug for Search<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Search")
            .field("visible", &self.visible)
            .field("input", &self.input.value())
            .field("results", &self.results)
            .finish_non_exhaustive()
    }
}

impl<T> Search<T>
where
    T: Clone + PartialEq + Send + 'static,
{
    pub fn new(
        update: mpsc::UnboundedSender<CommandQueue>,
        task: impl Fn(String) -> BoxFuture<'static, Vec<T>> + Send + Sync + 'static,
    ) -> Self {
        let (tx, mut rx) = watch::channel(String::new());
        let results = Arc::new(Mutex::new(vec![]));
        tokio::spawn({
            let results = results.clone();
            async move {
                loop {
                    _ = rx.changed().await;
                    // debounce input
                    loop {
                        match tokio::time::timeout(Duration::from_millis(10), rx.changed()).await {
                            Ok(Ok(_)) => {}
                            Ok(Err(_)) => return,
                            Err(_) => break,
                        }
                    }
                    let input = rx.borrow_and_update().to_string();
                    if input.is_empty() {
                        results.lock().unwrap().clear();
                    } else {
                        let result = (task)(input).await;
                        *results.lock().unwrap() = result;
                    }
                    let _ = update.send(CommandQueue::default());
                }
            }
        });
        Self {
            input: tui_input::Input::default(),
            table: TableState::default(),
            visible: false,
            results,
            history: vec![],
            tx,
        }
    }

    pub fn visible(&mut self) {
        self.visible = true;
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> (bool, Option<T>) {
        match event {
            key!(Esc) => {
                self.visible = false;
                return (true, None);
            }
            key!(Enter) => {
                if let Some(idx) = self.table.selected() {
                    self.table.select(None);
                    let selected = self.result(idx);
                    if self.history.len() >= 20 {
                        self.history.pop();
                    }
                    self.history.retain(|v| v != &selected);
                    self.history.insert(0, selected.clone());

                    self.input.reset();
                    self.results.lock().unwrap().clear();
                    self.visible = false;
                    return (true, Some(selected));
                }
            }
            key!(Up) => {
                let idx = cycle::prev_opt(self.table.selected(), self.results_length());
                self.table.select(idx);
            }
            key!(Down) => {
                let idx = cycle::next_opt(self.table.selected(), self.results_length());
                self.table.select(idx);
            }
            _ => {
                let evt = crossterm::event::Event::Key(event);
                if self.input.handle_event(&evt).is_some() {
                    let _ = self.tx.send(self.input.to_string());
                }
            }
        }
        (false, None)
    }

    pub fn results(&self) -> Vec<T> {
        let items = self.results.lock().unwrap();
        if !items.is_empty() {
            return items.clone();
        }
        if self.input.value().is_empty() {
            self.history.clone()
        } else {
            vec![]
        }
    }

    fn results_length(&self) -> usize {
        let items = self.results.lock().unwrap();
        if !items.is_empty() {
            return items.len();
        }
        if self.input.value().is_empty() {
            self.history.len()
        } else {
            0
        }
    }

    fn result(&self, index: usize) -> T {
        self.results
            .lock()
            .unwrap()
            .get(index)
            .or_else(|| self.history.get(index))
            .cloned()
            .expect("index in range")
    }
}
