use std::collections::BTreeSet;

use super::Blocker;
use crate::{Catalog, Checksum, Coverage, CoverageSource, HistoryEntry, Version};

impl<M> Catalog<'_, M> {
    /// The installed and covered versions the catalog contains, adding a
    /// blocker for every installed or covered version that is missing,
    /// changed or recorded twice, for an executed snapshot that is missing
    /// or changed, for every pending incremental at or below the coverage
    /// boundary, and for every other pending one below the newest installed
    /// or covered version.
    pub(crate) fn compare(
        &self,
        history: &[HistoryEntry],
        coverage: Option<&Coverage>,
        blockers: &mut Vec<Blocker>,
    ) -> BTreeSet<Version> {
        let mut installed = BTreeSet::new();
        let boundary = self.boundary(coverage);
        let mut newest = boundary;

        if let Some(coverage) = coverage {
            for member in &coverage.members {
                self.record(&member.version, member.checksum, &mut installed, blockers);
            }

            if let CoverageSource::Snapshot { checksum, .. } = coverage.source {
                self.check_snapshot(&coverage.boundary, boundary, checksum, blockers);
            }
        }

        for record in history {
            newest =
                newest.max(self.record(&record.version, record.checksum, &mut installed, blockers));
        }

        for entry in self.incrementals() {
            let below = |bound: Option<Version>| bound.is_some_and(|bound| entry.version < bound);

            if installed.contains(&entry.version) {
                continue;
            }

            if let Some(boundary) = boundary.filter(|boundary| entry.version <= *boundary) {
                blockers.push(Blocker::Uncovered {
                    version: entry.version,
                    boundary,
                });
            } else if below(newest) {
                blockers.push(Blocker::Late {
                    version: entry.version,
                    checksum: entry.checksum,
                });
            }
        }

        installed
    }

    /// The coverage boundary, parsed under the catalog's scheme.
    pub(crate) fn boundary(&self, coverage: Option<&Coverage>) -> Option<Version> {
        coverage.and_then(|coverage| self.scheme.parse(&coverage.boundary).ok())
    }

    /// Checks one installed or covered record against the catalog's
    /// incrementals, adding it to `installed` and returning its parsed
    /// version, which is `None` only if it does not parse.
    fn record(
        &self,
        version: &str,
        checksum: Checksum,
        installed: &mut BTreeSet<Version>,
        blockers: &mut Vec<Blocker>,
    ) -> Option<Version> {
        let parsed = self.scheme.parse(version).ok();
        let entry = parsed.and_then(|parsed| self.incrementals().find(|e| e.version == parsed));
        let Some(entry) = entry else {
            blockers.push(Blocker::Missing {
                version: version.to_owned(),
                checksum,
            });

            return parsed;
        };

        if !installed.insert(entry.version) {
            blockers.push(Blocker::Duplicate(entry.version));
        }

        if entry.checksum != checksum {
            blockers.push(Blocker::Changed {
                version: entry.version,
                installed: checksum,
                catalog: entry.checksum,
            });
        }

        parsed
    }

    /// Checks an executed snapshot, stored at `stored` with `checksum`,
    /// against the catalog's snapshot of that version.
    fn check_snapshot(
        &self,
        stored: &str,
        version: Option<Version>,
        checksum: Checksum,
        blockers: &mut Vec<Blocker>,
    ) {
        let entry = self
            .snapshots()
            .find(|entry| Some(entry.version) == version);

        match entry {
            None => blockers.push(Blocker::Missing {
                version: stored.to_owned(),
                checksum,
            }),
            Some(entry) if entry.checksum != checksum => blockers.push(Blocker::Changed {
                version: entry.version,
                installed: checksum,
                catalog: entry.checksum,
            }),
            Some(_) => {}
        }
    }
}
