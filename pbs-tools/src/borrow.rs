/// This ties two values T and U together, such that T does not move and cannot be used as long as
/// there's an U. This essentially replaces the borrow checker's job for dependent values which
/// need to be stored together in a struct {}, and is similar to what the 'rental' crate produces.
pub struct Tied<T, U: ?Sized>(Option<Box<T>>, Option<Box<U>>);

impl<T, U: ?Sized> Drop for Tied<T, U> {
    fn drop(&mut self) {
        // let's be explicit about order here!
        std::mem::drop(self.1.take());
    }
}

impl<T, U: ?Sized> Tied<T, U> {
    /// Takes an owner and a function producing the depending value. The owner will be inaccessible
    /// until the tied value is resolved. The dependent value is only accessible by reference.
    pub fn new<F>(owner: T, producer: F) -> Self
    where
        F: FnOnce(*mut T) -> Box<U>,
    {
        let mut owner = Box::new(owner);
        let dep = producer(&mut *owner);
        Tied(Some(owner), Some(dep))
    }

    pub fn into_boxed_inner(mut self) -> Box<T> {
        self.1 = None;
        self.0.take().unwrap()
    }

    pub fn into_inner(self) -> T {
        *self.into_boxed_inner()
    }
}

impl<T, U: ?Sized> AsRef<U> for Tied<T, U> {
    fn as_ref(&self) -> &U {
        self.1.as_ref().unwrap()
    }
}

impl<T, U: ?Sized> AsMut<U> for Tied<T, U> {
    fn as_mut(&mut self) -> &mut U {
        self.1.as_mut().unwrap()
    }
}

impl<T, U: ?Sized> std::ops::Deref for Tied<T, U> {
    type Target = U;

    fn deref(&self) -> &U {
        self.as_ref()
    }
}

impl<T, U: ?Sized> std::ops::DerefMut for Tied<T, U> {
    fn deref_mut(&mut self) -> &mut U {
        self.as_mut()
    }
}
