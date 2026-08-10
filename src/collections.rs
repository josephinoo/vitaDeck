use std::fs::File;
use std::io::{Read, Write};

const DATA_DIR: &str = "ux0:data/VitaDeck/";
const COLLECTIONS_FILE: &str = "ux0:data/VitaDeck/collections.txt";

const DEFAULT_NAMES: &[&str] = &["Favorites"];

pub struct Collection {
    pub name: String,
    pub title_ids: Vec<String>,
}

pub struct Collections {
    pub items: Vec<Collection>,
}

impl Collections {
    pub fn load() -> Self {
        let mut items = Vec::new();

        if let Ok(mut file) = File::open(COLLECTIONS_FILE) {
            let mut text = String::new();
            if file.read_to_string(&mut text).is_ok() {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if let Some(name) = line.strip_prefix('#') {
                        items.push(Collection {
                            name: name.to_string(),
                            title_ids: Vec::new(),
                        });
                    } else if let Some(current) = items.last_mut() {
                        current.title_ids.push(line.to_string());
                    }
                }
            }
        }

        if items.is_empty() {
            items = DEFAULT_NAMES
                .iter()
                .map(|name| Collection {
                    name: (*name).to_string(),
                    title_ids: Vec::new(),
                })
                .collect();
        }

        Collections { items }
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(DATA_DIR);
        let Ok(mut file) = File::create(COLLECTIONS_FILE) else {
            return;
        };
        for collection in &self.items {
            let _ = writeln!(file, "#{}", collection.name);
            for title_id in &collection.title_ids {
                let _ = writeln!(file, "{}", title_id);
            }
        }
    }

    pub fn contains(&self, index: usize, title_id: &str) -> bool {
        self.items
            .get(index)
            .is_some_and(|c| c.title_ids.iter().any(|id| id == title_id))
    }

    pub fn toggle(&mut self, index: usize, title_id: &str) -> bool {
        let Some(collection) = self.items.get_mut(index) else {
            return false;
        };

        let now_inside = match collection.title_ids.iter().position(|id| id == title_id) {
            Some(pos) => {
                collection.title_ids.remove(pos);
                false
            }
            None => {
                collection.title_ids.push(title_id.to_string());
                true
            }
        };

        self.save();
        now_inside
    }

    pub fn remove(&mut self, index: usize, title_id: &str) -> bool {
        let Some(collection) = self.items.get_mut(index) else {
            return false;
        };
        let Some(pos) = collection.title_ids.iter().position(|id| id == title_id) else {
            return false;
        };
        collection.title_ids.remove(pos);
        self.save();
        true
    }

    #[allow(dead_code)]
    pub fn create(&mut self, name: &str) {
        if self.items.iter().any(|c| c.name == name) {
            return;
        }
        self.items.push(Collection {
            name: name.to_string(),
            title_ids: Vec::new(),
        });
        self.save();
    }

    #[allow(dead_code)]
    pub fn delete(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.save();
        }
    }

    pub fn memberships(&self, title_id: &str) -> Vec<&str> {
        self.items
            .iter()
            .filter(|c| c.title_ids.iter().any(|id| id == title_id))
            .map(|c| c.name.as_str())
            .collect()
    }
}
