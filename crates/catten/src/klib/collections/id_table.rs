use alloc::vec::Vec;
use core::fmt::Debug;

#[derive(Debug)]
pub enum Error {
    IdNotActive,
}

#[derive(Debug)]
pub struct IdTable<T> {
    list: Vec<Option<T>>,
    available_ids: Vec<usize>,
    generations: Vec<usize>,
}

impl<T> IdTable<T> {
    pub fn new() -> Self {
        IdTable {
            list: Vec::new(),
            available_ids: Vec::new(),
            generations: Vec::new(),
        }
    }

    pub fn add_element(&mut self, element: T) -> usize {
        if let Some(id) = self.available_ids.pop() {
            self.generations[id] =
                self.generations[id].checked_add(1).expect("ID generation exhausted");
            self.list[id] = Some(element);
            id
        } else {
            let id = self.list.len();
            self.list.push(Some(element));
            // Generation zero is reserved for handles that were never
            // initialized. The first occupant of every slot is generation 1.
            self.generations.push(1);
            id
        }
    }

    pub fn get(&self, element_id: usize) -> Result<&T, Error> {
        self.list.get(element_id).ok_or(Error::IdNotActive)?.as_ref().ok_or(Error::IdNotActive)
    }

    pub fn get_mut(&mut self, element_id: usize) -> Result<&mut T, Error> {
        self.list.get_mut(element_id).ok_or(Error::IdNotActive)?.as_mut().ok_or(Error::IdNotActive)
    }

    /// Return the generation of the active occupant of `element_id`.
    ///
    /// A generation changes whenever a removed slot is reused, allowing
    /// long-lived handles to distinguish the new object from its predecessor.
    pub fn generation(&self, element_id: usize) -> Result<usize, Error> {
        self.get(element_id)?;
        self.generations.get(element_id).copied().ok_or(Error::IdNotActive)
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

    pub fn iter(&self) -> core::slice::Iter<'_, Option<T>> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Option<T>> {
        self.list.iter_mut()
    }
}

impl<T> Default for IdTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T> Send for IdTable<T> where T: Send {}
unsafe impl<T> Sync for IdTable<T> where T: Sync {}
