//! Least recently used (LRU) cache
//!
//! Implements a cache with least recently used cache replacement policy.
//! A HashMap is used for fast access by a given key and a doubly linked list
//! is used to keep track of the cache access order.

use std::collections::HashMap;
use std::marker::PhantomData;

/// Interface for getting values on cache misses.
pub trait Cacher<V> {
    /// Fetch a value for key on cache miss.
    ///
    /// Whenever a cache miss occurs, the fetch method provides a corresponding value.
    /// If no value can be obtained for the given key, None is returned, the cache is
    /// not updated in that case.
    fn fetch(&mut self, key: u64) -> Result<Option<V>, failure::Error>;
}

/// Node of the doubly linked list storing key and value
struct CacheNode<V> {
    // We need to additionally store the key to be able to remove it
    // from the HashMap when removing the tail.
    key: u64,
    value: V,
    prev: *mut CacheNode<V>,
    next: *mut CacheNode<V>,
    // Dropcheck marker. See the phantom-data section in the rustonomicon.
    _marker: PhantomData<Box<CacheNode<V>>>,
}

impl<V> CacheNode<V> {
    fn new(key: u64, value: V) -> Self {
        Self {
            key,
            value,
            prev: std::ptr::null_mut(),
            next: std::ptr::null_mut(),
            _marker: PhantomData,
        }
    }
}

/// LRU cache instance.
///
/// # Examples:
/// ```
/// # use self::proxmox_backup::tools::lru_cache::{Cacher, LruCache};
/// # fn main() -> Result<(), failure::Error> {
/// struct LruCacher {};
///
/// impl Cacher<u64> for LruCacher {
///     fn fetch(&mut self, key: u64) -> Result<Option<u64>, failure::Error> {
///         Ok(Some(key))
///     }
/// }
///
/// let mut cache = LruCache::new(3);
///
/// assert_eq!(cache.get_mut(1), None);
/// assert_eq!(cache.len(), 0);
///
/// cache.insert(1, 1);
/// cache.insert(2, 2);
/// cache.insert(3, 3);
/// cache.insert(4, 4);
/// assert_eq!(cache.len(), 3);
///
/// assert_eq!(cache.get_mut(1), None);
/// assert_eq!(cache.get_mut(2), Some(&mut 2));
/// assert_eq!(cache.get_mut(3), Some(&mut 3));
/// assert_eq!(cache.get_mut(4), Some(&mut 4));
///
/// cache.remove(4);
/// cache.remove(3);
/// cache.remove(2);
/// assert_eq!(cache.len(), 0);
/// assert_eq!(cache.get_mut(2), None);
/// // access will fill in missing cache entry by fetching from LruCacher
/// assert_eq!(cache.access(2, &mut LruCacher {}).unwrap(), Some(&mut 2));
///
/// cache.insert(1, 1);
/// assert_eq!(cache.get_mut(1), Some(&mut 1));
///
/// cache.clear();
/// assert_eq!(cache.len(), 0);
/// assert_eq!(cache.get_mut(1), None);
/// # Ok(())
/// # }
/// ```
pub struct LruCache<V> {
    map: HashMap<u64, *mut CacheNode<V>>,
    head: *mut CacheNode<V>,
    tail: *mut CacheNode<V>,
    capacity: usize,
    // Dropcheck marker. See the phantom-data section in the rustonomicon.
    _marker: PhantomData<Box<CacheNode<V>>>,
}

impl<V> LruCache<V> {
    /// Create LRU cache instance which holds up to `capacity` nodes at once.
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            head: std::ptr::null_mut(),
            tail: std::ptr::null_mut(),
            capacity,
            _marker: PhantomData,
        }
    }

    /// Clear all the entries from the cache.
    pub fn clear(&mut self) {
        // Dump all heap allocations, then dump all the pointers in the HashMap
        for node_ptr in self.map.values() {
            unsafe { Box::from_raw(*node_ptr) };
        }
        self.map.clear();
        // Reset head and tail pointers
        self.head = std::ptr::null_mut();
        self.tail = std::ptr::null_mut();
    }

    /// Insert or update an entry identified by `key` with the given `value`.
    /// This entry is placed as the most recently used node at the head.
    pub fn insert(&mut self, key: u64, value: V) {
        match self.get_mut(key) {
            // Key already exists and get_mut brings node to the front, so only update its value.
            Some(old_val) => *old_val = value,
            None => {
                // If we have more elements than capacity, delete the tail entry
                // (= oldest entry).
                if self.map.len() >= self.capacity {
                    self.remove_tail();
                }
                self.insert_front(key, value);
            }
        }
    }

    /// Insert a key, value pair at the front of the linked list and it's pointer
    /// into the HashMap.
    fn insert_front(&mut self, key: u64, value: V) {
        // First create heap allocated `CacheNode` containing value.
        let mut node = Box::new(CacheNode::new(key, value));
        // Old head gets new heads next
        node.next = self.head;
        // Release ownership of node, rest can be handled with just the pointer.
        let node_ptr = Box::into_raw(node);

        // Update the prev for the old head
        if !self.head.is_null() {
            unsafe { (*self.head).prev = node_ptr };
        }

        // Update the head to the new node pointer
        self.head = node_ptr;

        // If there was no old tail, this node will be the new tail too
        if self.tail.is_null() {
            self.tail = node_ptr;
        }
        // finally insert the node pointer into the HashMap
        self.map.insert(key, node_ptr);
    }

    /// Remove the given `key` and its `value` from the cache.
    pub fn remove(&mut self, key: u64) -> Option<V> {
        // Remove node pointer from the HashMap and get ownership of the node
        let node_ptr = self.map.remove(&key)?;
        let node = unsafe { Box::from_raw(node_ptr) };

        // Update the previous node or otherwise the head
        if !node.prev.is_null() {
            unsafe { (*node.prev).next = node.next };
        } else {
            // No previous node means this was the head
            self.head = node.next;
        }

        // Update the next node or otherwise the tail
        if !node.next.is_null() {
            unsafe { (*node.next).prev = node.prev };
        } else {
            // No next node means this was the tail
            self.tail = node.prev;
        }

        Some(node.value)
    }

    /// Remove the least recently used node from the cache.
    fn remove_tail(&mut self) {
        if self.tail.is_null() {
            panic!("Called remove_tail on empty tail pointer!");
        }

        let old_tail = unsafe { Box::from_raw(self.tail) };
        self.tail = old_tail.prev;
        // Update next node for new tail
        if !self.tail.is_null() {
            unsafe { (*self.tail).next = std::ptr::null_mut() };
        }

        // Remove HashMap entry for old tail
        self.map.remove(&old_tail.key);
    }

    /// Get a mutable reference to the value identified by `key`.
    /// This will update the cache entry to be the most recently used entry.
    /// On cache misses, None is returned.
    pub fn get_mut<'a>(&'a mut self, key: u64) -> Option<&'a mut V> {
        let node_ptr = self.map.get(&key)?;
        if *node_ptr == self.head {
            // node is already head, just return
            return Some(unsafe { &mut (*self.head).value });
        }

        // Update the prev node to point to next (or null if current node is tail)
        let mut node = unsafe { Box::from_raw(*node_ptr) };
        unsafe { (*node.prev).next = node.next };

        // Update the next node or otherwise the tail
        if !node.next.is_null() {
            unsafe { (*node.next).prev = node.prev };
        } else {
            // No next node means this was the tail
            self.tail = node.prev;
        }

        node.prev = std::ptr::null_mut();
        node.next = self.head;
        // update the head and release ownership of the node again
        let node_ptr = Box::into_raw(node);
        // Update current head
        unsafe { (*self.head).prev = node_ptr };
        // Update to new head
        self.head = node_ptr;

        Some(unsafe { &mut (*self.head).value })
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Get a mutable reference to the value identified by `key`.
    /// This will update the cache entry to be the most recently used entry.
    /// On cache misses, the cachers fetch method is called to get a corresponding
    /// value.
    /// If fetch returns a value, it is inserted as the most recently used entry
    /// in the cache.
    pub fn access<'a>(&'a mut self, key: u64, cacher: &mut dyn Cacher<V>) -> Result<Option<&'a mut V>, failure::Error> {
        if self.get_mut(key).is_some() {
            // get_mut brings the node to the front if present, so just return
            return Ok(Some(unsafe { &mut (*self.head).value }));
        }

        // Cache miss, try to fetch from cacher
        match cacher.fetch(key)? {
            None => Ok(None),
            Some(value) => {
                // If we have more elements than capacity, delete the tail entry
                // (= oldest entry).
                if self.map.len() >= self.capacity {
                    self.remove_tail();
                }
                self.insert_front(key, value);
                Ok(Some(unsafe { &mut (*self.head).value }))
            }
        }
    }
}
