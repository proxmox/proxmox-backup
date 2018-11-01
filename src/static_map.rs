use std::borrow::Borrow;

#[derive(Debug)]
pub struct StaticMap<'a, K, V> {
    pub entries: &'a [(K,V)],
}

impl<'a, K, V> StaticMap<'a, K, V>
    where K: Eq {

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
        where K: Borrow<Q> + std::cmp::PartialEq<Q>,
              Q: Eq {
        for (ref k, ref v) in self.entries {
            if k == key { return Some(v) }
        }

        None
    }
}
