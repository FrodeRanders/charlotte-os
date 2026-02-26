use alloc::vec::Vec;
use core::fmt::Debug;

use crate::logln;

#[derive(Debug)]
pub struct IdTable<T> {
    list: Vec<Option<T>>,
    available_ids: Vec<usize>,
}

impl<T> IdTable<T> {
    pub fn new() -> Self {
        IdTable {
            list: Vec::new(),
            available_ids: Vec::new(),
        }
    }

    pub fn add_element(&mut self, element: T) -> usize {
        logln!("Adding element to ID Table.");
        if let Some(id) = self.available_ids.pop() {
            logln!("ID Table: Available ID found: {id}.");
            self.list[id] = Some(element);
            logln!("ID Table: Added element to list.");
            id
        } else {
            logln!("ID Table: No available IDs. Extending list to push element.");
            let id = self.list.len();
            self.list.push(Some(element));
            logln!("ID Table: Added element to list.");
            id
        }
    }

    pub fn get(&self, element_id: usize) -> &Option<T> {
        &self.list[element_id]
    }

    pub fn get_mut(&mut self, element_id: usize) -> &mut Option<T> {
        &mut self.list[element_id]
    }

    pub fn remove_element(&mut self, element_id: usize) {
        self.list[element_id] = None;
        self.available_ids.push(element_id);
    }
}

unsafe impl<T> Send for IdTable<T> where T: Send {}
