//! Version string ordering, ported from `Version.cpp`.

use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Null,
    Textual,
    Numeric,
    PreRelease,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Section {
    kind: Kind,
    value: Vec<char>,
}

static NULL: Section = Section { kind: Kind::Null, value: Vec::new() };

impl Section {
    fn compare(&self, other: &Section) -> Ordering {
        match (self.kind, other.kind) {
            (Kind::Numeric, Kind::Numeric) => {
                self.value.len().cmp(&other.value.len()).then_with(|| self.value.cmp(&other.value))
            }
            (Kind::Null, Kind::PreRelease) => Ordering::Greater,
            (Kind::Null, _) => Ordering::Less,
            (Kind::PreRelease, Kind::Null) => Ordering::Less,
            (_, Kind::Null) => Ordering::Greater,
            // Vec<char> compares element-wise and then by length, like the original loop.
            _ => self.value.cmp(&other.value),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Version {
    sections: Vec<Section>,
}

impl Version {
    pub fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let mut sections = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let mut cur = Section { kind: Kind::Textual, value: Vec::new() };
            let c = chars[i];
            if c == '+' {
                break; // build metadata is ignored
            }
            if c == '-' || c == ' ' {
                cur.value.push(c);
                i += 1;
                if i < chars.len() && !chars[i].is_ascii_digit() {
                    cur.kind = Kind::PreRelease;
                }
            } else if c.is_ascii_digit() {
                cur.kind = Kind::Numeric;
            }
            while i < chars.len() {
                let r = chars[i];
                let numeric = cur.kind == Kind::Numeric;
                if r.is_ascii_digit() != numeric
                    || (r == ' ' && numeric)
                    || (r == '-' && cur.kind != Kind::PreRelease)
                    || r == '+'
                {
                    break;
                }
                cur.value.push(r);
                i += 1;
            }
            if !cur.value.is_empty() {
                if cur.kind == Kind::Numeric {
                    let zeros = cur.value.iter().take_while(|c| **c == '0').count();
                    cur.value.drain(..zeros);
                }
                sections.push(cur);
            }
        }
        Version { sections }
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.sections.len().max(other.sections.len());
        (0..len)
            .map(|i| {
                let a = self.sections.get(i).unwrap_or(&NULL);
                let b = other.sections.get(i).unwrap_or(&NULL);
                a.compare(b)
            })
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn orders_like_the_original() {
        let ascending = ["1.0-alpha", "1.0", "1.0.1", "1.2", "1.10", "1.10.2", "3.3.3", "3.4.3", "26.3"];
        for pair in ascending.windows(2) {
            assert!(Version::new(pair[0]) < Version::new(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(Version::new("1.2.0+build5"), Version::new("1.2.0+build7"));
        assert_eq!(Version::new("1.02"), Version::new("1.2"));
        assert!(Version::new("1.20.1-pre1") < Version::new("1.20.1"));
    }
}
