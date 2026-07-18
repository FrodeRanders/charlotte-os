use alloc::vec::Vec;
use core::fmt::Debug;

use crate::logln;

#[derive(Debug)]
pub enum Error {
    IdNotActive,
}

#[derive(Debug)]
pub struct IdTable<T> {
    list: Vec<Option<T>>,
    available_ids: Vec<usize>,
}

impl<'a, T> IdTable<T> {
    pub fn new() -> Self {
        IdTable {
            list: Vec::new(),
            available_ids: Vec::new(),
        }
    }

    pub fn add_element(&mut self, element: T) -> usize {
        if let Some(id) = self.available_ids.pop() {
            self.list[id] = Some(element);
            id
        } else {
            logln!("ID Table: extending list (new size={})", self.list.len() + 1);
            let id = self.list.len();
            self.list.push(Some(element));
            id
        }
    }

    pub fn get(&self, element_id: usize) -> Result<&T, Error> {
        self.list.get(element_id).ok_or(Error::IdNotActive)?.as_ref().ok_or(Error::IdNotActive)
    }

    pub fn get_mut(&mut self, element_id: usize) -> Result<&mut T, Error> {
        self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.as_mut().ok_or(Error::IdNotActive)
    }

    pub fn remove_element(&mut self, element_id: usize) -> Result<(), Error> {
        match self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.take() {
            Some(_) => {
                self.available_ids.push(element_id);
                Ok(())
            }
            None => Err(Error::IdNotActive),
        }
    }

    pub fn take_element(&mut self, element_id: usize) -> Result<T, Error> {
        match self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.take() {
            Some(element) => {
                self.available_ids.push(element_id);
                Ok(element)
            }
            None => Err(Error::IdNotActive),
        }
    }

    pub fn iter(&'a self) -> core::slice::Iter<'a, Option<T>> {
        self.list.iter()
    }

    pub fn iter_mut(&'a mut self) -> core::slice::IterMut<'a, Option<T>> {
        self.list.iter_mut()
    }
}

unsafe impl<T> Send for IdTable<T> where T: Send {}
unsafe impl<T> Sync for IdTable<T> where T: Sync {}
