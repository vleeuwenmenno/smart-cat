//! `--pages` specs: `1`, `1-3`, `2,5-7`, `4-` and `all`.

use std::fmt;

/// A parsed page selection. Pages are 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageSpec {
    All,
    /// Inclusive ranges in the order given; `None` as the end means "to the last page".
    Ranges(Vec<(u32, Option<u32>)>),
}

impl PageSpec {
    pub fn parse(spec: &str) -> Result<PageSpec, String> {
        let spec = spec.trim();
        if spec.eq_ignore_ascii_case("all") {
            return Ok(PageSpec::All);
        }
        if spec.is_empty() {
            return Err("empty page list".into());
        }
        let mut ranges = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            let range = match part.split_once('-') {
                None => {
                    let page = page_number(part)?;
                    (page, Some(page))
                }
                Some((start, end)) => {
                    let start = page_number(start.trim())?;
                    let end = end.trim();
                    if end.is_empty() {
                        (start, None)
                    } else {
                        let end = page_number(end)?;
                        if end < start {
                            return Err(format!("'{part}' is a backwards range"));
                        }
                        (start, Some(end))
                    }
                }
            };
            ranges.push(range);
        }
        Ok(PageSpec::Ranges(ranges))
    }

    /// The selected pages that exist in a document of `count` pages, in order and without
    /// duplicates, and the first requested page beyond the end, if any.
    pub fn resolve(&self, count: u32) -> (Vec<u32>, Option<u32>) {
        let ranges = match self {
            PageSpec::All => vec![(1, Some(count))],
            PageSpec::Ranges(ranges) => ranges.clone(),
        };
        let mut pages: Vec<u32> = Vec::new();
        let mut missing = None;
        for (start, end) in ranges {
            let end = end.unwrap_or(count);
            if start > count {
                missing.get_or_insert(start);
                continue;
            }
            if end > count {
                missing.get_or_insert(count + 1);
            }
            for page in start..=end.min(count) {
                if !pages.contains(&page) {
                    pages.push(page);
                }
            }
        }
        (pages, missing)
    }
}

impl fmt::Display for PageSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageSpec::All => f.write_str("all"),
            PageSpec::Ranges(ranges) => {
                let parts: Vec<String> = ranges
                    .iter()
                    .map(|&(start, end)| match end {
                        Some(end) if end == start => start.to_string(),
                        Some(end) => format!("{start}-{end}"),
                        None => format!("{start}-"),
                    })
                    .collect();
                f.write_str(&parts.join(","))
            }
        }
    }
}

impl serde::Serialize for PageSpec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for PageSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        PageSpec::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl Default for PageSpec {
    fn default() -> Self {
        PageSpec::Ranges(vec![(1, Some(1))])
    }
}

fn page_number(text: &str) -> Result<u32, String> {
    match text.parse::<u32>() {
        Ok(0) => Err("pages are numbered from 1".into()),
        Ok(n) => Ok(n),
        Err(_) if text.is_empty() => Err("missing page number".into()),
        Err(_) => Err(format!("'{text}' is not a page number")),
    }
}

/// Compact form of a sorted page list: `[1,2,3,7]` becomes `1-3,7`.
pub fn compact(pages: &[u32]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(spec: &str, count: u32) -> Vec<u32> {
        PageSpec::parse(spec).unwrap().resolve(count).0
    }

    #[test]
    fn parses_single_pages_and_ranges() {
        assert_eq!(pages("1", 10), vec![1]);
        assert_eq!(pages("1-3", 10), vec![1, 2, 3]);
        assert_eq!(pages("2,5-7", 10), vec![2, 5, 6, 7]);
        assert_eq!(pages(" 3 , 1 ", 10), vec![3, 1]);
        assert_eq!(pages("8-", 10), vec![8, 9, 10]);
        assert_eq!(pages("all", 4), vec![1, 2, 3, 4]);
        assert_eq!(pages("ALL", 2), vec![1, 2]);
        assert_eq!(pages("2,1-3", 10), vec![2, 1, 3], "duplicates dropped");
    }

    #[test]
    fn rejects_invalid_specs() {
        for bad in [
            "", "0", "a", "-3", "3-1", "1,,2", "1.5", "1-2-3", "-", "0-2",
        ] {
            assert!(PageSpec::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn reports_pages_beyond_the_end() {
        let spec = PageSpec::parse("2,9-12").unwrap();
        assert_eq!(spec.resolve(10), (vec![2, 9, 10], Some(11)));
        let spec = PageSpec::parse("15").unwrap();
        assert_eq!(spec.resolve(10), (vec![], Some(15)));
    }

    #[test]
    fn displays_and_compacts() {
        assert_eq!(
            PageSpec::parse("1-3, 7,9-").unwrap().to_string(),
            "1-3,7,9-"
        );
        assert_eq!(compact(&[1, 2, 3, 7, 9, 10]), "1-3,7,9-10");
        assert_eq!(compact(&[4]), "4");
    }
}
