//! Defines the `Repetition` enum for specifying pattern repetition in filter matching.

/// Defines how many times a pattern must occur for a match.
/// Corresponds to quantifier syntax in the filter DSL.
///
/// | Variant         | DSL Syntax  | Meaning         |
/// |-----------------|-------------|-----------------|
/// | `Unit`          | (none)      | Exactly once    |
/// | `Optional`      | `?`         | 0 or 1 times    |
/// | `OneOrMore`     | `+`         | 1 or more times |
/// | `ZeroOrMore`    | `*`         | 0 or more times |
/// | `NTimes(n)`     | `{n}`       | Exactly n times |
/// | `AtLeast(n)`    | `{n,}`      | n or more times |
/// | `AtMost(n)`     | `{,n}`      | 0 to n times    |
/// | `Interval(n,m)` | `{n,m}`     | n to m times    |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repetition {
    /// Exactly once
    Unit, // '' Pass-through, no repetition = {1,1}
    /// '?' = {0,1}
    Optional,
    /// '+' = {1,}
    OneOrMore,
    /// '*' = {0,}
    ZeroOrMore,
    /// '{n}' = {n,n}
    NTimes(usize),
    ///'{n,}': + = {1,}, * = {0,}
    AtLeast(usize),
    /// '{,m}' = {0,m}
    AtMost(usize),
    /// '{n,m}': ? = {0,1}
    Interval(usize, usize),
}

impl Repetition {
    /// Returns the minimum number of occurrences required for a match.
    pub fn min(&self) -> usize {
        match self {
            Self::Unit => 1,
            Self::Optional => 0,
            Self::OneOrMore => 1,
            Self::ZeroOrMore => 0,
            Self::NTimes(n) => *n,
            Self::AtLeast(n) => *n,
            Self::AtMost(_) => 0,
            Self::Interval(n, _) => *n,
        }
    }
    /// Returns the maximum number of occurrences allowed for a match, if bounded.
    pub fn max(&self) -> Option<usize> {
        match self {
            Self::Unit => Some(1),
            Self::Optional => Some(1),
            Self::OneOrMore => None,
            Self::ZeroOrMore => None,
            Self::NTimes(n) => Some(*n),
            Self::AtLeast(_) => None,
            Self::AtMost(m) => Some(*m),
            Self::Interval(_, m) => Some(*m),
        }
    }
    /// Checks if a given count of occurrences satisfies this repetition constraint.
    pub fn check(&self, count: usize) -> bool {
        let min = self.min();
        let max = self.max();
        count >= min && max.is_none_or(|max| count <= max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt;

    #[test]
    fn test_repetition_unit() {
        let rep = Repetition::Unit;
        assert_eq!(rep.min(), 1);
        assert_eq!(rep.max(), Some(1));
        assert!(!rep.check(0));
        assert!(rep.check(1));
        assert!(!rep.check(2));
    }

    #[test]
    fn test_repetition_optional() {
        let rep = Repetition::Optional;
        assert_eq!(rep.min(), 0);
        assert_eq!(rep.max(), Some(1));
        assert!(rep.check(0));
        assert!(rep.check(1));
        assert!(!rep.check(2));
    }

    #[test]
    fn test_repetition_one_or_more() {
        let rep = Repetition::OneOrMore;
        assert_eq!(rep.min(), 1);
        assert_eq!(rep.max(), None);
        assert!(!rep.check(0));
        assert!(rep.check(1));
        assert!(rep.check(5));
    }

    #[test]
    fn test_repetition_zero_or_more() {
        let rep = Repetition::ZeroOrMore;
        assert_eq!(rep.min(), 0);
        assert_eq!(rep.max(), None);
        assert!(rep.check(0));
        assert!(rep.check(1));
        assert!(rep.check(100));
    }

    #[test]
    fn test_repetition_n_times() {
        let n = (rand::rng().random::<u32>() % 100 + 1) as usize; // Random n between 1 and 100
        let rep = Repetition::NTimes(n);
        assert_eq!(rep.min(), n);
        assert_eq!(rep.max(), Some(n));
        assert!(!rep.check(0));
        assert!(!rep.check(n - 1));
        assert!(rep.check(n));
        assert!(!rep.check(n + 1));
    }

    #[test]
    fn test_repetition_at_least() {
        let n = (rand::rng().random::<u32>() % 100 + 1) as usize; // Random n between 1 and 100
        let rep = Repetition::AtLeast(n);
        assert_eq!(rep.min(), n);
        assert_eq!(rep.max(), None);
        assert!(!rep.check(0));
        assert!(!rep.check(n - 1));
        assert!(rep.check(n));
        assert!(rep.check(n + 100));
    }

    #[test]
    fn test_repetition_at_most() {
        let n = (rand::rng().random::<u32>() % 100 + 5) as usize; // Random n between 5 and 104
        let rep = Repetition::AtMost(n);
        assert_eq!(rep.min(), 0);
        assert_eq!(rep.max(), Some(n));
        assert!(rep.check(0));
        assert!(rep.check(3));
        assert!(rep.check(n));
        assert!(!rep.check(n + 1));
    }

    #[test]
    fn test_repetition_interval() {
        let rep = Repetition::Interval(2, 4);
        assert_eq!(rep.min(), 2);
        assert_eq!(rep.max(), Some(4));
        assert!(!rep.check(0));
        assert!(!rep.check(1));
        assert!(rep.check(2));
        assert!(rep.check(3));
        assert!(rep.check(4));
        assert!(!rep.check(5));
    }
}
