use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    coverage::Coverage,
    percent,
    types::{Branch, BranchCoverageMap, BranchHitMap, BranchMap, Function, FunctionMap},
    CoveragePercentage, CoverageSummary, LineHitMap, Range, SourceMap, StatementMap, Totals,
};
use std::fmt::Debug;

fn key_from_loc(range: &Range) -> String {
    format!(
        "{}|{}|{}|{}",
        range.start.line, range.start.column, range.end.line, range.end.column
    )
}

fn merge_properties_hits_vec(
    first_hits: &BranchHitMap,
    first_map: &BranchMap,
    second_hits: &BranchHitMap,
    second_map: &BranchMap,
    get_item_key_fn: for<'r> fn(&'r Branch) -> String,
) -> (BranchHitMap, IndexMap<u32, Branch>) {
    let mut items: IndexMap<String, (Vec<u32>, Branch)> = Default::default();

    for (key, item_hits) in first_hits {
        let item = first_map
            .get(key)
            .expect("Corresponding map value should exist");
        let item_key = get_item_key_fn(item);

        items.insert(item_key, (item_hits.clone(), item.clone()));
    }

    for (key, item_hits) in second_hits {
        let item = second_map
            .get(key)
            .expect("Corresponding map value should exist");
        let item_key = get_item_key_fn(item);

        items
            .entry(item_key)
            .and_modify(|pair| {
                if pair.0.len() < item_hits.len() {
                    pair.0.resize(item_hits.len(), 0);
                }

                for (h, hits) in item_hits.iter().enumerate() {
                    pair.0[h] += hits;
                }
            })
            .or_insert((item_hits.clone(), item.clone()));
    }

    let mut hits: BranchHitMap = Default::default();
    let mut map: BranchMap = Default::default();

    for (idx, (hit, item)) in items.values().enumerate() {
        hits.insert(idx as u32, hit.clone());
        map.insert(idx as u32, item.clone());
    }

    (hits, map)
}

fn merge_properties<T>(
    first_hits: &LineHitMap,
    first_map: &IndexMap<u32, T>,
    second_hits: &LineHitMap,
    second_map: &IndexMap<u32, T>,
    get_item_key_fn: for<'r> fn(&'r T) -> String,
) -> (LineHitMap, IndexMap<u32, T>)
where
    T: Clone + Debug,
{
    let mut items: IndexMap<String, (u32, T)> = Default::default();

    for (key, item_hits) in first_hits {
        let item = first_map
            .get(key)
            .expect("Corresponding map value should exist");
        let item_key = get_item_key_fn(item);

        items.insert(item_key, (*item_hits, item.clone()));
    }

    for (key, item_hits) in second_hits {
        let item = second_map
            .get(key)
            .expect("Corresponding map value should exist");
        let item_key = get_item_key_fn(item);

        items
            .entry(item_key)
            .and_modify(|pair| {
                pair.0 += *item_hits;
            })
            .or_insert((*item_hits, item.clone()));
    }

    let mut hits: LineHitMap = Default::default();
    let mut map: IndexMap<u32, T> = Default::default();

    for (idx, (hit, item)) in items.values().enumerate() {
        hits.insert(idx as u32, *hit);
        map.insert(idx as u32, item.clone());
    }

    (hits, map)
}

/// provides a read-only view of coverage for a single file.
/// It has the following properties:
/// `path` - the file path for which coverage is being tracked
/// `statementMap` - map of statement locations keyed by statement index
/// `fnMap` - map of function metadata keyed by function index
/// `branchMap` - map of branch metadata keyed by branch index
/// `s` - hit counts for statements
/// `f` - hit count for functions
/// `b` - hit count for branches
///
/// Note: internally it uses IndexMap to represent key-value pairs for the coverage data,
/// as logic for merge relies on the order of keys in the map.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileCoverage {
    #[serde(default)]
    pub all: bool,
    pub path: String,
    pub statement_map: StatementMap,
    pub fn_map: FunctionMap,
    pub branch_map: BranchMap,
    pub s: LineHitMap,
    pub f: LineHitMap,
    pub b: BranchHitMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b_t: Option<BranchHitMap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_source_map: Option<SourceMap>,
}

impl FileCoverage {
    pub fn empty(file_path: String, report_logic: bool) -> FileCoverage {
        FileCoverage {
            all: false,
            path: file_path,
            statement_map: Default::default(),
            fn_map: Default::default(),
            branch_map: Default::default(),
            s: Default::default(),
            b: Default::default(),
            f: Default::default(),
            b_t: if report_logic {
                Some(Default::default())
            } else {
                None
            },
            input_source_map: Default::default(),
        }
    }

    pub fn from_file_path(file_path: String, report_logic: bool) -> FileCoverage {
        FileCoverage::empty(file_path, report_logic)
    }

    pub fn from_file_coverage(coverage: &FileCoverage) -> FileCoverage {
        coverage.clone()
    }

    /// Returns computed line coverage from statement coverage.
    /// This is a map of hits keyed by line number in the source.
    pub fn get_line_coverage(&self) -> LineHitMap {
        let statements_map = &self.statement_map;
        let statements = &self.s;

        let mut line_map: LineHitMap = Default::default();

        for (st, count) in statements {
            let line = statements_map
                .get(st)
                .expect("statement not found")
                .start
                .line;
            let pre_val = line_map.get(&line);

            match pre_val {
                Some(pre_val) if pre_val < count => {
                    line_map.insert(line, *count);
                }
                None => {
                    line_map.insert(line, *count);
                }
                _ => {
                    //noop
                }
            }
        }

        line_map
    }

    /// Returns an array of uncovered line numbers.
    pub fn get_uncovered_lines(&self) -> Vec<u32> {
        let lc = self.get_line_coverage();
        let mut ret: Vec<u32> = Default::default();

        for (l, hits) in lc {
            if hits == 0 {
                ret.push(l);
            }
        }

        ret
    }

    pub fn get_branch_coverage_by_line(&self) -> BranchCoverageMap {
        let branch_map = &self.branch_map;
        let branches = &self.b;

        let mut prefilter_data: BranchHitMap = Default::default();
        let mut ret: BranchCoverageMap = Default::default();

        for (k, map) in branch_map {
            let line = if let Some(line) = map.line {
                line
            } else {
                map.loc.expect("Either line or loc should exist").start.line
            };

            let branch_data = branches.get(k).expect("branch data not found");

            if let Some(line_data) = prefilter_data.get_mut(&line) {
                line_data.append(&mut branch_data.clone());
            } else {
                prefilter_data.insert(line, branch_data.clone());
            }
        }

        for (k, data_array) in prefilter_data {
            let covered: Vec<&u32> = data_array.iter().filter(|&x| *x > 0).collect();
            let coverage = covered.len() as f32 / data_array.len() as f32 * 100 as f32;

            ret.insert(
                k,
                Coverage::new(covered.len() as u32, data_array.len() as u32, coverage),
            );
        }

        ret
    }

    pub fn to_json() {
        unimplemented!()
    }
    /// Merges a second coverage object into this one, updating hit counts
    pub fn merge(&mut self, coverage: &FileCoverage) {
        if coverage.all {
            return;
        }

        if self.all {
            *self = coverage.clone();
            return;
        }

        let (statement_hits_merged, statement_map_merged) = merge_properties(
            &self.s,
            &self.statement_map,
            &coverage.s,
            &coverage.statement_map,
            |range: &Range| key_from_loc(range),
        );

        self.s = statement_hits_merged;
        self.statement_map = statement_map_merged;

        let (fn_hits_merged, fn_map_merged) = merge_properties(
            &self.f,
            &self.fn_map,
            &coverage.f,
            &coverage.fn_map,
            |map: &Function| key_from_loc(&map.loc),
        );

        self.f = fn_hits_merged;
        self.fn_map = fn_map_merged;

        let (branches_hits_merged, branches_map_merged) = merge_properties_hits_vec(
            &self.b,
            &self.branch_map,
            &coverage.b,
            &coverage.branch_map,
            |branch: &Branch| key_from_loc(&branch.locations[0]),
        );
        self.b = branches_hits_merged;
        self.branch_map = branches_map_merged;

        // Tracking additional information about branch truthiness
        // can be optionally enabled:
        if let Some(branches_true) = &self.b_t {
            if let Some(coverage_branches_true) = &coverage.b_t {
                let (branches_true_hits_merged, _) = merge_properties_hits_vec(
                    branches_true,
                    &self.branch_map,
                    coverage_branches_true,
                    &coverage.branch_map,
                    |branch: &Branch| key_from_loc(&branch.locations[0]),
                );

                self.b_t = Some(branches_true_hits_merged);
            }
        }
    }

    pub fn compute_simple_totals<T>(line_map: &IndexMap<T, u32>) -> Totals {
        let total = line_map.len() as u32;
        let covered = line_map.values().filter(|&x| *x > 0).count() as u32;
        Totals {
            total,
            covered,
            skipped: 0,
            pct: CoveragePercentage::Value(percent(covered, total)),
        }
    }

    fn compute_branch_totals(branch_map: &BranchHitMap) -> Totals {
        let mut ret: Totals = Default::default();

        branch_map.values().for_each(|branches| {
            ret.covered += branches.iter().filter(|hits| **hits > 0).count() as u32;
            ret.total += branches.len() as u32;
        });

        ret.pct = CoveragePercentage::Value(percent(ret.covered, ret.total));
        ret
    }

    pub fn reset_hits(&mut self) {
        for val in self.s.values_mut() {
            *val = 0;
        }

        for val in self.f.values_mut() {
            *val = 0;
        }

        for val in self.b.values_mut() {
            val.iter_mut().for_each(|x| *x = 0);
        }

        if let Some(branches_true) = &mut self.b_t {
            for val in branches_true.values_mut() {
                val.iter_mut().for_each(|x| *x = 0);
            }
        }
    }

    pub fn to_summary(&self) -> CoverageSummary {
        let line_coverage = self.get_line_coverage();

        let line = FileCoverage::compute_simple_totals(&line_coverage);
        let function = FileCoverage::compute_simple_totals(&self.f);
        let statement = FileCoverage::compute_simple_totals(&self.s);
        let branches = FileCoverage::compute_branch_totals(&self.b);

        let branches_true = if let Some(branches_true) = &self.b_t {
            Some(FileCoverage::compute_branch_totals(&branches_true))
        } else {
            None
        };

        CoverageSummary::new(line, statement, function, branches, branches_true)
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use crate::{
        coverage::Coverage,
        coverage_summary::{CoveragePercentage, Totals},
        types::{Branch, Function},
        BranchType, FileCoverage, Range,
    };

    #[test]
    fn should_able_to_merge_another_file() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (0, Range::new(1, 1, 1, 100)),
                (1, Range::new(2, 1, 2, 50)),
                (2, Range::new(2, 51, 2, 100)),
                (3, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                0,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                0,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(0, 0), (1, 0), (2, 0), (3, 0)]),
            f: IndexMap::from([(0, 0)]),
            b: IndexMap::from([(0, vec![0, 0])]),
            b_t: None,
            input_source_map: None,
        };

        let mut first = base.clone();
        let mut second = base.clone();

        first.s.insert(0, 1);
        first.f.insert(0, 1);
        first.b.entry(0).and_modify(|v| v[0] = 1);

        second.s.insert(1, 1);
        second.f.insert(0, 1);
        second.b.entry(0).and_modify(|v| v[1] = 2);

        let summary = first.to_summary();
        assert_eq!(
            summary.statements,
            Totals::new(4, 1, 0, CoveragePercentage::Value(25.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );
        assert_eq!(
            summary.branches,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );

        first.merge(&second);
        let summary = first.to_summary();

        assert_eq!(
            summary.statements,
            Totals::new(4, 2, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(
            summary.branches,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(first.s.get(&0), Some(&1));
        assert_eq!(first.s.get(&1), Some(&1));
        assert_eq!(first.f.get(&0), Some(&2));
        assert_eq!(first.b.get(&0).unwrap()[0], 1);
        assert_eq!(first.b.get(&0).unwrap()[1], 2);
    }

    #[test]
    fn should_able_to_merge_another_file_with_different_starting_indices() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (0, Range::new(1, 1, 1, 100)),
                (1, Range::new(2, 1, 2, 50)),
                (2, Range::new(2, 51, 2, 100)),
                (3, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                0,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                0,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(0, 0), (1, 0), (2, 0), (3, 0)]),
            f: IndexMap::from([(0, 0)]),
            b: IndexMap::from([(0, vec![0, 0])]),
            b_t: None,
            input_source_map: None,
        };

        let base_other = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (1, Range::new(1, 1, 1, 100)),
                (2, Range::new(2, 1, 2, 50)),
                (3, Range::new(2, 51, 2, 100)),
                (4, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                1,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                1,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(1, 0), (2, 0), (3, 0), (4, 0)]),
            f: IndexMap::from([(1, 0)]),
            b: IndexMap::from([(1, vec![0, 0])]),
            b_t: None,
            input_source_map: None,
        };

        let mut first = base.clone();
        let mut second = base_other.clone();

        first.s.insert(0, 1);
        first.f.insert(0, 1);
        first.b.entry(0).and_modify(|v| v[0] = 1);

        second.s.insert(2, 1);
        second.f.insert(1, 1);
        second.b.entry(1).and_modify(|v| v[1] = 2);

        let summary = first.to_summary();
        assert_eq!(
            summary.statements,
            Totals::new(4, 1, 0, CoveragePercentage::Value(25.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );
        assert_eq!(
            summary.branches,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );

        first.merge(&second);
        let summary = first.to_summary();

        assert_eq!(
            summary.statements,
            Totals::new(4, 2, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(
            summary.branches,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(first.s.get(&0), Some(&1));
        assert_eq!(first.s.get(&1), Some(&1));
        assert_eq!(first.f.get(&0), Some(&2));
        assert_eq!(first.b.get(&0).unwrap()[0], 1);
        assert_eq!(first.b.get(&0).unwrap()[1], 2);
    }

    #[test]
    fn should_drop_data_while_merge() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (1, Range::new(1, 1, 1, 100)),
                (2, Range::new(2, 1, 2, 50)),
                (3, Range::new(2, 51, 2, 100)),
                (4, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                1,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                1,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(1, 0), (2, 0), (3, 0), (4, 0)]),
            f: IndexMap::from([(1, 0)]),
            b: IndexMap::from([(1, vec![0, 0])]),
            b_t: None,
            input_source_map: None,
        };

        let create_coverage = |all: bool| {
            let mut ret = base.clone();
            if all {
                ret.all = true;
            } else {
                ret.s.insert(1, 1);
                ret.f.insert(1, 1);
                ret.b.entry(1).and_modify(|v| v[0] = 1);
            }

            ret
        };

        let expected = create_coverage(false);

        let mut cov = create_coverage(true);
        cov.merge(&create_coverage(false));
        assert_eq!(cov, expected);

        let mut cov = create_coverage(false);
        cov.merge(&create_coverage(true));
        assert_eq!(cov, expected);
    }

    #[test]
    fn merges_another_file_coverage_tracks_logical_truthiness() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (0, Range::new(1, 1, 1, 100)),
                (1, Range::new(2, 1, 2, 50)),
                (2, Range::new(2, 51, 2, 100)),
                (3, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                0,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                0,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(0, 0), (1, 0), (2, 0), (3, 0)]),
            f: IndexMap::from([(0, 0)]),
            b: IndexMap::from([(0, vec![0, 0])]),
            b_t: None,
            input_source_map: None,
        };

        let mut first = base.clone();
        let mut second = base.clone();

        first.s.insert(0, 1);
        first.f.insert(0, 1);
        first.b.entry(0).and_modify(|v| v[0] = 1);
        first.b_t = Some(IndexMap::from([(0, vec![1])]));

        second.s.insert(1, 1);
        second.f.insert(0, 1);
        second.b.entry(0).and_modify(|v| v[1] = 2);
        second.b_t = Some(IndexMap::from([(0, vec![0, 2])]));

        let summary = first.to_summary();

        assert_eq!(
            summary.statements,
            Totals::new(4, 1, 0, CoveragePercentage::Value(25.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(
            summary.branches,
            Totals::new(2, 1, 0, CoveragePercentage::Value(50.0))
        );

        first.merge(&second);
        let summary = first.to_summary();

        assert_eq!(
            summary.statements,
            Totals::new(4, 2, 0, CoveragePercentage::Value(50.0))
        );
        assert_eq!(
            summary.lines,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );
        assert_eq!(
            summary.functions,
            Totals::new(1, 1, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(
            summary.branches,
            Totals::new(2, 2, 0, CoveragePercentage::Value(100.0))
        );

        assert_eq!(first.s.get(&0), Some(&1));
        assert_eq!(first.s.get(&1), Some(&1));
        assert_eq!(first.f.get(&0), Some(&2));
        assert_eq!(first.b.get(&0).unwrap()[0], 1);
        assert_eq!(first.b.get(&0).unwrap()[1], 2);
        let b_t = first.b_t.unwrap();
        assert_eq!(b_t.get(&0).unwrap()[0], 1);
        assert_eq!(b_t.get(&0).unwrap()[1], 2);
    }

    #[test]
    fn should_reset_hits() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (1, Range::new(1, 1, 1, 100)),
                (2, Range::new(2, 1, 2, 50)),
                (3, Range::new(2, 51, 2, 100)),
                (4, Range::new(2, 101, 3, 100)),
            ]),
            fn_map: IndexMap::from([(
                1,
                Function {
                    name: "foobar".to_string(),
                    line: 1,
                    loc: Range::new(1, 1, 1, 50),
                    decl: Default::default(),
                },
            )]),
            branch_map: IndexMap::from([(
                1,
                Branch::from_line(
                    BranchType::If,
                    2,
                    vec![Range::new(2, 1, 2, 20), Range::new(2, 50, 2, 100)],
                ),
            )]),
            s: IndexMap::from([(1, 2), (2, 3), (3, 1), (4, 0)]),
            f: IndexMap::from([(1, 54)]),
            b: IndexMap::from([(1, vec![1, 50])]),
            b_t: Some(IndexMap::from([(1, vec![1, 50])])),
            input_source_map: None,
        };

        let mut value = base.clone();
        value.reset_hits();

        assert_eq!(IndexMap::from([(1, 0), (2, 0), (3, 0), (4, 0)]), value.s);
        assert_eq!(IndexMap::from([(1, 0)]), value.f);
        assert_eq!(IndexMap::from([(1, vec![0, 0])]), value.b);
        assert_eq!(Some(IndexMap::from([(1, vec![0, 0])])), value.b_t);
    }

    #[test]
    fn should_return_uncovered_lines() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: IndexMap::from([
                (1, Range::new(1, 1, 1, 100)),
                (2, Range::new(1, 101, 1, 200)),
                (3, Range::new(2, 1, 2, 100)),
            ]),
            fn_map: Default::default(),
            branch_map: Default::default(),
            s: IndexMap::from([(1, 0), (2, 1), (3, 0)]),
            f: Default::default(),
            b: Default::default(),
            b_t: None,
            input_source_map: None,
        };

        assert_eq!(base.get_uncovered_lines(), vec![2]);
    }

    #[test]
    fn should_return_branch_coverage_by_line() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: Default::default(),
            fn_map: Default::default(),
            branch_map: IndexMap::from([
                (1, Branch::from_line(BranchType::If, 1, Default::default())),
                (2, Branch::from_line(BranchType::If, 2, Default::default())),
            ]),
            s: Default::default(),
            f: Default::default(),
            b: IndexMap::from([(1, vec![1, 0]), (2, vec![0, 0, 0, 1])]),
            b_t: None,
            input_source_map: None,
        };

        let coverage = base.get_branch_coverage_by_line();
        assert_eq!(
            coverage,
            IndexMap::from([
                (1, Coverage::new(1, 2, 50.0)),
                (2, Coverage::new(1, 4, 25.0)),
            ])
        );
    }

    #[test]
    fn should_return_branch_coverage_by_line_with_cobertura_branchmap_structure() {
        let base = FileCoverage {
            all: false,
            path: "/path/to/file".to_string(),
            statement_map: Default::default(),
            fn_map: Default::default(),
            branch_map: IndexMap::from([
                (
                    1,
                    Branch::from_loc(BranchType::If, Range::new(1, 1, 1, 100), Default::default()),
                ),
                (
                    2,
                    Branch::from_loc(
                        BranchType::If,
                        Range::new(2, 50, 2, 100),
                        Default::default(),
                    ),
                ),
            ]),
            s: Default::default(),
            f: Default::default(),
            b: IndexMap::from([(1, vec![1, 0]), (2, vec![0, 0, 0, 1])]),
            b_t: None,
            input_source_map: None,
        };

        let coverage = base.get_branch_coverage_by_line();
        assert_eq!(
            coverage,
            IndexMap::from([
                (1, Coverage::new(1, 2, 50.0)),
                (2, Coverage::new(1, 4, 25.0)),
            ])
        )
    }

    #[test]
    fn should_allow_file_coverage_to_be_init_with_logical_truthiness() {
        assert_eq!(
            FileCoverage::from_file_path("".to_string(), false).b_t,
            None
        );
        assert_eq!(
            FileCoverage::from_file_path("".to_string(), true).b_t,
            Some(Default::default())
        );
    }
}
