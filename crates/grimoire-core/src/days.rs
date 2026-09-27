//! A quiet record of each day's writing, for the Progress page: the book's
//! words at the end of the day and how many it grew by. One line a day in
//! `.grimoire/days.txt`, beside the day's baseline. There are no streaks
//! here and nothing to keep up with: a day without writing is just a day.

use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    /// The book's words when the day was last written down.
    pub total: usize,
    /// What the day added (below zero after a day of cutting).
    pub written: i64,
}

fn path(root: &Path) -> PathBuf {
    root.join(".grimoire").join("days.txt")
}

/// Every day written down, oldest first. A line that can't be read is
/// skipped rather than losing the rest.
pub fn read(root: &Path) -> Vec<Day> {
    let Ok(text) = std::fs::read_to_string(path(root)) else {
        return Vec::new();
    };
    let mut days: Vec<Day> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some(Day {
                date: NaiveDate::parse_from_str(f.next()?, "%Y-%m-%d").ok()?,
                total: f.next()?.parse().ok()?,
                written: f.next()?.parse().ok()?,
            })
        })
        .collect();
    days.sort_by_key(|d| d.date);
    days.dedup_by_key(|d| d.date);
    days
}

/// Write `day` down, replacing what was there for its date.
pub fn record(root: &Path, day: Day) -> Result<()> {
    let mut days = read(root);
    days.retain(|d| d.date != day.date);
    days.push(day);
    days.sort_by_key(|d| d.date);
    let text: String = days
        .iter()
        .map(|d| format!("{} {} {}\n", d.date.format("%Y-%m-%d"), d.total, d.written))
        .collect();
    let file = path(root);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic::write_text(&file, &text)
}

/// The Monday of `date`'s week.
pub fn week_of(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

/// Words written in each of the `n` weeks up to and including `today`'s,
/// oldest first: (the week's Monday, words). Weeks before the first day
/// written down are left out, since nothing was counting then.
pub fn weeks(days: &[Day], today: NaiveDate, n: usize) -> Vec<(NaiveDate, i64)> {
    let this = week_of(today);
    let Some(first) = days.first().map(|d| week_of(d.date)) else {
        return vec![(this, 0)];
    };
    (0..n as i64)
        .rev()
        .map(|back| this - Duration::weeks(back))
        .filter(|&w| w >= first)
        .map(|w| {
            let words = days
                .iter()
                .filter(|d| week_of(d.date) == w)
                .map(|d| d.written)
                .sum();
            (w, words)
        })
        .collect()
}

/// Words a week lately: the average of the last four weeks before this one
/// (fewer if counting began more recently), this week included only when
/// there's nothing else to go on. `None` until there's something to measure.
pub fn pace(days: &[Day], today: NaiveDate) -> Option<f64> {
    let all = weeks(days, today, 5);
    let done: Vec<i64> = all.iter().rev().skip(1).map(|&(_, w)| w).collect();
    let (sum, count) = if done.is_empty() {
        (all.last().map_or(0, |&(_, w)| w), 1)
    } else {
        (done.iter().sum(), done.len())
    };
    (sum > 0).then(|| sum as f64 / count as f64)
}

/// When `goal` comes at `per_week` from `words` today: `None` if it's
/// already reached or there's no pace yet.
pub fn arrives(
    words: usize,
    goal: usize,
    per_week: Option<f64>,
    today: NaiveDate,
) -> Option<NaiveDate> {
    let left = goal.checked_sub(words).filter(|&l| l > 0)?;
    let per_day = per_week? / 7.0;
    let days = (left as f64 / per_day).ceil() as i64;
    // Past a few years, a date says nothing useful.
    (days <= 3 * 365).then(|| today + Duration::days(days))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn day(s: &str, total: usize, written: i64) -> Day {
        Day {
            date: d(s),
            total,
            written,
        }
    }

    #[test]
    fn a_day_is_replaced_not_repeated_and_read_back_in_order() {
        let root = std::env::temp_dir().join(format!("grimoire-days-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        record(&root, day("2026-09-27", 1200, 300)).unwrap();
        record(&root, day("2026-09-25", 900, 900)).unwrap();
        record(&root, day("2026-09-27", 1500, 600)).unwrap();
        assert_eq!(
            read(&root),
            vec![day("2026-09-25", 900, 900), day("2026-09-27", 1500, 600)]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn weeks_start_on_monday_and_skip_the_time_before_counting() {
        // 2026-09-28 is a Monday.
        let days = [
            day("2026-09-16", 500, 500),
            day("2026-09-20", 900, 400),
            day("2026-09-22", 1000, 100),
            day("2026-09-29", 1600, 600),
        ];
        assert_eq!(
            weeks(&days, d("2026-10-01"), 8),
            vec![
                (d("2026-09-14"), 900),
                (d("2026-09-21"), 100),
                (d("2026-09-28"), 600)
            ]
        );
        // Two finished weeks: (900 + 100) / 2.
        assert_eq!(pace(&days, d("2026-10-01")), Some(500.0));
        assert_eq!(
            arrives(1600, 2600, Some(700.0), d("2026-10-01")),
            Some(d("2026-10-11"))
        );
        assert_eq!(
            arrives(3000, 2600, Some(700.0), d("2026-10-01")),
            None,
            "already there"
        );
        assert_eq!(pace(&[], d("2026-10-01")), None);
    }
}
