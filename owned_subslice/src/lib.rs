use std::{
    fmt::{self, Debug, Display},
    ops::{Deref, DerefMut, Index, IndexMut, Range},
};

/// An owning object (e.g. `String` or `Vec<u8>` and a range used to index it.
#[derive(Clone)]
pub struct OwnedSubslice<S>
where
    S: Index<Range<usize>>,
{
    data: S,
    range: Range<usize>,
}

impl<S> OwnedSubslice<S>
where
    S: Index<Range<usize>>,
{
    /// Use the data and the range to construct a new `OwnedSubslice`
    pub fn new(data: S, range: Range<usize>) -> Self {
        OwnedSubslice { data, range }
    }

    pub fn subslice(self, range: Range<usize>) -> Self {
        let start = self.range.start + range.start;
        let end = std::cmp::min(self.range.start + range.end, self.range.end);
        OwnedSubslice {
            data: self.data,
            range: Range { start, end },
        }
    }
}

impl<S> Deref for OwnedSubslice<S>
where
    S: Index<Range<usize>>,
{
    type Target = S::Output;

    fn deref(&self) -> &Self::Target {
        &self.data[self.range.clone()]
    }
}

impl<S> DerefMut for OwnedSubslice<S>
where
    S: IndexMut<Range<usize>>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data[self.range.clone()]
    }
}

impl From<String> for OwnedSubslice<String> {
    fn from(data: String) -> Self {
        let len = data.len();
        OwnedSubslice {
            data,
            range: 0..len,
        }
    }
}

impl<T> From<Vec<T>> for OwnedSubslice<Vec<T>> {
    fn from(data: Vec<T>) -> Self {
        let len = data.len();
        OwnedSubslice {
            data,
            range: 0..len,
        }
    }
}

impl<S> Debug for OwnedSubslice<S>
where
    S: Index<Range<usize>>,
    S::Output: Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(formatter)
    }
}

impl<S> Display for OwnedSubslice<S>
where
    S: Index<Range<usize>>,
    S::Output: Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice() {
        let string = String::from("Hello world!");
        let owned = OwnedSubslice::new(string, 1..4);
        assert_eq!(&*owned, "ell");
    }
}
