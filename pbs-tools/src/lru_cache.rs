//! Least recently used (LRU) cache
//!
//! Implements a cache with least recently used cache replacement policy.
//! A HashMap is used for fast access by a given key and a doubly linked list
//! is used to keep track of the cache access order.

use std::collections::{hash_map::Entry, HashMap};
use std::marker::PhantomData;

/// Interface for getting values on cache misses.
pub trait Cacher<K, V> {
    /// Fetch a value for key on cache miss.
    ///
    /// Whenever a cache miss occurs, the fetch method provides a corresponding value.
    /// If no value can be obtained for the given key, None is returned, the cache is
    /// not updated in that case.
    fn fetch(&mut self, key: K) -> Result<Option<V>, anyhow::Error>;
}

/// Node of the doubly linked list storing key and value
struct CacheNode<K, V> {
    // We need to additionally store the key to be able to remove it
    // from the HashMap when removing the tail.
    key: K,
    value: V,
    prev: *mut CacheNode<K, V>,
    next: *mut CacheNode<K, V>,
    // Dropcheck marker. See the phantom-data section in the rustonomicon.
    _marker: PhantomData<Box<CacheNode<K, V>>>,
}

impl<K, V> CacheNode<K, V> {
    fn new(key: K, value: V) -> Self {
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
/// # use pbs_tools::lru_cache::{Cacher, LruCache};
/// # fn main() -> Result<(), anyhow::Error> {
/// struct LruCacher {};
///
/// impl Cacher<u64, u64> for LruCacher {
///     fn fetch(&mut self, key: u64) -> Result<Option<u64>, anyhow::Error> {
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
pub struct LruCache<K, V> {
    /// Quick access to individual nodes via the node pointer.
    map: HashMap<K, *mut CacheNode<K, V>>,
    /// Actual nodes stored in a linked list.
    list: LinkedList<K, V>,
    /// Max nodes the cache can hold, temporarily exceeded by 1 due to
    /// implementation details.
    capacity: usize,
    // Dropcheck marker. See the phantom-data section in the rustonomicon.
    _marker: PhantomData<Box<CacheNode<K, V>>>,
}

impl<K, V> Drop for LruCache<K, V> {
    fn drop(&mut self) {
        self.clear();
    }
}

// trivial: if our contents are Send, the whole cache is Send
unsafe impl<K: Send, V: Send> Send for LruCache<K, V> {}

impl<K, V> LruCache<K, V> {
    /// Clear all the entries from the cache.
    pub fn clear(&mut self) {
        // This frees only the HashMap with the node pointers.
        self.map.clear();
        // This frees the actual nodes and resets the list head and tail.
        self.list.clear();
    }
}

impl<K: std::cmp::Eq + std::hash::Hash + Copy, V> LruCache<K, V> {
    /// Create LRU cache instance which holds up to `capacity` nodes at once.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            map: HashMap::with_capacity(capacity),
            list: LinkedList::new(),
            capacity,
            _marker: PhantomData,
        }
    }

    /// Insert or update an entry identified by `key` with the given `value`.
    /// This entry is placed as the most recently used node at the head.
    pub fn insert(&mut self, key: K, value: V) {
        match self.map.entry(key) {
            Entry::Occupied(mut o) => {
                // Node present, update value
                let node_ptr = *o.get_mut();
                self.list.bring_to_front(node_ptr);
                let mut node = unsafe { Box::from_raw(node_ptr) };
                node.value = value;
                let _node_ptr = Box::into_raw(node);
            }
            Entry::Vacant(v) => {
                // Node not present, insert a new one
                // Unfortunately we need a copy of the key here, therefore it has
                // to impl the copy trait
                let node = Box::new(CacheNode::new(key, value));
                let node_ptr = Box::into_raw(node);
                self.list.push_front(node_ptr);
                v.insert(node_ptr);
                // If we have more elements than capacity,
                // delete the lists tail node (= oldest node).
                // This needs to be executed after the insert in order to
                // avoid borrow conflict. This means there are temporarily
                // self.capacity + 1 cache nodes.
                if self.map.len() > self.capacity {
                    self.pop_tail();
                }
            }
        }
    }

    /// Remove the given `key` and its `value` from the cache.
    pub fn remove(&mut self, key: K) -> Option<V> {
        // Remove node pointer from the HashMap and get ownership of the node
        let node_ptr = self.map.remove(&key)?;
        let node = self.list.remove(node_ptr);
        Some(node.value)
    }

    /// Remove the least recently used node from the cache.
    fn pop_tail(&mut self) {
        if let Some(old_tail) = self.list.pop_tail() {
            // Remove HashMap entry for old tail
            self.map.remove(&old_tail.key);
        }
    }

    /// Get a mutable reference to the value identified by `key`.
    /// This will update the cache entry to be the most recently used entry.
    /// On cache misses, None is returned.
    pub fn get_mut(&mut self, key: K) -> Option<&mut V> {
        let node_ptr = self.map.get(&key)?;
        self.list.bring_to_front(*node_ptr);
        Some(unsafe { &mut (*self.list.head).value })
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` when the cache is empty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Get a mutable reference to the value identified by `key`.
    /// This will update the cache entry to be the most recently used entry.
    /// On cache misses, the cachers fetch method is called to get a corresponding
    /// value.
    /// If fetch returns a value, it is inserted as the most recently used entry
    /// in the cache.
    pub fn access<'a>(
        &'a mut self,
        key: K,
        cacher: &mut dyn Cacher<K, V>,
    ) -> Result<Option<&'a mut V>, anyhow::Error> {
        match self.map.entry(key) {
            Entry::Occupied(mut o) => {
                // Cache hit, birng node to front of list
                let node_ptr = *o.get_mut();
                self.list.bring_to_front(node_ptr);
            }
            Entry::Vacant(v) => {
                // Cache miss, try to fetch from cacher and insert at the front
                match cacher.fetch(key)? {
                    None => return Ok(None),
                    Some(value) => {
                        // Unfortunately we need a copy of the key here, therefore it has
                        // to impl the copy trait
                        let node = Box::new(CacheNode::new(key, value));
                        let node_ptr = Box::into_raw(node);
                        self.list.push_front(node_ptr);
                        v.insert(node_ptr);
                        // If we have more elements than capacity,
                        // delete the lists tail node (= oldest node).
                        // This needs to be executed after the insert in order to
                        // avoid borrow conflict. This means there are temporarily
                        // self.capacity + 1 cache nodes.
                        if self.map.len() > self.capacity {
                            self.pop_tail();
                        }
                    }
                }
            }
        }

        Ok(Some(unsafe { &mut (*self.list.head).value }))
    }
}

/// Linked list holding the nodes of the LruCache.
///
/// This struct actually holds the CacheNodes via the raw linked list pointers
/// and allows to define the access sequence of these via the list sequence.
/// The LinkedList of the standard library unfortunately does not implement
/// an efficient way to bring list entries to the front, therefore we need our own.
struct LinkedList<K, V> {
    head: *mut CacheNode<K, V>,
    tail: *mut CacheNode<K, V>,
}

impl<K, V> LinkedList<K, V> {
    /// Create a new empty linked list.
    fn new() -> Self {
        Self {
            head: std::ptr::null_mut(),
            tail: std::ptr::null_mut(),
        }
    }

    /// Bring the CacheNode referenced by `node_ptr` to the front of the linked list.
    fn bring_to_front(&mut self, node_ptr: *mut CacheNode<K, V>) {
        if node_ptr == self.head {
            // node is already head, just return
            return;
        }

        let mut node = unsafe { Box::from_raw(node_ptr) };
        // Update the prev node to point to next (or null if current node is tail)
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
    }

    /// Insert a new node at the front of the linked list.
    fn push_front(&mut self, node_ptr: *mut CacheNode<K, V>) {
        let mut node = unsafe { Box::from_raw(node_ptr) };

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
    }

    /// Remove the node referenced by `node_ptr` from the linked list and return it.
    fn remove(&mut self, node_ptr: *mut CacheNode<K, V>) -> Box<CacheNode<K, V>> {
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
        node
    }

    /// Remove the tail node from the linked list and return it.
    fn pop_tail(&mut self) -> Option<Box<CacheNode<K, V>>> {
        if self.tail.is_null() {
            return None;
        }

        let old_tail = unsafe { Box::from_raw(self.tail) };
        self.tail = old_tail.prev;
        // Update next node for new tail
        if !self.tail.is_null() {
            unsafe { (*self.tail).next = std::ptr::null_mut() };
        }
        Some(old_tail)
    }

    /// Clear the linked list and free all the nodes.
    fn clear(&mut self) {
        let mut next = self.head;
        while !next.is_null() {
            // Taking ownership of node and drop it at the end of the block.
            let current = unsafe { Box::from_raw(next) };
            next = current.next;
        }
        // Reset head and tail pointers
        self.head = std::ptr::null_mut();
        self.tail = std::ptr::null_mut();
    }
}

#[test]
fn test_linked_list() {
    let mut list = LinkedList::new();
    for idx in 0..3 {
        let node = Box::new(CacheNode::new(idx, idx + 1));
        // Get pointer, release ownership.
        let node_ptr = Box::into_raw(node);
        list.push_front(node_ptr);
    }
    assert_eq!(unsafe { (*list.head).key }, 2);
    assert_eq!(unsafe { (*list.head).value }, 3);
    assert_eq!(unsafe { (*list.tail).key }, 0);
    assert_eq!(unsafe { (*list.tail).value }, 1);

    list.bring_to_front(list.tail);
    assert_eq!(unsafe { (*list.head).key }, 0);
    assert_eq!(unsafe { (*list.head).value }, 1);
    assert_eq!(unsafe { (*list.tail).key }, 1);
    assert_eq!(unsafe { (*list.tail).value }, 2);

    list.bring_to_front(list.tail);
    assert_eq!(unsafe { (*list.head).key }, 1);
    assert_eq!(unsafe { (*list.head).value }, 2);
    assert_eq!(unsafe { (*list.tail).key }, 2);
    assert_eq!(unsafe { (*list.tail).value }, 3);

    let tail = list.pop_tail().unwrap();
    assert_eq!(tail.key, 2);
    assert_eq!(tail.value, 3);
    assert_eq!(unsafe { (*list.head).key }, 1);
    assert_eq!(unsafe { (*list.head).value }, 2);
    assert_eq!(unsafe { (*list.tail).key }, 0);
    assert_eq!(unsafe { (*list.tail).value }, 1);

    list.clear();
    assert!(list.head.is_null());
    assert!(list.tail.is_null());
}
