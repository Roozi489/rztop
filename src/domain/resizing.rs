//! Persistent proportions and integer allocation shared by layout and process rankings.
// Modified for rztop on 2026-11-02.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResizeConfig {
    pub layout: String,
    pub splits: BTreeMap<String, Vec<u64>>,
    pub process_weights: [u64; 4],
    pub io_weight: u64,
}

impl Default for ResizeConfig {
    fn default() -> Self {
        Self {
            layout: String::new(),
            splits: BTreeMap::new(),
            process_weights: [1; 4],
            io_weight: 0,
        }
    }
}

impl ResizeConfig {
    pub fn ranking_weights(&self) -> [u64; 5] {
        let [cpu, mem, gpu, vram] = self.process_weights;
        let io = if self.io_weight == 0 {
            (self
                .process_weights
                .iter()
                .map(|&w| u128::from(w))
                .sum::<u128>()
                / 4)
            .max(1) as u64
        } else {
            self.io_weight
        };
        [cpu, mem, io, gpu, vram]
    }
    pub fn set_ranking_weights(&mut self, weights: &[u64]) {
        if let [cpu, mem, io, gpu, vram] = *weights {
            self.process_weights = [cpu, mem, gpu, vram];
            self.io_weight = io;
        }
    }
}

/// Allocate exact totals with floors. Zero minima mark hidden children.
/// When weights already represent legal pixel sizes their proportions are preserved.
pub fn allocate(total: usize, weights: &[u64], minimums: &[usize]) -> Vec<usize> {
    let mut sizes = vec![0; minimums.len()];
    let mut active: Vec<usize> = minimums
        .iter()
        .enumerate()
        .filter_map(|(i, &m)| (m > 0).then_some(i))
        .collect();
    let mut remaining = total;
    while !active.is_empty() {
        let sum: u128 = active
            .iter()
            .map(|&i| u128::from(weights.get(i).copied().unwrap_or(1).max(1)))
            .sum();
        let constrained = active.iter().copied().find(|&i| {
            (remaining as u128 * u128::from(weights.get(i).copied().unwrap_or(1).max(1)) / sum)
                < minimums[i] as u128
        });
        if let Some(i) = constrained {
            sizes[i] = minimums[i].min(remaining);
            remaining -= sizes[i];
            active.retain(|&j| j != i);
            continue;
        }
        let mut used = 0;
        for (index, &i) in active.iter().enumerate() {
            sizes[i] = if index + 1 == active.len() {
                remaining - used
            } else {
                (remaining as u128 * u128::from(weights.get(i).copied().unwrap_or(1).max(1)) / sum)
                    as usize
            };
            used += sizes[i];
        }
        break;
    }
    sizes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitTarget {
    Stack(String),
    Rankings,
}

#[derive(Debug, Clone)]
pub struct Divider {
    pub target: SplitTarget,
    pub axis: Axis,
    /// First of the two border strokes, in 1-based terminal cells.
    pub position: usize,
    pub start: usize,
    pub end: usize,
    pub before: usize,
    pub after: usize,
    pub sizes: Vec<usize>,
    pub minimums: Vec<usize>,
}

impl Divider {
    pub fn hit(&self, x: usize, y: usize) -> bool {
        let (across, along) = match self.axis {
            Axis::Horizontal => (y, x),
            Axis::Vertical => (x, y),
        };
        (across == self.position || across == self.position + 1)
            && along >= self.start
            && along <= self.end
    }
    pub fn dragged(&self, delta: isize) -> Vec<u64> {
        let mut weights: Vec<u64> = self.sizes.iter().map(|&v| v as u64).collect();
        let sum = self.sizes[self.before] + self.sizes[self.after];
        let low = self.minimums[self.before];
        let high = sum.saturating_sub(self.minimums[self.after]);
        if low <= high {
            let new_size = self.sizes[self.before]
                .saturating_add_signed(delta)
                .clamp(low, high);
            weights[self.before] = new_size as u64;
            weights[self.after] = (sum - new_size) as u64;
        }
        weights
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_totals_floors_hidden_and_exact_restore() {
        assert_eq!(allocate(100, &[30, 70], &[5, 5]), [30, 70]);
        assert_eq!(allocate(100, &[1, 99], &[10, 10]), [10, 90]);
        assert_eq!(allocate(100, &[1, u64::MAX, 1], &[5, 0, 5]), [50, 0, 50]);
        for total in 10..200 {
            assert_eq!(
                allocate(total, &[3, 7], &[5, 5]).iter().sum::<usize>(),
                total
            );
        }
    }
    #[test]
    fn old_weights_keep_original_panels_and_insert_average() {
        let config = ResizeConfig {
            process_weights: [2, 4, 6, 8],
            ..Default::default()
        };
        assert_eq!(config.ranking_weights(), [2, 4, 5, 6, 8]);
    }
}
