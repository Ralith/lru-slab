//! Pre-allocated storage with constant-time LRU tracking

#![warn(missing_docs)]
#![no_std]

extern crate alloc;

use alloc::boxed::Box;

/// A random-access table that maintains an LRU list in constant time
#[derive(Clone)]
pub struct LruSlab<T> {
    slots: Box<[Slot<T>]>,
    /// Most recently used
    head: u32,
    /// Least recently used
    tail: u32,
    /// First unused
    free: u32,
    /// Number of occupied slots
    len: u32,
}

impl<T> LruSlab<T> {
    /// Create an empty [`LruSlab`]
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Create an [`LruSlab`] that can store at least `capacity` elements without reallocating
    pub fn with_capacity(capacity: u32) -> Self {
        assert!(capacity != u32::max_value(), "capacity too large");
        Self {
            slots: (0..capacity)
                .map(|n| Slot {
                    value: None,
                    prev: NONE,
                    next: if n + 1 == capacity { NONE } else { n + 1 },
                })
                .collect(),
            head: NONE,
            tail: NONE,
            free: if capacity == 0 { NONE } else { 0 },
            len: 0,
        }
    }

    /// Whether no elements are stored
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of elements stored
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Number of elements that can be stored without reallocating
    pub fn capacity(&self) -> u32 {
        self.slots.len() as u32
    }

    /// Insert a value, returning the slot it was stored in
    ///
    /// The returned slot is marked as the most recently used.
    pub fn insert(&mut self, value: T) -> u32 {
        let id = match self.alloc() {
            Some(id) => id,
            None => {
                let len = self.capacity();
                let cap = 2 * len.max(2);
                self.slots = self
                    .slots
                    .iter_mut()
                    .map(|x| Slot {
                        value: x.value.take(),
                        next: x.next,
                        prev: x.prev,
                    })
                    .chain((len..cap).map(|n| Slot {
                        value: None,
                        prev: NONE,
                        next: if n + 1 == cap { NONE } else { n + 1 },
                    }))
                    .collect();
                self.free = len + 1;
                len
            }
        };
        let idx = id as usize;

        debug_assert!(self.slots[idx].value.is_none(), "corrupt free list");
        self.slots[idx].value = Some(value);
        self.link_at_head(id);
        self.len += 1;

        id
    }

    /// Get the least recently used slot, if any
    pub fn lru(&self) -> Option<u32> {
        if self.tail == NONE {
            debug_assert_eq!(self.head, NONE);
            None
        } else {
            Some(self.tail)
        }
    }

    /// Remove the element stored in `slot`, returning it
    pub fn remove(&mut self, slot: u32) -> T {
        self.unlink(slot);
        self.slots[slot as usize].next = self.free;
        self.slots[slot as usize].prev = NONE;
        self.free = slot;
        self.len -= 1;
        self.slots[slot as usize]
            .value
            .take()
            .expect("removing empty slot")
    }

    /// Mark `slot` as the most recently used and access it uniquely
    pub fn get_mut(&mut self, slot: u32) -> &mut T {
        self.freshen(slot);
        self.peek_mut(slot)
    }

    /// Access `slot` without marking it as most recently used
    pub fn peek(&self, slot: u32) -> &T {
        self.slots[slot as usize].value.as_ref().unwrap()
    }

    /// Access `slot` uniquely without marking it as most recently used
    pub fn peek_mut(&mut self, slot: u32) -> &mut T {
        self.slots[slot as usize].value.as_mut().unwrap()
    }

    /// Walk the container from most to least recently used
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            slots: &self.slots[..],
            head: self.head,
            tail: self.tail,
            len: self.len,
        }
    }

    /// Remove a slot from the freelist
    fn alloc(&mut self) -> Option<u32> {
        if self.free == NONE {
            return None;
        }
        let slot = self.free;
        self.free = self.slots[slot as usize].next;
        Some(slot)
    }

    /// Mark `slot` as the most recently used
    fn freshen(&mut self, slot: u32) {
        if self.slots[slot as usize].prev == NONE {
            // This is already the freshest slot, so we don't need to do anything
            debug_assert_eq!(self.head, slot, "corrupt lru list");
            return;
        }

        self.unlink(slot);
        self.link_at_head(slot);
    }

    /// Add a link to the head of the list
    fn link_at_head(&mut self, slot: u32) {
        let idx = slot as usize;
        if self.head == NONE {
            // List was empty
            self.slots[idx].next = NONE;
            self.tail = slot;
        } else {
            self.slots[idx].next = self.head;
            self.slots[self.head as usize].prev = slot;
        }
        self.slots[idx].prev = NONE;
        self.head = slot;
    }

    /// Remove a link from anywhere in the list
    fn unlink(&mut self, slot: u32) {
        let idx = slot as usize;
        if self.slots[idx].prev != NONE {
            self.slots[self.slots[idx].prev as usize].next = self.slots[idx].next;
        } else {
            self.head = self.slots[idx].next;
        }
        if self.slots[idx].next != NONE {
            self.slots[self.slots[idx].next as usize].prev = self.slots[idx].prev;
        } else {
            // This was the tail
            self.tail = self.slots[idx].prev;
        }
    }
}

impl<T> Default for LruSlab<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct Slot<T> {
    value: Option<T>,
    /// Next slot in the LRU or free list
    next: u32,
    /// Previous slot in the LRU list; NONE when free
    prev: u32,
}

const NONE: u32 = u32::MAX;

/// Iterator over elements of an [`LruSlab`], from most to least recently used
pub struct Iter<'a, T> {
    slots: &'a [Slot<T>],
    head: u32,
    tail: u32,
    len: u32,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            return None;
        }
        let idx = self.head as usize;
        let result = self.slots[idx].value.as_ref().expect("corrupt LRU list");
        self.head = self.slots[idx].next;
        self.len -= 1;
        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len as usize, Some(self.len as usize))
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T> {
    fn next_back(&mut self) -> Option<&'a T> {
        if self.len == 0 {
            return None;
        }
        let idx = self.tail as usize;
        let result = self.slots[idx].value.as_ref().expect("corrupt LRU list");
        self.tail = self.slots[idx].prev;
        self.len -= 1;
        Some(result)
    }
}

impl<T> ExactSizeIterator for Iter<'_, T> {
    fn len(&self) -> usize {
        self.len as usize
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;

    #[test]
    fn lru_order() {
        let mut cache = LruSlab::new();
        let b = cache.insert('b');
        assert_eq!(cache.iter().collect::<String>(), "b");
        let _a = cache.insert('a');
        assert_eq!(cache.iter().collect::<String>(), "ab");
        let d = cache.insert('d');
        assert_eq!(cache.iter().collect::<String>(), "dab");
        let c = cache.insert('c');
        assert_eq!(cache.iter().collect::<String>(), "cdab");
        let e = cache.insert('e');
        assert_eq!(cache.iter().collect::<String>(), "ecdab");

        cache.get_mut(b);
        cache.get_mut(c);
        cache.get_mut(d);
        cache.get_mut(e);

        assert_eq!(cache.remove(cache.lru().unwrap()), 'a');
        assert_eq!(cache.remove(cache.lru().unwrap()), 'b');
        assert_eq!(cache.remove(cache.lru().unwrap()), 'c');
        assert_eq!(cache.remove(cache.lru().unwrap()), 'd');
        assert_eq!(cache.remove(cache.lru().unwrap()), 'e');
        assert!(cache.lru().is_none());
    }

    #[test]
    fn slot_reuse() {
        let mut cache = LruSlab::new();
        let a = cache.insert('a');
        cache.remove(a);
        let a_prime = cache.insert('a');
        assert_eq!(a, a_prime);
        assert_eq!(cache.len(), 1);
    }
}