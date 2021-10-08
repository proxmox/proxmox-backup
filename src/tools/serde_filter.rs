use std::marker::PhantomData;

use serde::Deserialize;

/// Helper to filter data while deserializing it.
///
/// An example use case is filtering out expired registration challenges at load time of our TFA
/// config:
///
/// ```
/// # use proxmox_backup::tools::serde_filter::FilteredVecVisitor;
/// # use serde::{Deserialize, Deserializer, Serialize};
/// # const CHALLENGE_TIMEOUT: i64 = 2 * 60;
/// #[derive(Deserialize)]
/// struct Challenge {
///     /// Expiration time as unix epoch.
///     expires: i64,
///
///     // ...other entries...
/// }
///
/// #[derive(Default, Deserialize)]
/// #[serde(deny_unknown_fields)]
/// #[serde(rename_all = "kebab-case")]
/// pub struct TfaUserData {
///     // ...other entries...
///
///     #[serde(skip_serializing_if = "Vec::is_empty", default)]
///     #[serde(deserialize_with = "filter_expired_registrations")]
///     registrations: Vec<Challenge>,
/// }
///
/// fn filter_expired_registrations<'de, D>(deserializer: D) -> Result<Vec<Challenge>, D::Error>
/// where
///     D: Deserializer<'de>,
/// {
///     let expire_before = proxmox_time::epoch_i64() - CHALLENGE_TIMEOUT;
///
///     Ok(deserializer.deserialize_seq(
///         FilteredVecVisitor::new(
///             "a u2f registration challenge entry",
///             move |c: &Challenge| c.expires < expire_before,
///         )
///     )?)
/// }
/// ```
pub struct FilteredVecVisitor<F, T>
where
    F: Fn(&T) -> bool
{
    filter: F,
    expecting: &'static str,
    _ty: PhantomData<T>,
}

impl<F, T> FilteredVecVisitor<F, T>
where
    F: Fn(&T) -> bool,
{
    pub fn new(expecting: &'static str, filter: F) -> Self {
        Self {
            filter,
            expecting,
            _ty: PhantomData,
        }
    }
}

impl<'de, F, T> serde::de::Visitor<'de> for FilteredVecVisitor<F, T>
where
    F: Fn(&T) -> bool,
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(self.expecting)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut out = match seq.size_hint() {
            Some(hint) => Vec::with_capacity(hint),
            None => Vec::new(),
        };

        while let Some(entry) = seq.next_element::<T>()? {
            if (self.filter)(&entry) {
                out.push(entry);
            }
        }

        Ok(out)
    }
}
