//! ID maps of a rootless domain (docs/plans/PODMAN-PLUGIN.md, "Named
//! Volumes", "Namespace und Start").
//!
//! # Purpose
//!
//! A rootless account owns its uid plus the sub-ID ranges of
//! `/etc/subuid`/`/etc/subgid`. The broker's namespace maps ns 0 to the
//! account and ns 1.. to the first sub-ID range (`0 <uid> 1`,
//! `1 <subuid> <count>`). The host reads files with their *host* IDs; this
//! module tells whether a host ID belongs to the account (otherwise the
//! file is rejected) and translates it to the namespace view for
//! `ownership.tsv` and the manifest.
//!
//! # Data flow
//!
//! `/etc/subuid` text → [`sub_ranges`] → [`IdMap::rootless`] → used by
//! `sanitize::classify` ([`IdMap::to_ns`], [`IdMap::contains`]) and written
//! as `uid_map` into `/proc/<pid>/uid_map` by `ofd` ([`IdMap::proc_map`]).
//!
//! # Counterpart
//!
//! The kernel's own mapping in the broker's namespace; the restore side
//! translates back with [`IdMap::to_host`].

/// One line of a uid_map: `count` IDs from `ns_start` map to `host_start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Range {
    /// First ID inside the namespace.
    pub ns_start: u32,
    /// First ID on the host.
    pub host_start: u32,
    /// Number of IDs.
    pub count: u32,
}

/// A namespace map: a list of non-overlapping ranges.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct IdMap {
    /// The ranges, in the order written to uid_map.
    pub ranges: Vec<Range>,
}

/// Most sub-ID ranges used for one map (the kernel allows 340 lines; the
/// plan uses one range after the account's own ID).
const MAX_RANGES: usize = 8;

impl IdMap {
    /// The map of a rootless account: ns 0 → `id`, then the sub ranges
    /// back to back from ns 1. Fails when ranges overlap the account or
    /// each other, are empty or overflow.
    pub fn rootless(id: u32, sub: &[(u32, u32)]) -> Result<Self, String> {
        if sub.len() > MAX_RANGES {
            return Err("too many sub-ID ranges".into());
        }
        let mut ranges = vec![Range { ns_start: 0, host_start: id, count: 1 }];
        let mut next_ns: u32 = 1;
        for &(start, count) in sub {
            if count == 0 {
                return Err("empty sub-ID range".into());
            }
            start.checked_add(count).ok_or("sub-ID range overflows")?;
            let ns_end = next_ns.checked_add(count).ok_or("namespace range overflows")?;
            let candidate = Range { ns_start: next_ns, host_start: start, count };
            if ranges.iter().any(|r| overlaps_host(r, &candidate)) {
                return Err("sub-ID ranges overlap".into());
            }
            ranges.push(candidate);
            next_ns = ns_end;
        }
        Ok(Self { ranges })
    }

    /// The identity map of the rootful profile: host IDs are namespace IDs.
    pub fn identity() -> Self {
        Self { ranges: vec![Range { ns_start: 0, host_start: 0, count: u32::MAX }] }
    }

    /// Whether host ID `id` belongs to this map.
    pub fn contains(&self, id: u32) -> bool {
        self.to_ns(id).is_some()
    }

    /// Host ID → namespace ID.
    pub fn to_ns(&self, id: u32) -> Option<u32> {
        self.ranges.iter().find_map(|r| {
            let off = id.checked_sub(r.host_start)?;
            (off < r.count).then(|| r.ns_start + off)
        })
    }

    /// Namespace ID → host ID.
    pub fn to_host(&self, ns: u32) -> Option<u32> {
        self.ranges.iter().find_map(|r| {
            let off = ns.checked_sub(r.ns_start)?;
            (off < r.count).then(|| r.host_start + off)
        })
    }

    /// The text for `/proc/<pid>/uid_map` or `gid_map`.
    pub fn proc_map(&self) -> String {
        self.ranges.iter().map(|r| format!("{} {} {}\n", r.ns_start, r.host_start, r.count)).collect()
    }

    /// Compact form for ownership.tsv and the manifest: `ns:host:count,…`.
    pub fn compact(&self) -> String {
        self.ranges.iter().map(|r| format!("{}:{}:{}", r.ns_start, r.host_start, r.count)).collect::<Vec<_>>().join(",")
    }
}

/// Whether two ranges share a host ID.
fn overlaps_host(a: &Range, b: &Range) -> bool {
    let (a0, a1) = (u64::from(a.host_start), u64::from(a.host_start) + u64::from(a.count));
    let (b0, b1) = (u64::from(b.host_start), u64::from(b.host_start) + u64::from(b.count));
    a0 < b1 && b0 < a1
}

/// The ranges of `/etc/subuid` (or subgid) text for an account given by
/// name or numeric ID (both forms occur). Comments and malformed lines are
/// skipped; a line naming the account with bad numbers is an error.
pub fn sub_ranges(text: &str, name: &str, id: u32) -> Result<Vec<(u32, u32)>, String> {
    let id_text = id.to_string();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(':');
        let (Some(who), Some(start), Some(count), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if who != name && who != id_text {
            continue;
        }
        let start: u32 = start.parse().map_err(|_| format!("bad sub-ID start for {name}"))?;
        let count: u32 = count.parse().map_err(|_| format!("bad sub-ID count for {name}"))?;
        out.push((start, count));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootless_map_translates_both_ways() {
        let m = IdMap::rootless(1000, &[(100000, 65536)]).unwrap();
        assert_eq!(m.to_ns(1000), Some(0));
        assert_eq!(m.to_ns(100000), Some(1));
        assert_eq!(m.to_ns(165535), Some(65536));
        assert_eq!(m.to_ns(165536), None);
        assert_eq!(m.to_ns(0), None, "host root is never part of a rootless map");
        assert_eq!(m.to_host(0), Some(1000));
        assert_eq!(m.to_host(1), Some(100000));
        assert_eq!(m.proc_map(), "0 1000 1\n1 100000 65536\n");
        assert_eq!(m.compact(), "0:1000:1,1:100000:65536");
    }

    #[test]
    fn bad_ranges_are_refused() {
        assert!(IdMap::rootless(1000, &[(1000, 5)]).is_err(), "overlaps the account");
        assert!(IdMap::rootless(1000, &[(100000, 10), (100005, 10)]).is_err());
        assert!(IdMap::rootless(1000, &[(100000, 0)]).is_err());
        assert!(IdMap::rootless(1000, &[(u32::MAX - 1, 5)]).is_err());
        assert!(IdMap::rootless(1000, &[(1, 1); 9]).is_err());
    }

    #[test]
    fn subuid_lines_by_name_or_id() {
        let text = "# x\nalice:100000:65536\n1001:200000:1000\nbob:x:1\nbroken\n";
        assert_eq!(sub_ranges(text, "alice", 1000).unwrap(), [(100000, 65536)]);
        assert_eq!(sub_ranges(text, "carol", 1001).unwrap(), [(200000, 1000)]);
        assert!(sub_ranges(text, "bob", 7).is_err());
        assert!(sub_ranges(text, "nobody", 9).unwrap().is_empty());
    }

    #[test]
    fn identity_contains_everything() {
        let m = IdMap::identity();
        assert_eq!(m.to_ns(0), Some(0));
        assert_eq!(m.to_ns(70000), Some(70000));
    }
}
