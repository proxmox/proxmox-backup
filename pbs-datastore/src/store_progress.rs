#[derive(Debug, Default)]
/// Tracker for progress of operations iterating over `Datastore` contents.
pub struct StoreProgress {
    /// Completed groups
    pub done_groups: u64,
    /// Total groups
    pub total_groups: u64,
    /// Completed snapshots within current group
    pub done_snapshots: u64,
    /// Total snapshots in current group
    pub group_snapshots: u64,
}

impl StoreProgress {
    pub fn new(total_groups: u64) -> Self {
        StoreProgress {
            total_groups,
            ..Default::default()
        }
    }

    /// Calculates an interpolated relative progress based on current counters.
    pub fn percentage(&self) -> f64 {
        let per_groups = (self.done_groups as f64) / (self.total_groups as f64);
        if self.group_snapshots == 0 {
            per_groups
        } else {
            let per_snapshots = (self.done_snapshots as f64) / (self.group_snapshots as f64);
            per_groups + (1.0 / self.total_groups as f64) * per_snapshots
        }
    }
}

impl std::fmt::Display for StoreProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let current_group = if self.done_groups < self.total_groups {
            self.done_groups + 1
        } else {
            self.done_groups
        };

        if self.group_snapshots == 0 {
            write!(
                f,
                "{:.2}% ({}/{} groups)",
                self.percentage() * 100.0,
                self.done_groups,
                self.total_groups,
            )
        } else if self.total_groups == 1 {
            write!(
                f,
                "{:.2}% ({}/{} snapshots)",
                self.percentage() * 100.0,
                self.done_snapshots,
                self.group_snapshots,
            )
        } else if self.done_snapshots == self.group_snapshots {
            write!(
                f,
                "{:.2}% ({}/{} groups)",
                self.percentage() * 100.0,
                current_group,
                self.total_groups,
            )
        } else {
            write!(
                f,
                "{:.2}% ({}/{} groups, {}/{} snapshots in group #{})",
                self.percentage() * 100.0,
                self.done_groups,
                self.total_groups,
                self.done_snapshots,
                self.group_snapshots,
                current_group,
            )
        }
    }
}
