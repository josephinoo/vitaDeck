use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};

const DATA_DIR: &str = "ux0:data/VitaDeck/";
const COLLECTIONS_FILE: &str = "ux0:data/VitaDeck/collections.txt";

const DEFAULT_NAMES: &[&str] = &["Favorites"];

pub struct Collection {
    pub name: String,
    /// Membership checks happen while rendering and filtering the library.
    /// A set keeps that work constant-time even for large collections.
    pub title_ids: HashSet<String>,
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
                            title_ids: HashSet::new(),
                        });
                    } else if let Some(current) = items.last_mut() {
                        current.title_ids.insert(line.to_string());
                    }
                }
            }
        }

        if items.is_empty() {
            items = DEFAULT_NAMES
                .iter()
                .map(|name| Collection {
                    name: (*name).to_string(),
                            title_ids: HashSet::new(),
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
            let mut title_ids: Vec<_> = collection.title_ids.iter().collect();
            title_ids.sort();
            for title_id in title_ids {
                let _ = writeln!(file, "{}", title_id);
            }
        }
    }

    pub fn contains(&self, index: usize, title_id: &str) -> bool {
        self.items
            .get(index)
            .is_some_and(|c| c.title_ids.contains(title_id))
    }

    pub fn toggle(&mut self, index: usize, title_id: &str) -> bool {
        let Some(collection) = self.items.get_mut(index) else {
            return false;
        };

        let now_inside = if collection.title_ids.remove(title_id) {
            false
        } else {
            collection.title_ids.insert(title_id.to_string());
            true
        };

        self.save();
        now_inside
    }

    pub fn remove(&mut self, index: usize, title_id: &str) -> bool {
        let Some(collection) = self.items.get_mut(index) else {
            return false;
        };
        if collection.title_ids.remove(title_id) {
            self.save();
            true
        } else {
            false
        }
    }

    pub fn create(&mut self, name: &str) -> bool {
        let name = name.trim().to_uppercase();
        if name.is_empty() || self.items.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            return false;
        }
        self.items.push(Collection {
            name,
            title_ids: HashSet::new(),
        });
        self.save();
        true
    }

    #[allow(dead_code)]
    pub fn delete(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.save();
        }
    }

    pub fn is_default(&self, index: usize) -> bool {
        index < DEFAULT_NAMES.len()
    }

    pub fn delete_custom(&mut self, index: usize) -> bool {
        if self.is_default(index) || index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        self.save();
        true
    }

    pub fn memberships(&self, title_id: &str) -> Vec<&str> {
        self.items
            .iter()
            .filter(|c| c.title_ids.contains(title_id))
            .map(|c| c.name.as_str())
            .collect()
    }
}
