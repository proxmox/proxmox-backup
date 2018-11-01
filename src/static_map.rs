#[derive(Debug)]
pub struct StaticMap<'a, K, V> {
    pub entries: &'a [(K,V)],
}

impl<'a, K: Eq, V> StaticMap<'a, K, V> {

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        for (ref k, ref v) in self.entries {
            if k == key { return Some(v) }
        }
        None
    }
}
