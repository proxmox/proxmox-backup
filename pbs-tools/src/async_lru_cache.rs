//! An 'async'-safe layer on the existing sync LruCache implementation. Supports multiple
//! concurrent requests to the same key.

use anyhow::Error;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::lru_cache::LruCache;
use proxmox_async::broadcast_future::BroadcastFuture;

/// Interface for asynchronously getting values on cache misses.
pub trait AsyncCacher<K, V: Clone>: Sync + Send {
    /// Fetch a value for key on cache miss.
    ///
    /// Works similar to non-async lru_cache::Cacher, but if the key has already been requested
    /// and the result is not cached yet, the 'fetch' function will not be called and instead the
    /// result of the original request cloned and returned upon completion.
    ///
    /// The underlying LRU structure is not modified until the returned future resolves to an
    /// Ok(Some(_)) value.
    fn fetch(&self, key: K) -> Box<dyn Future<Output = Result<Option<V>, Error>> + Send>;
}

/// See tools::lru_cache::LruCache, this implements an async-safe variant of that with the help of
/// AsyncCacher.
#[derive(Clone)]
pub struct AsyncLruCache<K, V> {
    #[allow(clippy::type_complexity)]
    maps: Arc<Mutex<(LruCache<K, V>, HashMap<K, BroadcastFuture<Option<V>>>)>>,
}

impl<K: std::cmp::Eq + std::hash::Hash + Copy, V: Clone + Send + 'static> AsyncLruCache<K, V> {
    /// Create a new AsyncLruCache with the given maximum capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            maps: Arc::new(Mutex::new((LruCache::new(capacity), HashMap::new()))),
        }
    }

    /// Access an item either via the cache or by calling cacher.fetch. A return value of Ok(None)
    /// means the item requested has no representation, Err(_) means a call to fetch() failed,
    /// regardless of whether it was initiated by this call or a previous one.
    pub async fn access(&self, key: K, cacher: &dyn AsyncCacher<K, V>) -> Result<Option<V>, Error> {
        let (owner, result_fut) = {
            // check if already requested
            let mut maps = self.maps.lock().unwrap();
            if let Some(fut) = maps.1.get(&key) {
                // wait for the already scheduled future to resolve
                (false, fut.listen())
            } else {
                // check if value is cached in LRU
                if let Some(val) = maps.0.get_mut(key) {
                    return Ok(Some(val.clone()));
                }

                // if neither, start broadcast future and put into map while we still have lock
                let fut = cacher.fetch(key);
                let broadcast = BroadcastFuture::new(fut);
                let result_fut = broadcast.listen();
                maps.1.insert(key, broadcast);
                (true, result_fut)
            }
            // drop Mutex before awaiting any future
        };

        let result = result_fut.await;

        if owner {
            // this call was the one initiating the request, put into LRU and remove from map
            let mut maps = self.maps.lock().unwrap();
            if let Ok(Some(ref value)) = result {
                maps.0.insert(key, value.clone());
            }
            maps.1.remove(&key);
        }

        result
    }
}

mod test {
    use super::*;

    struct TestAsyncCacher {
        prefix: &'static str,
    }

    impl AsyncCacher<i32, String> for TestAsyncCacher {
        fn fetch(
            &self,
            key: i32,
        ) -> Box<dyn Future<Output = Result<Option<String>, Error>> + Send> {
            let x = self.prefix;
            Box::new(async move { Ok(Some(format!("{}{}", x, key))) })
        }
    }

    #[test]
    fn test_async_lru_cache() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let cacher = TestAsyncCacher { prefix: "x" };
            let cache: AsyncLruCache<i32, String> = AsyncLruCache::new(2);

            assert_eq!(
                cache.access(10, &cacher).await.unwrap(),
                Some("x10".to_string())
            );
            assert_eq!(
                cache.access(20, &cacher).await.unwrap(),
                Some("x20".to_string())
            );
            assert_eq!(
                cache.access(30, &cacher).await.unwrap(),
                Some("x30".to_string())
            );

            for _ in 0..10 {
                let c = cache.clone();
                tokio::spawn(async move {
                    let cacher = TestAsyncCacher { prefix: "y" };
                    assert_eq!(
                        c.access(40, &cacher).await.unwrap(),
                        Some("y40".to_string())
                    );
                });
            }

            assert_eq!(
                cache.access(20, &cacher).await.unwrap(),
                Some("x20".to_string())
            );
        });
    }
}
